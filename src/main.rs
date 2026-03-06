mod acp;
mod agents;
mod app;
mod config;
mod error;
mod logging;
mod mcp_server;
mod models;
mod tasks;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod types;

use anyhow::Result;
use config::{build_model_config, load_agent_configs};
use logging::RunLogManager;
use models::discover_models;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};

#[tokio::main]
async fn main() -> Result<()> {
    // Check if we're running in MCP server mode (before setting up full logging)
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--mcp-server" {
        // Simple console-only logging for MCP server mode
        tracing_subscriber::fmt()
            .with_env_filter("villalobos=info,info")
            .init();

        let socket_path = std::env::var("VILLALOBOS_SOCKET")
            .expect("VILLALOBOS_SOCKET environment variable not set");

        tracing::info!("Running in MCP server mode");
        return mcp_server::run_stdio_server(PathBuf::from(socket_path)).await;
    }

    // Create run directory first (so we can put logs there)
    let log_base = std::env::var("VILLALOBOS_LOG_DIR").unwrap_or_else(|_| "logs".to_string());
    let log_manager = Arc::new(RunLogManager::new(&log_base)?);
    let run_dir = log_manager.run_dir().clone();

    // Initialize logging with both console and file output
    // Console filter: respects RUST_LOG env var, defaults to info level
    let console_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "villalobos=info,info".into());

    // File filter: always captures debug level for diagnostics
    let file_filter = tracing_subscriber::EnvFilter::new("villalobos=debug,debug");

    // Write app log to run directory (not a rolling file)
    let log_file = std::fs::File::create(run_dir.join("app.log"))?;
    let (non_blocking, _guard) = tracing_appender::non_blocking(log_file);

    // Console layer: with ANSI colors for readability
    let console_layer = fmt::layer()
        .with_ansi(true)
        .with_target(true)
        .with_level(true)
        .with_filter(console_filter);

    // File layer: no ANSI colors, with timestamps
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_writer(non_blocking)
        .with_filter(file_filter);

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    // Get goal from command line
    let goal = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Create a simple hello world program in Python".to_string());

    tracing::info!("📁 Run directory: {:?}", run_dir);
    tracing::info!("Starting orchestrator with goal: {}", goal);

    // Load agent configurations from config files
    // Project-level (.villalobos/agents/) overrides user-level (~/.villalobos/agents/)
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
    let model_config = build_model_config(&loaded_configs, &available_models)?;
    model_config.validate()?;
    tracing::info!(
        "🎯 Model configuration: orchestrator={}, planner={}, implementer={}",
        model_config.orchestrator_model,
        model_config.planner_model,
        model_config.implementer_model
    );

    // Run the orchestrator
    let mut app = app::App::with_log_manager(model_config, log_manager).await?;
    let result = app.run(&goal).await?;

    // Gracefully shutdown all processes
    app.shutdown().await?;

    if result.success {
        println!("\n✅ Task completed successfully!");
    } else {
        println!("\n❌ Task failed");
    }

    Ok(())
}
