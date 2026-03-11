mod acp;
mod agents;
mod app;
mod backend;
mod cli;
mod config;
mod error;
mod ipc;
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
use backend::{discover_available_backends, prompt_backend_selection};
use cli::{parse_args, print_help, print_no_goal_error, prompt_goal_interactively};
use config::{build_model_config, get_explicit_backend_config, load_agent_configs};
use logging::RunLogManager;
use std::sync::Arc;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};

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

        // Get socket address from --socket argument (preferred) or PAPERBOAT_SOCKET env var (fallback)
        let socket_address_str = args
            .iter()
            .position(|a| a == "--socket")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .or_else(|| std::env::var("PAPERBOAT_SOCKET").ok())
            .expect("Socket address required: use --socket <address> or set PAPERBOAT_SOCKET");

        tracing::info!("Running in MCP server mode (socket={})", socket_address_str);
        let socket_address = ipc::IpcAddress::from_string(&socket_address_str);
        return mcp_server::run_stdio_server(socket_address).await;
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
        print_no_goal_error();
        std::process::exit(1);
    };

    // ==========================================================================
    // TUI INITIALIZATION (if enabled, start TUI immediately for splash screen)
    // ==========================================================================
    // Store TUI config channels for later use
    #[cfg(feature = "tui")]
    let mut tui_config_channels: Option<tui::TuiConfigChannels>;

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

        // Spawn TUI thread - starts showing splash immediately
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

    // ==========================================================================
    // BACKEND SELECTION (via TUI popup or console prompt)
    // ==========================================================================
    let (backend_kind, transport_kind) = if let Some(ref config) = cli_backend_config {
        // CLI flag takes precedence - no selection needed
        let transport = config.effective_transport();
        (config.kind, transport)
    } else if let Some(config) = get_explicit_backend_config() {
        // Explicit config from env var or config file - no selection needed
        let transport = config.effective_transport();
        (config.kind, transport)
    } else {
        // Auto-detect available backends
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
                // Send to TUI so it knows (single backend = no popup needed)
                #[cfg(feature = "tui")]
                if let Some(ref channels) = tui_config_channels {
                    let _ = channels.available_backends_tx.send(available.clone());
                }
                let kind = available[0];
                let transport = kind.default_transport();
                (kind, transport)
            }
            _ => {
                // Multiple backends available
                #[cfg(feature = "tui")]
                {
                    if let Some(ref mut channels) = tui_config_channels {
                        // TUI mode: send backends to TUI, wait for selection via popup
                        if let Err(e) = channels.available_backends_tx.send(available.clone()) {
                            tracing::warn!("Failed to send backends to TUI: {:?}", e);
                            // Fall back to first available
                            let kind = available[0];
                            let transport = kind.default_transport();
                            (kind, transport)
                        } else if let Some(kind) = channels.selected_backend_rx.recv().await {
                            // Wait for TUI to send back the selected backend
                            let transport = kind.default_transport();
                            (kind, transport)
                        } else {
                            // Channel closed, fall back to first
                            tracing::warn!("Backend selection channel closed, using default");
                            let kind = available[0];
                            let transport = kind.default_transport();
                            (kind, transport)
                        }
                    } else {
                        // TUI feature enabled but not running, prompt in console
                        if let Some(kind) = prompt_backend_selection(&available) {
                            let transport = kind.default_transport();
                            (kind, transport)
                        } else {
                            let kind = available[0];
                            let transport = kind.default_transport();
                            (kind, transport)
                        }
                    }
                }

                #[cfg(not(feature = "tui"))]
                {
                    // Non-TUI mode: prompt in console
                    if let Some(kind) = prompt_backend_selection(&available) {
                        let transport = kind.default_transport();
                        (kind, transport)
                    } else {
                        let kind = available[0];
                        let transport = kind.default_transport();
                        (kind, transport)
                    }
                }
            }
        }
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

        #[cfg(windows)]
        {
            // On Windows, we use Ctrl+C handler which is cross-platform in tokio
            if let Err(e) = tokio::signal::ctrl_c().await {
                tracing::error!("Failed to listen for Ctrl+C: {}", e);
                return;
            }
            tracing::info!("📴 Received Ctrl+C, initiating shutdown...");

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

    // Print human action items if any exist
    {
        let task_manager = app.task_manager().read().await;
        if let Some(human_actions) = task_manager.format_human_actions_required() {
            println!("{human_actions}");
        }
    }

    if result.success {
        println!("\n✅ Task completed successfully!");
    } else {
        println!("\n❌ Task failed");
    }

    Ok(())
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
