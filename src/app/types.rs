//! Types and constants for the app module.

use crate::mcp_server::{ToolRequest, ToolResponse};
use tokio::sync::mpsc;

/// Message sent from tool handlers back to the orchestrator loop.
///
/// This is public to allow test harnesses to inject tool calls directly
/// without going through Unix sockets.
#[derive(Debug)]
pub enum ToolMessage {
    /// A tool request that needs processing
    Request {
        /// The tool request with `request_id` and `tool_call`
        request: ToolRequest,
        /// Channel to send response back
        response_tx: tokio::sync::oneshot::Sender<ToolResponse>,
    },
}

/// Path to the orchestrator-specific auggie cache directory.
/// This directory has a settings.json with editing tools removed,
/// forcing the orchestrator to delegate work to worker agents.
pub const ORCHESTRATOR_CACHE_DIR: &str = "~/.villalobos/augment-orchestrator";

/// Tools to remove from the orchestrator agent.
/// These are editing/execution tools that should only be available to worker agents.
pub const ORCHESTRATOR_REMOVED_TOOLS: &[&str] = &[
    "str-replace-editor",
    "save-file",
    "remove-files",
    "apply_patch",
    "launch-process",
    "kill-process",
    "read-process",
    "write-process",
    "list-processes",
    "web-search",
    "web-fetch",
];

/// Path to the planner-specific auggie cache directory.
/// This directory has a settings.json with built-in task management tools removed,
/// so the planner uses our custom write_plan tool instead.
pub const PLANNER_CACHE_DIR: &str = "~/.villalobos/augment-planner";

/// Tools to remove from the planner agent.
/// These are built-in task management tools that conflict with our custom planning workflow.
pub const PLANNER_REMOVED_TOOLS: &[&str] = &[
    "view_tasklist",
    "reorganize_tasklist",
    "update_tasks",
    "add_tasks",
];

/// System prompt for the orchestrator agent (loaded from prompts/orchestrator.txt)
pub const ORCHESTRATOR_PROMPT: &str = include_str!("../../prompts/orchestrator.txt");

/// System prompt for the planner agent (loaded from prompts/planner.txt)
pub const PLANNER_PROMPT: &str = include_str!("../../prompts/planner.txt");

/// System prompt for the implementer agent (loaded from prompts/implementer.txt)
/// Note: Kept for reference; actual prompt now comes from AgentRegistry.
#[allow(dead_code)]
pub const IMPLEMENTER_PROMPT: &str = include_str!("../../prompts/implementer.txt");

/// Truncate a string for logging, adding "..." if truncated.
pub fn truncate_for_log(s: &str, max_len: usize) -> String {
    // Replace newlines with spaces for cleaner log output
    let single_line = s.replace('\n', " ");
    if single_line.len() <= max_len {
        single_line
    } else {
        format!("{}...", &single_line[..max_len])
    }
}

/// Format a duration as a human-readable string for logging.
pub fn format_duration_human(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 60 {
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{mins}m {secs}s")
    } else if secs > 0 {
        format!("{secs}s")
    } else {
        format!("{}ms", duration.as_millis())
    }
}

/// Type alias for the channel receiver for tool messages
pub type ToolReceiver = mpsc::Receiver<ToolMessage>;

/// Type alias for the channel sender for tool messages
pub type ToolSender = mpsc::Sender<ToolMessage>;
