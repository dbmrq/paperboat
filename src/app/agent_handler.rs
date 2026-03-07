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
    if success {
        tm.update_status(
            &task_id.to_string(),
            &TaskStatus::Complete {
                success: true,
                summary: message.unwrap_or("Task completed").to_string(),
            },
        );
        tracing::info!("📋 Task {} marked as Complete", task_id);
    } else {
        tm.update_status(
            &task_id.to_string(),
            &TaskStatus::Failed {
                error: message.unwrap_or("Task failed").to_string(),
            },
        );
        tracing::info!("📋 Task {} marked as Failed", task_id);
    }
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
                            let response = ToolResponse::failure(
                                request.request_id,
                                format!(
                                    "Worker agents can only call complete(). \
                                     Tool '{}' is not available.",
                                    other.tool_type()
                                ),
                            );
                            let _ = response_tx.send(response);
                        }
                    }
                }

                // Handle ACP session messages (for logging and completion detection)
                Some(msg) = session_rx.recv() => {
                    if let Some(params) = msg.get("params") {
                        if let Some(update) = params.get("update") {
                            if let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
                                match session_update {
                                    "session_finished" => {
                                        tracing::debug!(
                                            "[{}] Agent {} received session_finished without complete call",
                                            agent_name, role
                                        );
                                        // Clean up socket
                                        socket_handle.cleanup();
                                        // Treat as failure since agent didn't call complete()
                                        return AgentCompletionData {
                                            success: false,
                                            message: Some(format!("Agent finished without calling complete() for task: {task}")),
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
                                        }
                                    }
                                    "tool_result" => {
                                        // Log tool errors (successes are not logged to keep logs clean)
                                        if let Some(is_error) = update.get("isError").and_then(|v| v.as_bool()) {
                                            if is_error {
                                                let tool_name = update.get("toolName")
                                                    .and_then(|t| t.as_str())
                                                    .unwrap_or("unknown");
                                                let content = update.get("content")
                                                    .and_then(|c| c.as_str())
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
        AgentCompletionData {
            success: false,
            message: Some(format!("Agent timed out for task: {task}")),
            notes: None,
            add_tasks: None,
        }
    }
}

/// Internal helper to wait for an agent to complete via session messages.
///
/// This is a simplified version that just waits for the session to finish.
/// Used by the old sequential mode - kept for fallback compatibility.
#[allow(dead_code)]
pub async fn wait_for_agent_completion(
    mut session_rx: tokio::sync::mpsc::Receiver<serde_json::Value>,
    timeout: std::time::Duration,
) -> bool {
    let result = tokio::time::timeout(timeout, async {
        while let Some(msg) = session_rx.recv().await {
            // Check for session_finished
            if let Some(params) = msg.get("params") {
                if let Some(update) = params.get("update") {
                    if let Some(session_update) =
                        update.get("sessionUpdate").and_then(|v| v.as_str())
                    {
                        if session_update == "session_finished" {
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
