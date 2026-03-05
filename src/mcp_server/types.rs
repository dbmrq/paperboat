//! Type definitions for the MCP server.
//!
//! This module contains shared types used by the MCP server for communication
//! between the orchestrator agent and the main application.

/// Tool call from the orchestrator agent.
///
/// Represents the different operations that can be requested by the orchestrator
/// via the MCP protocol. These calls are sent from the MCP server to the main
/// application via a Unix socket.
///
/// # Variants
///
/// - `Decompose` - Request to break down a task into smaller subtasks
/// - `Implement` - Request to implement a specific task
/// - `Complete` - Signal that the orchestrator has finished processing
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolCall {
    /// Request to decompose a task into subtasks.
    Decompose {
        /// The task description to decompose.
        task: String,
    },
    /// Request to implement a specific task.
    Implement {
        /// The task description to implement.
        task: String,
    },
    /// Signal completion of the orchestrator's work.
    Complete {
        /// Whether the orchestration was successful.
        success: bool,
        /// Optional message providing details about the completion.
        message: Option<String>,
    },
}

