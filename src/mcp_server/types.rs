//! Type definitions for the MCP server.
//!
//! This module contains shared types used by the MCP server for communication
//! between the orchestrator agent and the main application.

/// Specification for an agent to be spawned.
///
/// Used by the orchestrator to describe worker agents when making
/// `spawn_agents` tool calls.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentSpec {
    /// The role of the agent (e.g., "implementer", "verifier", "explorer", "custom")
    pub role: String,
    /// The task to be performed by this agent
    pub task: String,
    /// Custom prompt (required for role="custom", optional for others)
    #[serde(default)]
    pub prompt: Option<String>,
    /// Explicit tool whitelist (required for role="custom")
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

/// Wait mode for spawned agents.
///
/// Controls how the orchestrator waits for spawned agents to complete.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum WaitMode {
    /// Wait for all agents to complete before continuing
    #[default]
    All,
    /// Wait for any one agent to complete before continuing
    Any,
    /// Don't wait; fire and forget
    None,
}

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
    pub const fn success(request_id: String, summary: String) -> Self {
        Self {
            request_id,
            success: true,
            summary,
            files_modified: None,
            error: None,
        }
    }

    /// Create a successful response with file list
    #[allow(dead_code)]
    pub const fn success_with_files(
        request_id: String,
        summary: String,
        files: Vec<String>,
    ) -> Self {
        Self {
            request_id,
            success: true,
            summary,
            files_modified: Some(files),
            error: None,
        }
    }

    /// Create a failure response
    pub const fn failure(request_id: String, error: String) -> Self {
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
/// - `SpawnAgents` - Request to spawn one or more worker agents
/// - `Complete` - Signal that the orchestrator has finished processing
/// - `WritePlan` - Submit a plan (planner agent only)
/// - `CreateTask` - Create a task (planner agent only)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolCall {
    /// Request to decompose a task into subtasks.
    Decompose {
        /// The task description to decompose.
        task: String,
    },
    /// Request to spawn one or more worker agents.
    SpawnAgents {
        /// The agents to spawn with their roles and tasks.
        agents: Vec<AgentSpec>,
        /// How to wait for the spawned agents.
        #[serde(default)]
        wait: WaitMode,
    },
    /// Signal completion of an agent's work.
    Complete {
        /// Whether the work was successful.
        success: bool,
        /// Optional message providing details about the completion.
        message: Option<String>,
    },
    /// Submit a plan (used by planner agents).
    WritePlan {
        /// The plan content as markdown.
        plan: String,
    },
    /// Create a task (used by planner agents).
    /// This creates a task for the orchestrator to track and execute.
    CreateTask {
        /// The name of the task.
        name: String,
        /// The description of the task.
        description: String,
        /// Names of tasks that this task depends on.
        dependencies: Vec<String>,
    },
}

impl ToolCall {
    /// Returns the type of tool call as a string.
    pub const fn tool_type(&self) -> &'static str {
        match self {
            Self::Decompose { .. } => "decompose",
            Self::SpawnAgents { .. } => "spawn_agents",
            Self::Complete { .. } => "complete",
            Self::WritePlan { .. } => "write_plan",
            Self::CreateTask { .. } => "create_task",
        }
    }
}
