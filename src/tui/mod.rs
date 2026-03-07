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
//! - [`run_tui`] - Main entry point that runs the TUI event loop
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

mod agent_tree_state;
mod app;
mod events;
mod layout;
mod state;
mod task_list_state;
pub mod widgets;

pub use app::run_tui;

// Re-exports for backward compatibility and convenient access
// These are part of the public API but may not be used internally
#[allow(unused_imports)]
pub use agent_tree_state::{AgentNode, AgentStatus, AgentTreeState};
#[allow(unused_imports)]
pub use state::{FocusedPanel, TuiState};
#[allow(unused_imports)]
pub use task_list_state::{TaskDisplay, TaskListState};

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
