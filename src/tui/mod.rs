//! Terminal User Interface (TUI) module for Villalobos.
//!
//! This module provides an interactive terminal-based user interface for monitoring
//! and interacting with the orchestrator. It displays real-time agent output, task
//! progress, and system status.
//!
//! # Feature Flag
//!
//! This module is only compiled when the `tui` feature is enabled:
//!
//! ```toml
//! [dependencies]
//! paperboat = { version = "0.1", features = ["tui"] }
//! ```
//!
//! # Architecture
//!
//! The TUI is built using ratatui and crossterm, with a reactive architecture:
//!
//! ## Entry Points
//! - [`app::run_tui_with_channels`] - Main entry point that runs the TUI event loop
//! - [`spawn_event_bridge`] - Bridges async broadcast channel to sync mpsc for TUI thread
//!
//! ## State Management
//! - [`state`] - Core TUI state including focus management and event routing
//! - [`agent_tree_state`] - Agent hierarchy tree state from `AgentStarted` events
//! - [`task_list_state`] - Task list state from `TaskCreated` events
//!
//! ## UI Components
//! - [`layout`] - Panel layout calculations and screen partitioning
//! - [`events`] - Event handling and input processing
//! - [`widgets`] - Custom widgets for displaying agent output, tasks, etc.
//!
//! ## Application Loop
//! - [`app`] - TUI application runner and render loop
//!
//! ## Model Configuration
//! - [`ModelConfigUpdate`] - Message type for TUI -> App model configuration updates

mod agent_node;
mod agent_tree_state;
mod app;
mod events;
mod layout;
mod model_config_update;
mod state;
mod task_list_state;
pub mod widgets;

pub use app::run_tui_with_channels;

// Re-exports for backward compatibility and convenient access
// These are part of the public API but may not be used internally
#[allow(unused_imports)]
pub use agent_tree_state::{AgentNode, AgentStatus, AgentTreeState};
#[allow(unused_imports)]
pub use model_config_update::ModelConfigUpdate;
#[allow(unused_imports)]
pub use state::{FocusedPanel, TuiState};
#[allow(unused_imports)]
pub use task_list_state::{TaskDisplay, TaskListState};

use crate::models::ModelConfig;

/// Channels for App -> TUI communication (model config)
pub struct TuiConfigChannels {
    /// Sender for initial model config (App sends once after discovery)
    pub initial_config_tx: std::sync::mpsc::SyncSender<ModelConfig>,
    /// Receiver for model configuration updates from TUI (App reads)
    pub config_update_rx: tokio::sync::mpsc::Receiver<ModelConfigUpdate>,
}

/// Channels for TUI thread (passed to `run_tui_with_channels`)
pub struct TuiThreadChannels {
    /// Receiver for log events
    pub event_rx: std::sync::mpsc::Receiver<crate::logging::LogEvent>,
    /// Receiver for initial model config (TUI reads once)
    pub initial_config_rx: std::sync::mpsc::Receiver<ModelConfig>,
    /// Sender for model configuration updates (TUI sends to App)
    pub config_update_tx: std::sync::mpsc::SyncSender<ModelConfigUpdate>,
}

/// Spawns a bridge task that forwards `LogEvent`s from a tokio broadcast channel
/// to a `std::sync::mpsc` channel for consumption by the TUI thread.
///
/// This function handles the impedance mismatch between the async broadcast channel
/// (used by the main application) and the sync mpsc channel (used by the TUI thread).
///
/// # Arguments
///
/// * `broadcast_rx` - A receiver from the `RunLogManager`'s broadcast channel
///
/// # Returns
///
/// Returns an `mpsc::Receiver<LogEvent>` that the TUI thread can use to receive events.
///
/// # Threading Model
///
/// - The bridge task runs as a tokio task
/// - It receives events from the async broadcast channel
/// - It forwards events to the sync mpsc channel
/// - The mpsc channel has a bounded capacity of 1000 to prevent unbounded memory growth
///
/// # Error Handling
///
/// - If the broadcast channel lags (events are dropped), a warning is logged and the bridge continues
/// - If the TUI thread exits (mpsc send fails), the bridge task terminates gracefully
/// - If the broadcast channel is closed, the bridge task terminates gracefully
///
/// # Example
///
/// ```ignore
/// let run_log_manager = RunLogManager::new(".paperboat/logs")?;
/// let broadcast_rx = run_log_manager.subscribe();
/// let event_rx = spawn_event_bridge(broadcast_rx);
///
/// // Spawn TUI thread
/// std::thread::spawn(move || {
///     run_tui(event_rx)
/// });
/// ```
#[allow(dead_code)] // Public API for alternative event routing patterns
pub fn spawn_event_bridge(
    mut broadcast_rx: tokio::sync::broadcast::Receiver<crate::logging::LogEvent>,
) -> std::sync::mpsc::Receiver<crate::logging::LogEvent> {
    // Create a bounded sync mpsc channel for communicating with the TUI thread.
    // Using sync_channel with a buffer to prevent blocking the bridge task while
    // allowing backpressure if the TUI can't keep up.
    let (tui_tx, tui_rx) = std::sync::mpsc::sync_channel::<crate::logging::LogEvent>(1000);

    // Spawn the bridge task that runs for the lifetime of the application
    tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(event) => {
                    // Forward the event to the TUI thread via mpsc
                    // If send fails, the TUI thread has exited, so we should stop
                    if tui_tx.send(event).is_err() {
                        tracing::debug!("TUI thread has exited, stopping event bridge");
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    // Some events were dropped due to the TUI not keeping up
                    tracing::warn!(
                        "Event bridge lagged, {} events were dropped. TUI may be slow.",
                        count
                    );
                    // Continue processing - don't break on lag
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // The broadcast channel was closed (all senders dropped)
                    tracing::debug!("Broadcast channel closed, stopping event bridge");
                    break;
                }
            }
        }
    });

    tui_rx
}

/// Spawns a bidirectional bridge for TUI communication with model configuration support.
///
/// This creates all the channels needed for bidirectional communication between
/// the TUI thread and the main application:
///
/// - **App -> TUI**: Log events (broadcast -> sync mpsc via bridge task)
/// - **App -> TUI**: Initial model config (sync mpsc, sent once after discovery)
/// - **TUI -> App**: Model config updates (sync mpsc -> async mpsc via bridge task)
///
/// # Arguments
///
/// * `broadcast_rx` - A receiver from the `RunLogManager`'s broadcast channel
///
/// # Returns
///
/// Returns a tuple of:
/// - [`TuiConfigChannels`]: Channels for the App to send config and receive updates
/// - [`TuiThreadChannels`]: Channels to pass to the TUI thread
///
/// # Example
///
/// ```ignore
/// let run_log_manager = RunLogManager::new(".paperboat/logs")?;
/// let broadcast_rx = run_log_manager.subscribe();
/// let (app_channels, tui_channels) = spawn_event_bridge_with_config(broadcast_rx);
///
/// // Spawn TUI thread
/// std::thread::spawn(move || {
///     run_tui_with_channels(tui_channels)
/// });
///
/// // After model discovery, send initial config to TUI
/// app_channels.initial_config_tx.send(model_config.clone()).ok();
///
/// // App can listen for config updates
/// while let Some(update) = app_channels.config_update_rx.recv().await {
///     app.apply_model_config_update(update);
/// }
/// ```
pub fn spawn_event_bridge_with_config(
    mut broadcast_rx: tokio::sync::broadcast::Receiver<crate::logging::LogEvent>,
) -> (TuiConfigChannels, TuiThreadChannels) {
    // Create bounded sync channel for log events (TUI reads)
    let (event_tx, event_rx) = std::sync::mpsc::sync_channel::<crate::logging::LogEvent>(1000);

    // Create sync channel for initial model config (App sends once, TUI reads once)
    let (initial_config_tx, initial_config_rx) = std::sync::mpsc::sync_channel::<ModelConfig>(1);

    // Create sync channel for config updates (TUI writes, bridged to async)
    let (config_sync_tx, config_sync_rx) = std::sync::mpsc::sync_channel::<ModelConfigUpdate>(100);

    // Create async channel for config updates (App reads)
    let (config_async_tx, config_async_rx) = tokio::sync::mpsc::channel::<ModelConfigUpdate>(100);

    // Spawn bridge task for log events (async broadcast -> sync mpsc)
    tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(event) => {
                    if event_tx.send(event).is_err() {
                        tracing::debug!("TUI thread has exited, stopping event bridge");
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(
                        "Event bridge lagged, {} events were dropped. TUI may be slow.",
                        count
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!("Broadcast channel closed, stopping event bridge");
                    break;
                }
            }
        }
    });

    // Spawn bridge task for config updates (sync mpsc -> async mpsc)
    // This bridges the TUI's sync sends to the App's async receives
    tokio::spawn(async move {
        loop {
            // Use blocking task to receive from sync channel
            let update = tokio::task::spawn_blocking({
                let rx = config_sync_rx.try_recv();
                move || rx
            })
            .await;

            match update {
                Ok(Ok(config_update)) => {
                    if config_async_tx.send(config_update).await.is_err() {
                        tracing::debug!("Config update receiver dropped, stopping bridge");
                        break;
                    }
                }
                Ok(Err(std::sync::mpsc::TryRecvError::Empty)) => {
                    // No message available, yield and try again
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Ok(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    tracing::debug!("TUI config sender dropped, stopping bridge");
                    break;
                }
                Err(_) => {
                    // spawn_blocking failed, unlikely but handle gracefully
                    tracing::warn!("Config bridge spawn_blocking failed");
                    break;
                }
            }
        }
    });

    let app_channels = TuiConfigChannels {
        initial_config_tx,
        config_update_rx: config_async_rx,
    };

    let tui_channels = TuiThreadChannels {
        event_rx,
        initial_config_rx,
        config_update_tx: config_sync_tx,
    };

    (app_channels, tui_channels)
}
