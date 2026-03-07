mod acp;
mod agents;
mod app;
mod config;
mod error;
mod logging;
mod mcp_server;
mod metrics;
mod models;
mod tasks;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(feature = "tui")]
mod tui;
mod types;

use anyhow::Result;
use config::{build_model_config, load_agent_configs};
use logging::RunLogManager;
use models::discover_models;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};

/// Parsed command-line arguments.
#[allow(clippy::struct_excessive_bools)]
struct CliArgs {
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
    /// The goal/task to accomplish
    goal: Option<String>,
}

/// Parse command-line arguments and extract flags and the goal.
fn parse_args(args: &[String]) -> CliArgs {
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

    // Goal is the first non-flag argument after the program name
    let goal = args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with("--") && !arg.starts_with('-'))
        .cloned();

    CliArgs {
        headless_mode,
        mcp_server_mode,
        validate_config_mode,
        json_logs,
        metrics_enabled,
        goal,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let args: Vec<String> = std::env::args().collect();
    let cli_args = parse_args(&args);

    // Handle --validate-config mode FIRST - just validate and exit
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

    // Get goal from parsed arguments
    let goal =
        goal_arg.unwrap_or_else(|| "Create a simple hello world program in Python".to_string());

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

    // Discover available models from auggie
    tracing::info!("🔍 Discovering available models...");
    let available_models = discover_models().await?;
    tracing::info!(
        "📋 Available models: {:?}",
        available_models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>()
    );

    // Build model configuration from loaded configs and available models
    let mut model_config = build_model_config(&loaded_configs, &available_models)?;

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
    let mut app = app::App::with_log_manager(model_config, log_manager).await?;

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
