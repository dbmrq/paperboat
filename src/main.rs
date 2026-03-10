mod acp;
mod agents;
mod app;
mod backend;
mod config;
mod error;
mod logging;
mod mcp_server;
mod metrics;
mod models;
mod self_improve;
mod tasks;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(feature = "tui")]
mod tui;
mod types;

use anyhow::Result;
use backend::{discover_available_backends, prompt_backend_selection, BackendConfig};
use config::{build_model_config, get_explicit_backend_config, load_agent_configs};
use logging::RunLogManager;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};

/// Parsed command-line arguments.
#[allow(clippy::struct_excessive_bools)]
struct CliArgs {
    /// Show help and exit
    help_mode: bool,
    /// Disable TUI mode and run headlessly
    headless_mode: bool,
    /// Run in MCP server mode
    mcp_server_mode: bool,
    /// Validate config files only (don't run the app)
    validate_config_mode: bool,
    /// Enable JSON formatted logs (for machine parsing)
    json_logs: bool,
    /// Enable metrics collection (can also be set via `PAPERBOAT_METRICS` env var)
    metrics_enabled: bool,
    /// Backend configuration (backend and optional transport).
    ///
    /// Parsed from `--backend` flag in format "backend:transport".
    /// Examples: "cursor", "cursor:cli", "cursor:acp", "auggie:acp"
    backend_config: Option<BackendConfig>,
    /// The goal/task to accomplish
    goal: Option<String>,
}

/// Parse command-line arguments and extract flags and the goal.
///
/// # Goal Resolution
///
/// The goal argument is resolved in this priority order:
/// 1. If argument is a file path that exists → read file contents as goal
/// 2. Otherwise → use argument as direct prompt string
/// 3. No argument → `None` (handled later with interactive prompt)
///
/// # Backend Selection
///
/// The `--backend` flag supports an optional transport suffix:
/// - `--backend cursor` - Cursor with default transport (CLI)
/// - `--backend cursor:cli` - Cursor with CLI transport (explicit)
/// - `--backend cursor:acp` - Cursor with ACP transport
/// - `--backend auggie` - Auggie with default transport (ACP)
/// - `--backend auggie:acp` - Auggie with ACP transport (explicit)
fn parse_args(args: &[String]) -> CliArgs {
    // --help or -h shows help and exits
    let help_mode = args.contains(&"--help".to_string()) || args.contains(&"-h".to_string());
    // --headless disables TUI mode (TUI is enabled by default)
    let headless_mode = args.contains(&"--headless".to_string());
    let mcp_server_mode = args.get(1).is_some_and(|a| a == "--mcp-server");
    let validate_config_mode = args.contains(&"--validate-config".to_string());
    // --json-logs enables JSON formatted logs (can also be set via PAPERBOAT_JSON_LOGS env var)
    let json_logs = args.contains(&"--json-logs".to_string())
        || std::env::var("PAPERBOAT_JSON_LOGS").is_ok_and(|v| v == "1" || v == "true");
    // --metrics enables metrics collection (can also be set via PAPERBOAT_METRICS env var)
    let metrics_enabled = args.contains(&"--metrics".to_string())
        || std::env::var("PAPERBOAT_METRICS").is_ok_and(|v| v == "1" || v == "true");

    // --backend <name[:transport]> selects the backend and optional transport
    // Examples: cursor, cursor:cli, cursor:acp, auggie, auggie:acp
    let backend_config = args
        .iter()
        .position(|a| a == "--backend")
        .and_then(|i| args.get(i + 1))
        .map(|backend_str| {
            match BackendConfig::parse(backend_str) {
                Ok(config) => config,
                Err(e) => {
                    // Print error and exit immediately for invalid backend config
                    eprintln!(
                        "❌ Invalid --backend value '{backend_str}': {e}\n\n\
                        Valid backends:\n  \
                          auggie        Augment's Auggie CLI (default transport: acp)\n  \
                          cursor        Cursor's agent CLI (default transport: cli)\n\n\
                        Transport options (optional suffix):\n  \
                          cursor:cli    CLI transport (better MCP support, default for Cursor)\n  \
                          cursor:acp    ACP transport (JSON-RPC protocol)\n  \
                          auggie:acp    ACP transport (only option for Auggie)\n\n\
                        Examples:\n  \
                          --backend cursor\n  \
                          --backend cursor:cli\n  \
                          --backend cursor:acp\n  \
                          --backend auggie"
                    );
                    std::process::exit(1);
                }
            }
        });

    // Goal is the first non-flag argument after the program name
    // Skip arguments that are values for flags (like --backend value)
    let flags_with_values = ["--backend", "--socket"];
    let goal_arg = args
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(i, arg)| {
            // Skip flag arguments
            if arg.starts_with("--") || arg.starts_with('-') {
                return false;
            }
            // Skip values of flags that take arguments
            if *i > 0 {
                if let Some(prev) = args.get(i - 1) {
                    if flags_with_values.contains(&prev.as_str()) {
                        return false;
                    }
                }
            }
            true
        })
        .map(|(_, arg)| arg.clone())
        .next();

    // Resolve goal: check if argument is a file path or direct prompt
    let goal = goal_arg.and_then(|arg| resolve_goal_argument(&arg));

    CliArgs {
        help_mode,
        headless_mode,
        mcp_server_mode,
        validate_config_mode,
        json_logs,
        metrics_enabled,
        backend_config,
        goal,
    }
}

/// Resolves a goal argument, checking if it's a file path or a direct prompt.
///
/// Returns:
/// - `Some(content)` if argument is a file that exists (reads file content)
/// - `Some(arg)` if argument is not a file (uses as direct prompt)
/// - Exits with error if file exists but cannot be read
fn resolve_goal_argument(arg: &str) -> Option<String> {
    let path = PathBuf::from(arg);

    // Check if the argument looks like a file path and exists
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    eprintln!("❌ Goal file is empty: {}", path.display());
                    std::process::exit(1);
                }
                eprintln!("📄 Reading goal from file: {}", path.display());
                Some(trimmed.to_string())
            }
            Err(e) => {
                eprintln!("❌ Failed to read goal file '{}': {}", path.display(), e);
                std::process::exit(1);
            }
        }
    } else {
        // Not a file, use as direct prompt
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

/// Prompts the user for a goal interactively via stdin.
///
/// This function handles:
/// - Checking if stdin is a terminal (returns None if not)
/// - Timeout handling (60 seconds)
/// - Ctrl+C/Ctrl+D handling (exits gracefully)
/// - Empty input handling
///
/// Returns `Some(goal)` if user enters valid input, `None` if stdin is not a terminal.
fn prompt_goal_interactively() -> Option<String> {
    // Don't prompt if stdin is not a terminal
    if !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return None;
    }

    println!("\n🎯 No goal provided. Please enter your task:");
    println!("(Enter your task/goal. Press Enter twice or Ctrl+D when done)\n");
    print!("> ");
    io::stdout().flush().ok()?;

    // Use a channel with timeout for reading input
    let (tx, rx) = mpsc::channel();
    let stdin_thread = std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut lines = Vec::new();

        // Read multiple lines until empty line or EOF
        loop {
            let mut line = String::new();
            let read_result = stdin.lock().read_line(&mut line);
            match read_result {
                Ok(0) => {
                    // EOF (Ctrl+D on Unix)
                    break;
                }
                Ok(_) => {
                    // Check for empty line (just Enter pressed) to end input
                    if line.trim().is_empty() && !lines.is_empty() {
                        break;
                    }
                    lines.push(line);
                }
                Err(_) => {
                    // Read error (possibly Ctrl+C)
                    let _ = tx.send(None);
                    return;
                }
            }
        }

        let combined = lines.concat();
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            let _ = tx.send(None);
        } else {
            let _ = tx.send(Some(trimmed.to_string()));
        }
    });

    // Wait for input with a 60-second timeout
    let result = match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(Some(goal)) => {
            println!("\n✓ Goal received\n");
            Some(goal)
        }
        Ok(None) => {
            // Empty input or error
            println!();
            eprintln!("\n❌ No goal provided. Exiting.");
            std::process::exit(1);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            println!();
            eprintln!("\n⏱ Input timeout. Exiting.");
            std::process::exit(1);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // Thread panicked or disconnected
            println!();
            eprintln!("\n❌ Input error. Exiting.");
            std::process::exit(1);
        }
    };

    // Wait for the stdin thread to finish
    let _ = stdin_thread.join();

    result
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let args: Vec<String> = std::env::args().collect();
    let cli_args = parse_args(&args);

    // Handle --help mode FIRST - show help and exit
    if cli_args.help_mode {
        print_help();
        return Ok(());
    }

    // Handle --validate-config mode - just validate and exit
    if cli_args.validate_config_mode {
        return run_validate_config_mode();
    }

    // Handle MCP server mode - MCP server is inherently headless
    // This must be checked before TUI mode logic since MCP servers don't use TUI
    if cli_args.mcp_server_mode {
        // Simple console-only logging for MCP server mode
        tracing_subscriber::fmt()
            .with_env_filter("paperboat=info,info")
            .init();

        // Get socket path from --socket argument (preferred) or PAPERBOAT_SOCKET env var (fallback)
        let socket_path = args
            .iter()
            .position(|a| a == "--socket")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .or_else(|| std::env::var("PAPERBOAT_SOCKET").ok())
            .expect("Socket path required: use --socket <path> or set PAPERBOAT_SOCKET");

        tracing::info!("Running in MCP server mode (socket={})", socket_path);
        return mcp_server::run_stdio_server(PathBuf::from(socket_path)).await;
    }

    // Extract values for use later
    #[allow(unused_variables)]
    let headless_mode = cli_args.headless_mode;
    let json_logs = cli_args.json_logs;
    let metrics_enabled = cli_args.metrics_enabled;
    let goal_arg = cli_args.goal;
    let cli_backend_config = cli_args.backend_config;

    // Create run directory first (so we can put logs there)
    let log_base =
        std::env::var("PAPERBOAT_LOG_DIR").unwrap_or_else(|_| ".paperboat/logs".to_string());
    let log_manager = Arc::new(RunLogManager::new(&log_base)?);
    let run_dir = log_manager.run_dir().clone();

    // Write app log to run directory (not a rolling file)
    let log_file = std::fs::File::create(run_dir.join("app.log"))?;
    let (non_blocking, _guard) = tracing_appender::non_blocking(log_file);

    // Conditionally set up logging based on TUI mode
    // TUI is enabled by default unless --headless is passed or stdout is not a terminal
    #[cfg(feature = "tui")]
    let tui_enabled = !headless_mode && std::io::IsTerminal::is_terminal(&std::io::stdout());

    // ==========================================================================
    // EARLY GOAL RESOLUTION (before TUI takes over terminal)
    // ==========================================================================
    // This must happen BEFORE the TUI starts because the interactive prompt uses stdin/stdout.
    // If no goal was provided via CLI args, we prompt the user interactively.
    let goal = if let Some(g) = goal_arg {
        g
    } else if let Some(g) = prompt_goal_interactively() {
        // No goal provided - try interactive prompt
        g
    } else {
        // Not a terminal or other error - exit with usage message
        eprintln!(
            "❌ No goal provided.\n\n\
            Usage:\n  \
              paperboat \"Your task description\"     # Direct prompt\n  \
              paperboat path/to/goal.txt            # Read goal from file\n  \
              paperboat                             # Interactive mode (terminal required)\n\n\
            Options:\n  \
              --backend <name>    Select AI backend (auggie, cursor)\n  \
              --headless          Disable TUI mode\n  \
              --help              Show help"
        );
        std::process::exit(1);
    };

    // ==========================================================================
    // EARLY BACKEND SELECTION (before TUI takes over terminal)
    // ==========================================================================
    // This must happen BEFORE the TUI starts because the prompt uses stdin/stdout.
    // We determine backend_kind and transport_kind here, then use them after TUI init.
    let (backend_kind, transport_kind) = if let Some(ref config) = cli_backend_config {
        // CLI flag takes precedence - no prompt needed
        let transport = config.effective_transport();
        (config.kind, transport)
    } else if let Some(config) = get_explicit_backend_config() {
        // Explicit config from env var or config file - no prompt needed
        let transport = config.effective_transport();
        (config.kind, transport)
    } else {
        // No explicit config - auto-detect available backends
        // This may prompt the user if multiple backends are available
        let available = discover_available_backends().await;

        match available.len() {
            0 => {
                eprintln!(
                    "❌ No AI backends available.\n\n\
                    Please install and authenticate at least one backend:\n  \
                    • Augment CLI: Install auggie and run 'auggie login'\n  \
                    • Cursor: Install cursor-agent and run 'agent login'\n\n\
                    You can also specify a backend explicitly with --backend <name>"
                );
                std::process::exit(1);
            }
            1 => {
                // Only one backend available - use it automatically
                let kind = available[0];
                let transport = kind.default_transport();
                (kind, transport)
            }
            _ => {
                // Multiple backends available - prompt user to select
                // This MUST happen before TUI starts since it uses stdin/stdout
                if let Some(kind) = prompt_backend_selection(&available) {
                    let transport = kind.default_transport();
                    (kind, transport)
                } else {
                    // Could not prompt (not a terminal) - use first available
                    let kind = available[0];
                    let transport = kind.default_transport();
                    (kind, transport)
                }
            }
        }
    };

    // Store TUI config channels for later use (after model_config is built)
    #[cfg(feature = "tui")]
    let tui_config_channels: Option<tui::TuiConfigChannels>;

    #[cfg(feature = "tui")]
    let tui_handle: Option<std::thread::JoinHandle<anyhow::Result<()>>> = if tui_enabled {
        // Initialize tui-logger for the App Logs panel
        tui_logger::init_logger(tui_logger::LevelFilter::Debug)
            .expect("Failed to initialize tui-logger");
        tui_logger::set_default_level(tui_logger::LevelFilter::Debug);

        // TUI mode: file logging + tui-logger (TUI takes over the terminal)
        let file_filter = tracing_subscriber::EnvFilter::new("paperboat=debug,debug");

        // JSON format for file logs provides structured data for analysis
        if json_logs {
            let file_layer = fmt::layer()
                .json()
                .with_target(true)
                .with_writer(non_blocking)
                .with_filter(file_filter);

            tracing_subscriber::registry()
                .with(file_layer)
                .with(tui_logger::TuiTracingSubscriberLayer)
                .init();
        } else {
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
                .with_writer(non_blocking)
                .with_filter(file_filter);

            tracing_subscriber::registry()
                .with(file_layer)
                .with(tui_logger::TuiTracingSubscriberLayer)
                .init();
        }

        // Subscribe to log events for TUI
        let log_event_rx = log_manager.subscribe();

        // Spawn event bridge with bidirectional config channels
        let (app_channels, tui_channels) = tui::spawn_event_bridge_with_config(log_event_rx);
        tui_config_channels = Some(app_channels);

        // Spawn TUI thread - it will wait for initial config via channels
        Some(std::thread::spawn(move || {
            tui::run_tui_with_channels(tui_channels)
        }))
    } else {
        tui_config_channels = None;
        // Headless/console mode: normal output with ANSI colors + file logging
        let file_filter = tracing_subscriber::EnvFilter::new("paperboat=debug,debug");
        let console_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "paperboat=info,info".into());

        // JSON format for both file and console logs when --json-logs is enabled
        if json_logs {
            let file_layer = fmt::layer()
                .json()
                .with_target(true)
                .with_writer(non_blocking)
                .with_filter(file_filter);

            let console_layer = fmt::layer()
                .json()
                .with_target(true)
                .with_filter(console_filter);

            tracing_subscriber::registry()
                .with(console_layer)
                .with(file_layer)
                .init();
        } else {
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
                .with_writer(non_blocking)
                .with_filter(file_filter);

            let console_layer = fmt::layer()
                .with_ansi(true)
                .with_target(true)
                .with_level(true)
                .with_filter(console_filter);

            tracing_subscriber::registry()
                .with(console_layer)
                .with(file_layer)
                .init();
        }

        None
    };

    // Non-TUI build: standard console output + file logging
    #[cfg(not(feature = "tui"))]
    {
        let file_filter = tracing_subscriber::EnvFilter::new("paperboat=debug,debug");
        let console_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "paperboat=info,info".into());

        // JSON format for both file and console logs when --json-logs is enabled
        if json_logs {
            let file_layer = fmt::layer()
                .json()
                .with_target(true)
                .with_writer(non_blocking)
                .with_filter(file_filter);

            let console_layer = fmt::layer()
                .json()
                .with_target(true)
                .with_filter(console_filter);

            tracing_subscriber::registry()
                .with(console_layer)
                .with(file_layer)
                .init();
        } else {
            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
                .with_writer(non_blocking)
                .with_filter(file_filter);

            let console_layer = fmt::layer()
                .with_ansi(true)
                .with_target(true)
                .with_level(true)
                .with_filter(console_filter);

            tracing_subscriber::registry()
                .with(console_layer)
                .with(file_layer)
                .init();
        }
    }

    // Initialize metrics collection if enabled
    if metrics_enabled {
        // Default port for Prometheus exporter (can be customized via PAPERBOAT_METRICS_PORT env var)
        let prometheus_port: u16 = std::env::var("PAPERBOAT_METRICS_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9090);
        metrics::init_metrics(true, prometheus_port)?;
    }

    tracing::info!("📁 Run directory: {:?}", run_dir);

    // Load agent configurations from config files
    // Project-level (.paperboat/agents/) overrides user-level (~/.paperboat/agents/)
    tracing::info!("📂 Loading agent configurations...");
    let loaded_configs = load_agent_configs()?;
    tracing::debug!("Loaded configs: {:?}", loaded_configs);

    // Log the backend selection (already determined before TUI init)
    tracing::info!(
        "🔌 Using backend '{}' with transport '{}'",
        backend_kind,
        transport_kind
    );

    // Create the backend for agent communication
    let backend = backend_kind.create();

    tracing::debug!("📡 Transport kind: {}", transport_kind);

    // Check authentication before proceeding
    if let Err(auth_err) = backend.check_auth() {
        eprintln!(
            "❌ Authentication error:\n\n{}",
            backend.auth_error_message()
        );
        tracing::error!("Backend authentication failed: {}", auth_err);
        std::process::exit(1);
    }

    // Discover available model tiers from the backend
    tracing::info!("🔍 Discovering available model tiers...");
    let available_tiers = match backend.available_tiers().await {
        Ok(tiers) => tiers,
        Err(e) => {
            eprintln!(
                "❌ Failed to discover model tiers from {} backend: {}\n\n\
                This may indicate a network issue or backend service problem.\n\
                Try running '{}' to verify your authentication.",
                backend.name(),
                e,
                backend.login_hint()
            );
            tracing::error!("Model tier discovery failed: {}", e);
            std::process::exit(1);
        }
    };
    tracing::info!(
        "📋 Available tiers: {:?}",
        available_tiers
            .iter()
            .map(models::ModelTier::as_str)
            .collect::<Vec<_>>()
    );

    // Build model configuration from loaded configs and available tiers
    let mut model_config = build_model_config(&loaded_configs, available_tiers)?;

    // Apply debug build override (uses Haiku for fast, cheap testing)
    // In release builds, this is a no-op
    model_config.apply_debug_override();

    model_config.validate()?;
    tracing::info!(
        "🎯 Model configuration: orchestrator={}, planner={}, implementer={}",
        model_config.orchestrator_model,
        model_config.planner_model,
        model_config.implementer_model
    );

    // Send initial model config to TUI if it's running
    #[cfg(feature = "tui")]
    if let Some(ref channels) = tui_config_channels {
        if let Err(e) = channels.initial_config_tx.send(model_config.clone()) {
            tracing::warn!("Failed to send initial config to TUI: {:?}", e);
        } else {
            tracing::debug!("Sent initial model config to TUI");
        }
    }

    // Run the orchestrator with signal handling for graceful shutdown
    let mut app =
        app::App::with_log_manager(model_config, log_manager, backend, transport_kind).await?;

    // Connect App to TUI config update channel if TUI is running
    #[cfg(feature = "tui")]
    if let Some(channels) = tui_config_channels {
        app.set_config_update_channel(channels.config_update_rx);
    }

    // Set up signal handler
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(shutdown_tx)));

    // Spawn signal handler task
    let signal_shutdown_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("Failed to set up SIGTERM handler");
            let mut sigint =
                signal(SignalKind::interrupt()).expect("Failed to set up SIGINT handler");
            let mut sighup = signal(SignalKind::hangup()).expect("Failed to set up SIGHUP handler");

            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("📴 Received SIGTERM, initiating shutdown...");
                }
                _ = sigint.recv() => {
                    tracing::info!("📴 Received SIGINT, initiating shutdown...");
                }
                _ = sighup.recv() => {
                    tracing::info!("📴 Received SIGHUP (terminal closed), initiating shutdown...");
                }
            }

            // Signal main task to shutdown
            let tx_opt = signal_shutdown_tx.lock().unwrap().take();
            if let Some(tx) = tx_opt {
                let _ = tx.send(());
            }
        }
    });

    // Run the main task with ability to be interrupted
    let result = tokio::select! {
        result = app.run(&goal) => result?,
        _ = &mut shutdown_rx => {
            tracing::info!("🛑 Shutdown requested, cleaning up...");
            app.shutdown().await?;
            std::process::exit(130); // Exit code for SIGINT
        }
    };

    // Normal completion - gracefully shutdown all processes
    app.shutdown().await?;

    // Self-improvement phase (only in paperboat's own repository)
    // This runs after shutdown so it doesn't interfere with the main task.
    // Errors are non-fatal - they're logged but don't affect the main task's result.
    {
        let task_manager = app.task_manager().read().await;
        match self_improve::maybe_run_self_improvement(&run_dir, &result, &task_manager).await {
            Ok(Some(outcome)) => {
                if outcome.success {
                    tracing::info!("🔄 Self-improvement completed successfully");
                    if let Some(msg) = outcome.message {
                        tracing::debug!("Self-improvement message: {}", msg);
                    }
                } else {
                    tracing::warn!("🔄 Self-improvement completed with issues");
                    if let Some(msg) = outcome.message {
                        tracing::warn!("Self-improvement message: {}", msg);
                    }
                }
            }
            Ok(None) => {
                // Self-improvement was skipped (disabled, not in paperboat repo, etc.)
                tracing::debug!("Self-improvement skipped");
            }
            Err(e) => {
                // Self-improvement failed, but this is non-fatal
                tracing::warn!("Self-improvement failed (non-fatal): {}", e);
            }
        }
    }

    // Wait for TUI thread to finish if it was started
    #[cfg(feature = "tui")]
    if let Some(handle) = tui_handle {
        // The TUI thread will exit when it receives the quit command or the channel closes
        match handle.join() {
            Ok(Ok(())) => {
                tracing::debug!("TUI thread exited cleanly");
            }
            Ok(Err(e)) => {
                // TUI had an error but we don't want to fail the whole app
                tracing::warn!("TUI thread exited with error: {}", e);
            }
            Err(_) => {
                // Thread panicked, but we handled it with the panic hook
                tracing::warn!("TUI thread panicked");
            }
        }
    }

    if result.success {
        println!("\n✅ Task completed successfully!");
    } else {
        println!("\n❌ Task failed");
    }

    Ok(())
}

/// Print help message and exit.
fn print_help() {
    println!(
        r#"paperboat - AI-powered task automation

USAGE:
    paperboat [OPTIONS] [GOAL]

ARGS:
    <GOAL>    Task description as a string, or path to a file containing the goal
              If omitted, paperboat will prompt interactively for input

EXAMPLES:
    paperboat "Fix all TODO comments in src/"
    paperboat plan.txt
    paperboat

OPTIONS:
    -h, --help              Show this help message and exit
    --backend <NAME>        Select AI backend (auggie, cursor, cursor:cli, cursor:acp)
    --headless              Disable TUI mode and run in console mode
    --json-logs             Enable JSON formatted logs
    --metrics               Enable Prometheus metrics collection
    --validate-config       Validate configuration files and exit

ENVIRONMENT:
    PAPERBOAT_BACKEND       Set default backend (same format as --backend)
    PAPERBOAT_LOG_DIR       Set custom log directory (default: .paperboat/logs)
    PAPERBOAT_JSON_LOGS     Enable JSON logs (set to "1" or "true")
    PAPERBOAT_METRICS       Enable metrics (set to "1" or "true")
    PAPERBOAT_METRICS_PORT  Set Prometheus metrics port (default: 9090)

CONFIG:
    Configuration files are loaded from:
      - ~/.paperboat/         User-level configuration
      - .paperboat/           Project-level configuration (overrides user-level)

For more information, see https://github.com/your-repo/paperboat
"#
    );
}

/// Run config validation mode.
///
/// Loads and validates all configuration files without running the app.
/// Prints validation results and exits with code 0 on success, 1 on failure.
#[allow(clippy::unnecessary_wraps)]
fn run_validate_config_mode() -> Result<()> {
    println!("🔍 Validating Paperboat configuration files...\n");

    // Try to load agent configs - this will validate them during loading
    match load_agent_configs() {
        Ok(loaded) => {
            println!("✅ Configuration files are valid!");
            println!();
            println!("Loaded configurations:");

            // Report orchestrator config
            if let Some(ref model) = loaded.orchestrator.model {
                println!("  Orchestrator: model = \"{model}\"");
            } else {
                println!("  Orchestrator: (using defaults)");
            }

            // Report planner config
            if let Some(ref model) = loaded.planner.model {
                println!("  Planner: model = \"{model}\"");
            } else {
                println!("  Planner: (using defaults)");
            }

            // Report implementer config
            if let Some(ref model) = loaded.implementer.model {
                println!("  Implementer: model = \"{model}\"");
            } else {
                println!("  Implementer: (using defaults)");
            }

            println!();
            println!(
                "Config locations searched:\n  User-level: ~/.paperboat/agents/\n  Project-level: .paperboat/agents/"
            );

            Ok(())
        }
        Err(e) => {
            eprintln!("❌ Configuration validation failed!\n");
            eprintln!("Error: {e}");

            // Print chain of errors for context
            let mut source = e.source();
            while let Some(cause) = source {
                eprintln!("Caused by: {cause}");
                source = cause.source();
            }

            std::process::exit(1);
        }
    }
}
