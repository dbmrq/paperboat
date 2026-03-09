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

// NOTE: Cache directory paths are managed by the backend abstraction.
// See `crate::backend` module for backend implementations (auggie, cursor).

// NOTE: Tool removal configuration is centralized in `crate::agents::config`.
// Use ORCHESTRATOR_CONFIG and PLANNER_CONFIG from there for removed tool lists.

/// System prompt for the orchestrator agent (loaded from prompts/orchestrator.txt)
pub const ORCHESTRATOR_PROMPT: &str = include_str!("../../prompts/orchestrator.txt");

/// System prompt for the planner agent (loaded from prompts/planner.txt)
pub const PLANNER_PROMPT: &str = include_str!("../../prompts/planner.txt");

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
