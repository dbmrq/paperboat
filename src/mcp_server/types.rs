//! Type definitions for the MCP server.
//!
//! This module contains shared types used by the MCP server for communication
//! between the orchestrator agent and the main application.

/// Specification for an agent to be spawned.
///
/// Used by the orchestrator to describe worker agents when making
/// `spawn_agents` tool calls.
///
/// There are two ways to specify an agent:
/// 1. **By task_id**: Just provide `task_id` (e.g., "task001"). The task description
///    is looked up from TaskManager, and role defaults to "implementer".
/// 2. **Explicitly**: Provide `role` and `task` directly. Use this for custom agents
///    or when not using the task tracking system.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentSpec {
    /// The role of the agent (e.g., "implementer", "verifier", "explorer", "custom").
    /// Defaults to "implementer" when task_id is provided.
    #[serde(default)]
    pub role: Option<String>,
    /// The task to be performed by this agent.
    /// Optional when task_id is provided (looked up from TaskManager).
    #[serde(default)]
    pub task: Option<String>,
    /// Task ID linking this agent to a tracked task (e.g., "task001").
    /// When provided:
    /// - Task description is looked up automatically
    /// - Role defaults to "implementer"
    /// - Status is auto-updated: InProgress → Complete/Failed
    #[serde(default)]
    pub task_id: Option<String>,
    /// Custom prompt (required for role="custom", optional for others)
    #[serde(default)]
    pub prompt: Option<String>,
    /// Explicit tool whitelist (required for role="custom")
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

/// A fully resolved agent specification with all required fields populated.
///
/// Created from an `AgentSpec` by resolving task_id lookups and applying defaults.
#[derive(Debug, Clone)]
pub struct ResolvedAgentSpec {
    /// The role of the agent (resolved from spec or defaulted to "implementer")
    pub role: String,
    /// The task description (resolved from TaskManager if task_id was provided)
    pub task: String,
    /// The task ID if this agent is linked to a tracked task
    pub task_id: Option<String>,
    /// Custom prompt (for role="custom")
    pub prompt: Option<String>,
    /// Explicit tool whitelist (for role="custom")
    pub tools: Option<Vec<String>>,
}

impl AgentSpec {
    /// Resolve this spec into a fully-populated ResolvedAgentSpec.
    ///
    /// If task_id is provided, looks up the task description from the provided lookup function.
    /// Returns an error if:
    /// - task_id is provided but the task is not found
    /// - Neither task_id nor task is provided
    pub fn resolve<F>(&self, task_lookup: F) -> Result<ResolvedAgentSpec, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        // Resolve task: either from task_id lookup or explicit task field
        let task = if let Some(ref tid) = self.task_id {
            task_lookup(tid).ok_or_else(|| format!("Task '{}' not found", tid))?
        } else if let Some(ref t) = self.task {
            t.clone()
        } else {
            return Err("Either 'task_id' or 'task' must be provided".to_string());
        };

        // Resolve role: explicit or default to "implementer"
        let role = self
            .role
            .clone()
            .unwrap_or_else(|| "implementer".to_string());

        Ok(ResolvedAgentSpec {
            role,
            task,
            task_id: self.task_id.clone(),
            prompt: self.prompt.clone(),
            tools: self.tools.clone(),
        })
    }
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

/// A task suggested by an agent during completion.
///
/// Agents can suggest tasks they discovered were needed but are outside their scope.
/// These are added to the TaskManager and the orchestrator can act on them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SuggestedTask {
    /// The name of the suggested task.
    pub name: String,
    /// What needs to be done.
    pub description: String,
    /// Optional dependencies (by task name or ID).
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
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
/// - `CreateTask` - Create a task (planner agent only)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolCall {
    /// Request to decompose a task into subtasks.
    /// Either `task_id` or `task` must be provided.
    Decompose {
        /// Task ID to decompose (e.g., "task001"). Looked up from TaskManager.
        #[serde(default)]
        task_id: Option<String>,
        /// Explicit task description to decompose.
        #[serde(default)]
        task: Option<String>,
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
        /// Optional notes for future agents/orchestrator (insights, decisions, warnings).
        /// Use this to leave context that will help other agents or inform the orchestrator.
        #[serde(default)]
        notes: Option<String>,
        /// Optional tasks to add to the plan.
        /// Use this for work you discovered was needed but is outside your current scope.
        #[serde(default)]
        add_tasks: Option<Vec<SuggestedTask>>,
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
    /// Set the goal summary (used by planner agents).
    /// Called first to define what success looks like before creating tasks.
    SetGoal {
        /// A concise summary of the user's goal.
        summary: String,
        /// Success criteria / acceptance conditions.
        #[serde(default)]
        acceptance_criteria: Option<String>,
    },
}

impl ToolCall {
    /// Returns the type of tool call as a string.
    pub const fn tool_type(&self) -> &'static str {
        match self {
            Self::Decompose { .. } => "decompose",
            Self::SpawnAgents { .. } => "spawn_agents",
            Self::Complete { .. } => "complete",
            Self::CreateTask { .. } => "create_task",
            Self::SetGoal { .. } => "set_goal",
        }
    }
}
