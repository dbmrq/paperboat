mod acp;
mod app;
mod config;
mod error;
mod mcp_server;
mod models;
mod types;

use anyhow::Result;
use models::{discover_models, ModelConfig};
use std::path::PathBuf;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with both console and file output
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "villalobos=debug,info".into());

    // Set up file appender with daily rotation
    let log_dir = std::env::var("VILLALOBOS_LOG_DIR").unwrap_or_else(|_| "logs".to_string());
    let file_appender = tracing_appender::rolling::daily(&log_dir, "villalobos.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Console layer: with ANSI colors for readability
    let console_layer = fmt::layer()
        .with_ansi(true)
        .with_target(true)
        .with_level(true);

    // File layer: no ANSI colors, with timestamps, targets, and structured fields
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_writer(non_blocking);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    // Check if we're running in MCP server mode
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--mcp-server" {
        // Get socket path from environment variable
        let socket_path = std::env::var("VILLALOBOS_SOCKET")
            .expect("VILLALOBOS_SOCKET environment variable not set");

        tracing::info!("Running in MCP server mode");
        return mcp_server::run_stdio_server(PathBuf::from(socket_path)).await;
    }

    // Get goal from command line
    let goal = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Create a simple hello world program in Python".to_string());

    tracing::info!("Starting orchestrator with goal: {}", goal);

    // Discover available models from auggie
    tracing::info!("🔍 Discovering available models...");
    let available_models = discover_models().await?;
    tracing::info!(
        "📋 Available models: {:?}",
        available_models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
    );

    // Create model configuration with sensible defaults
    let model_config = ModelConfig::new(available_models);
    model_config.validate()?;
    tracing::info!(
        "🎯 Model configuration: orchestrator={}, planner={}, implementer={}",
        model_config.orchestrator_model,
        model_config.planner_model,
        model_config.implementer_model
    );

    // Run the orchestrator
    let mut app = app::App::new(model_config).await?;
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
