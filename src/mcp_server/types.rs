//! Type definitions for the MCP server.
//!
//! This module contains shared types used by the MCP server for communication
//! between the orchestrator agent and the main application.

/// Request sent from MCP server to the app via Unix socket.
///
/// Wraps a tool call with a unique request ID for response correlation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolRequest {
    /// Unique identifier for correlating responses
    pub request_id: String,
    /// The actual tool call
    pub tool_call: ToolCall,
}

/// Response sent from the app back to the MCP server via Unix socket.
///
/// Contains the result of executing a tool call, correlated by request ID.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolResponse {
    /// Request ID this response corresponds to
    pub request_id: String,
    /// Whether the operation succeeded
    pub success: bool,
    /// Human-readable summary of what was done
    pub summary: String,
    /// Optional list of files that were modified
    pub files_modified: Option<Vec<String>>,
    /// Optional error message if the operation failed
    pub error: Option<String>,
}

impl ToolResponse {
    /// Create a successful response
    pub fn success(request_id: String, summary: String) -> Self {
        Self {
            request_id,
            success: true,
            summary,
            files_modified: None,
            error: None,
        }
    }

    /// Create a successful response with file list
    pub fn success_with_files(request_id: String, summary: String, files: Vec<String>) -> Self {
        Self {
            request_id,
            success: true,
            summary,
            files_modified: Some(files),
            error: None,
        }
    }

    /// Create a failure response
    pub fn failure(request_id: String, error: String) -> Self {
        Self {
            request_id,
            success: false,
            summary: String::new(),
            files_modified: None,
            error: Some(error),
        }
    }
}

/// Tool call from an agent.
///
/// Represents the different operations that can be requested by agents
/// via the MCP protocol. These calls are sent from the MCP server to the main
/// application via a Unix socket.
///
/// # Variants
///
/// - `Decompose` - Request to break down a task into smaller subtasks
/// - `Implement` - Request to implement a specific task
/// - `Complete` - Signal that the orchestrator has finished processing
/// - `WritePlan` - Write a structured plan (planner agent only)
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
    /// Signal completion of an agent's work.
    Complete {
        /// Whether the work was successful.
        success: bool,
        /// Optional message providing details about the completion.
        message: Option<String>,
    },
    /// Write a structured plan (used by planner agents).
    /// This stores the plan for the orchestrator to retrieve.
    WritePlan {
        /// The structured plan content.
        plan: String,
    },
}

impl ToolCall {
    /// Returns the type of tool call as a string.
    pub fn tool_type(&self) -> &'static str {
        match self {
            ToolCall::Decompose { .. } => "decompose",
            ToolCall::Implement { .. } => "implement",
            ToolCall::Complete { .. } => "complete",
            ToolCall::WritePlan { .. } => "write_plan",
        }
    }
}

