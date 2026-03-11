//! Agent handler functions for processing tool calls and managing agent completion.
//!
//! This module provides utilities for handling agent socket communication,
//! processing tool calls, and managing task completion status updates.

use super::socket::AgentSocketHandle;
use super::types::ToolMessage;
use crate::logging::AgentWriter;
use crate::mcp_server::{SuggestedTask, ToolCall, ToolResponse};
use crate::tasks::{TaskManager, TaskStatus};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Strip MCP server prefixes from tool names for cleaner logging.
///
/// Augment's ACP prefixes tool names with the MCP server name (e.g., `complete_paperboat-implementer`
/// instead of just `complete`). This function extracts the base tool name for readability.
///
/// Returns the original title if no prefix pattern is found.
fn strip_mcp_prefix(title: &str) -> &str {
    // Pattern: toolname_servername (e.g., "complete_paperboat-implementer-abc123")
    // We want to extract just "toolname"
    if let Some(underscore_pos) = title.find('_') {
        let potential_prefix = &title[..underscore_pos];
        let potential_suffix = &title[underscore_pos + 1..];
        // Check if suffix looks like our MCP server name
        if potential_suffix.starts_with("paperboat-") {
            return potential_prefix;
        }
    }
    title
}

/// Build a helpful error message when an agent tries to use an unavailable tool.
///
/// Provides targeted guidance based on the specific tool that was called,
/// especially for common mistakes like calling `create_task` instead of using
/// `add_tasks` parameter in `complete()`.
fn build_tool_rejection_message(tool_type: &str) -> String {
    match tool_type {
        "create_task" => "Tool 'create_task' is not available to worker agents. \
             To suggest new tasks, use the 'add_tasks' parameter when calling \
             complete(success=..., message=..., add_tasks=[...]). \
             The add_tasks array accepts objects with 'name' and 'description' fields."
            .to_string(),
        _ => {
            format!(
                "Worker agents can only call complete(). \
                 Tool '{tool_type}' is not available. \
                 Call complete(success=true/false, message=\"...\") when done."
            )
        }
    }
}

/// Internal result of an agent completion, including notes and suggested tasks.
pub struct AgentCompletionData {
    pub success: bool,
    pub message: Option<String>,
    pub notes: Option<String>,
    pub add_tasks: Option<Vec<SuggestedTask>>,
}

/// Update task status when an agent completes.
pub async fn update_task_completion(
    task_manager: &Arc<RwLock<TaskManager>>,
    task_id: &str,
    success: bool,
    message: Option<&str>,
) {
    let mut tm = task_manager.write().await;
    let id = task_id.to_string();
    let status = if success {
        tracing::info!("📋 Task {} marked as Complete", task_id);
        TaskStatus::Complete {
            success: true,
            summary: message.unwrap_or("Task completed").to_string(),
        }
    } else {
        tracing::info!("📋 Task {} marked as Failed", task_id);
        TaskStatus::Failed {
            error: message.unwrap_or("Task failed").to_string(),
        }
    };
    tm.update_status(&id, &status);
}

/// Run the agent handler task, processing tool calls until completion.
///
/// This handles the agent's dedicated socket, responding to tool calls
/// (especially the `Complete` call that signals the agent is done).
pub async fn run_agent_handler(
    mut socket_handle: AgentSocketHandle,
    mut session_rx: tokio::sync::mpsc::Receiver<serde_json::Value>,
    timeout: std::time::Duration,
    role: &str,
    task: &str,
    agent_name: &str,
    writer: &mut AgentWriter,
) -> AgentCompletionData {
    let result = tokio::time::timeout(timeout, async {
        loop {
            tokio::select! {
                // Handle tool calls from the agent's socket
                Some(tool_msg) = socket_handle.tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;

                    let depth = writer.depth();
                    match &request.tool_call {
                        ToolCall::Complete { success, message, notes, add_tasks } => {
                            tracing::info!(
                                "[L{}] ✅ [{}] Agent {} complete: success={}, message={:?}",
                                depth, agent_name, role, success, message
                            );

                            if let Some(msg) = message {
                                let _ = writer.write_result(msg).await;
                            }

                            // Log notes if provided
                            if let Some(n) = notes {
                                tracing::info!("[L{}] 📝 [{}] Agent notes: {}", depth, agent_name, n);
                            }

                            // Log add_tasks if provided
                            if let Some(tasks) = add_tasks {
                                tracing::info!("[L{}] 📋 [{}] Agent suggested {} task(s)", depth, agent_name, tasks.len());
                            }

                            // Send success response
                            let response = ToolResponse::success(
                                request.request_id,
                                message.clone().unwrap_or_else(|| "Done".to_string()),
                            );
                            let _ = response_tx.send(response);

                            // Clean up socket before returning
                            socket_handle.cleanup();

                            return AgentCompletionData {
                                success: *success,
                                message: message.clone(),
                                notes: notes.clone(),
                                add_tasks: add_tasks.clone(),
                            };
                        }
                        other => {
                            // Worker agents should only call complete()
                            // Log warning and return error
                            tracing::warn!(
                                "⚠️ [{}] Agent {} made unexpected tool call: {:?}",
                                agent_name, role, other.tool_type()
                            );
                            let error_msg = build_tool_rejection_message(other.tool_type());
                            let response = ToolResponse::failure(request.request_id, error_msg);
                            let _ = response_tx.send(response);
                        }
                    }
                }

                // Handle ACP session messages (for logging and completion detection)
                Some(msg) = session_rx.recv() => {
                    if let Some(params) = msg.get("params") {
                        if let Some(update) = params.get("update") {
                            // Support both ACP format (sessionUpdate) and CLI format (type)
                            if let Some(session_update) = update
                                .get("sessionUpdate")
                                .or_else(|| update.get("type"))
                                .and_then(|v| v.as_str())
                            {
                                match session_update {
                                    // ACP: "session_finished", CLI: "complete"
                                    "session_finished" | "complete" => {
                                        tracing::debug!(
                                            "[{}] Agent {} received {} without complete call",
                                            agent_name, role, session_update
                                        );
                                        // Clean up socket
                                        socket_handle.cleanup();
                                        // Treat as failure since agent didn't call complete()
                                        // Note: This can happen if the agent's complete() call failed due to
                                        // socket connection issues. The agent may have finished its work
                                        // but couldn't signal completion.
                                        return AgentCompletionData {
                                            success: false,
                                            message: Some(format!(
                                                "Agent finished without calling complete() for task: {task}. \
                                                 This may indicate the agent completed its work but the MCP socket \
                                                 connection failed when calling complete(). Check logs for socket errors."
                                            )),
                                            notes: None,
                                            add_tasks: None,
                                        };
                                    }
                                    "agent_message_chunk" | "agent_thought_chunk" => {
                                        // Write agent output to log file
                                        if let Some(text) = update
                                            .get("content")
                                            .and_then(|c| c.get("text"))
                                            .and_then(|t| t.as_str())
                                        {
                                            let _ = writer.write_message_chunk(text).await;
                                        }
                                    }
                                    "tool_call" => {
                                        // Log tool calls made by the agent
                                        if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                                            // Strip MCP prefix if present (e.g., "mcp_paperboat-implementer_" -> "")
                                            let clean_title = strip_mcp_prefix(title);
                                            let _ = writer.write_tool_call(clean_title).await;
                                            tracing::info!("🔧 [{}] tool call: {}", agent_name, clean_title);

                                            // Record tool call metric
                                            crate::metrics::record_tool_call(clean_title);
                                        }
                                    }
                                    "tool_result" => {
                                        // Log tool errors (successes are not logged to keep logs clean)
                                        if let Some(is_error) = update.get("isError").and_then(serde_json::Value::as_bool) {
                                            if is_error {
                                                let tool_name = update.get("toolName")
                                                    .and_then(serde_json::Value::as_str)
                                                    .unwrap_or("unknown");
                                                let content = update.get("content")
                                                    .and_then(serde_json::Value::as_str)
                                                    .unwrap_or("");
                                                let _ = writer.write_tool_result(tool_name, true, content).await;
                                            }
                                        }
                                    }
                                    _ => {
                                        // Ignore other update types (tool_progress, etc.)
                                    }
                                }
                            }
                        }
                    }
                }

                // Both channels closed unexpectedly
                else => {
                    tracing::warn!(
                        "[{}] Agent {} channels closed unexpectedly",
                        agent_name, role
                    );
                    socket_handle.cleanup();
                    return AgentCompletionData {
                        success: false,
                        message: Some(format!("Agent channels closed for task: {task}")),
                        notes: None,
                        add_tasks: None,
                    };
                }
            }
        }
    })
    .await;

    if let Ok(data) = result {
        data
    } else {
        tracing::error!(
            "⏰ [{}] Agent {} timed out after {:?}",
            agent_name,
            role,
            timeout
        );
        // Socket cleanup happens when socket_handle is dropped
        let timeout_secs = timeout.as_secs();
        AgentCompletionData {
            success: false,
            message: Some(format!(
                "Agent timed out for task: {task} (after {timeout_secs}s). \
                 The task may be too large or include long-running operations. \
                 Consider breaking it into smaller pieces or using focused verification."
            )),
            notes: None,
            add_tasks: None,
        }
    }
}

/// Internal helper to wait for an agent to complete via session messages.
///
/// This is a simplified version that just waits for the session to finish.
/// Used by the old sequential mode - kept for fallback compatibility.
#[allow(dead_code)] // Kept for fallback compatibility with sequential mode
pub async fn wait_for_agent_completion(
    mut session_rx: tokio::sync::mpsc::Receiver<serde_json::Value>,
    timeout: std::time::Duration,
) -> bool {
    let result = tokio::time::timeout(timeout, async {
        while let Some(msg) = session_rx.recv().await {
            // Check for session_finished (ACP) or complete (CLI)
            if let Some(params) = msg.get("params") {
                if let Some(update) = params.get("update") {
                    // Support both ACP format (sessionUpdate) and CLI format (type)
                    if let Some(session_update) = update
                        .get("sessionUpdate")
                        .or_else(|| update.get("type"))
                        .and_then(|v| v.as_str())
                    {
                        // ACP: "session_finished", CLI: "complete"
                        if session_update == "session_finished" || session_update == "complete" {
                            return true;
                        }
                    }
                }
            }
        }
        false
    })
    .await;

    result.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::socket::setup_agent_socket;
    use crate::logging::{AgentType, AgentWriter};
    use tempfile::tempdir;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn run_agent_handler_propagates_missing_complete_as_failure_and_cleans_socket() {
        let dir = tempdir().expect("create temp dir");
        let log_path = dir.path().join("implementer.log");
        let (event_tx, _) = broadcast::channel(8);
        let mut writer =
            AgentWriter::new(log_path, AgentType::Implementer { index: 1 }, event_tx, 0)
                .await
                .expect("create writer");

        let socket_handle = setup_agent_socket("handler-missing-complete")
            .await
            .expect("create test socket");
        let socket_address = socket_handle.socket_address.clone();

        let (session_tx, session_rx) = tokio::sync::mpsc::channel(4);
        session_tx
            .send(serde_json::json!({
                "params": {
                    "update": {
                        "sessionUpdate": "session_finished"
                    }
                }
            }))
            .await
            .expect("send session_finished");
        drop(session_tx);

        let completion = run_agent_handler(
            socket_handle,
            session_rx,
            std::time::Duration::from_secs(1),
            "implementer",
            "Investigate startup failure",
            "implementer-001",
            &mut writer,
        )
        .await;

        writer.finalize(false).await.expect("flush log");

        assert!(!completion.success);
        let message = completion
            .message
            .expect("missing complete should produce a failure message");
        assert!(message.contains("without calling complete()"));
        assert!(message.contains("Investigate startup failure"));
        assert!(
            !socket_address.exists(),
            "socket endpoint should be cleaned up after handler failure"
        );
    }

    #[test]
    fn build_tool_rejection_message_create_task_has_helpful_guidance() {
        let msg = super::build_tool_rejection_message("create_task");

        // Should guide user to use add_tasks parameter in complete()
        assert!(
            msg.contains("add_tasks"),
            "Should mention add_tasks parameter"
        );
        assert!(
            msg.contains("complete("),
            "Should mention the complete() tool"
        );
        assert!(
            !msg.contains("is not available."),
            "Should NOT use generic rejection for create_task"
        );
    }

    #[test]
    fn build_tool_rejection_message_other_tools_uses_generic_message() {
        let msg = super::build_tool_rejection_message("spawn_agents");

        assert!(msg.contains("spawn_agents"), "Should include the tool name");
        assert!(
            msg.contains("is not available"),
            "Should use generic rejection"
        );
        assert!(
            msg.contains("complete(success=true/false"),
            "Should guide to use complete()"
        );
    }
}
