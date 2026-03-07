//! Session drain and message handling functions.

use super::App;
use crate::logging::AgentWriter;
use crate::types::SessionOutput;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use tokio::sync::mpsc;

/// Strip MCP server prefixes from tool names for cleaner logging.
///
/// Augment's ACP prefixes tool names with the MCP server name (e.g., `complete_paperboat-planner`
/// instead of just `complete`). This function extracts the base tool name for readability.
///
/// Returns the original title if no prefix pattern is found.
fn strip_mcp_prefix(title: &str) -> &str {
    // Pattern: toolname_servername (e.g., "complete_paperboat-planner")
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

impl App {
    /// Handle a worker session message from ACP.
    /// Returns Ok(true) if the session is finished, Ok(false) to continue processing.
    pub(crate) async fn handle_worker_session_message(
        &mut self,
        msg: &serde_json::Value,
        session_id: &str,
        writer: &mut AgentWriter,
        output: &mut SessionOutput,
        seen_unhandled: &mut HashSet<String>,
    ) -> Result<bool> {
        let method = msg.get("method").and_then(|v| v.as_str());

        if method != Some("session/update") {
            return Ok(false);
        }

        let Some(params) = msg.get("params") else {
            return Ok(false);
        };

        let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

        // Ignore messages for other sessions
        if msg_session_id != Some(session_id) {
            return Ok(false);
        }

        let Some(update) = params.get("update") else {
            return Ok(false);
        };

        let session_update = update.get("sessionUpdate").and_then(|v| v.as_str());

        let Some(session_update) = session_update else {
            return Ok(false);
        };

        match session_update {
            // Message chunks - stream to stdout and collect
            "agent_message_chunk" | "agent_thought_chunk" => {
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    output.append(text);
                    let _ = writer.write_message_chunk(text).await;
                }
            }
            "tool_call" => {
                if let Some(raw_title) = update.get("title").and_then(|t| t.as_str()) {
                    let title = strip_mcp_prefix(raw_title);
                    let _ = writer.write_tool_call(title).await;
                    tracing::info!("🔧 tool call: {}", title);
                }
            }
            "tool_result" => {
                let raw_title = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let title = strip_mcp_prefix(raw_title);
                let is_error = update
                    .get("isError")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let content = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let _ = writer.write_tool_result(title, is_error, content).await;
                if is_error {
                    tracing::error!("❌ tool failed: {} - {}", title, content);
                }
            }
            // Also handle explicit completion signals from ACP
            "agent_turn_finished" | "session_finished" => {
                tracing::info!("✅ Session {} completed: {}", session_id, session_update);
                return Ok(true);
            }
            // Tool progress updates (streaming output from tools)
            "tool_call_update" => {
                let raw_tool_name = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let tool_name = strip_mcp_prefix(raw_tool_name);
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    // Stream tool progress for UI observation
                    let _ = writer.write_tool_progress(tool_name, text).await;
                    tracing::trace!("🔄 tool progress: {} - {} chars", tool_name, text.len());
                }
            }
            // Silently ignore known non-essential updates
            "agent_turn_started" | "thinking_start" | "thinking_end" => {}
            // Log unknown types once per type to help diagnose issues
            other => {
                if seen_unhandled.insert(other.to_string()) {
                    tracing::debug!("📨 Unhandled session update type: {}", other);
                }
            }
        }

        Ok(false)
    }

    /// Drain remaining messages from a per-session receiver after `complete()` is called.
    /// This ensures we don't leave stale messages in the notification channel.
    /// Waits until we receive `session_finished` or the caller times out.
    ///
    /// This is a static method that takes the receiver as a parameter to avoid
    /// holding any locks on `self` during the drain loop.
    pub(crate) async fn drain_session_messages_from_rx(
        session_rx: &mut mpsc::Receiver<Value>,
        session_id: &str,
        writer: &mut AgentWriter,
    ) {
        while let Some(msg) = session_rx.recv().await {
            let method = msg.get("method").and_then(|v| v.as_str());
            if method == Some("session/update") {
                if let Some(params) = msg.get("params") {
                    let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

                    // Only process messages for this session (should always match
                    // since the router sends only this session's messages)
                    if msg_session_id == Some(session_id) {
                        if let Some(update) = params.get("update") {
                            let session_update =
                                update.get("sessionUpdate").and_then(|v| v.as_str());

                            if let Some(update_type) = session_update {
                                match update_type {
                                    "session_finished" | "agent_turn_finished" => {
                                        tracing::debug!(
                                            "✅ Session {} properly finished",
                                            session_id
                                        );
                                        return;
                                    }
                                    "agent_message_chunk" | "agent_thought_chunk" => {
                                        // Continue logging any remaining output
                                        if let Some(text) = update
                                            .get("content")
                                            .and_then(|c| c.get("text"))
                                            .and_then(|t| t.as_str())
                                        {
                                            let _ = writer.write_message_chunk(text).await;
                                        }
                                    }
                                    _ => {
                                        // Silently ignore other update types during drain
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        tracing::debug!("Session channel closed during drain: {}", session_id);
    }

    /// Drain remaining messages directly from ACP clients after `complete()` is called.
    /// This is used in direct mode (for tests with mock clients).
    /// Polls both planner and worker clients to handle all session types.
    /// Waits until we receive `session_finished` or the caller times out.
    pub(crate) async fn drain_session_messages_direct(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) {
        // Track which clients have been exhausted
        let mut worker_exhausted = false;
        let mut planner_exhausted = false;

        loop {
            // If both clients are exhausted, we're done draining
            if worker_exhausted && planner_exhausted {
                tracing::debug!("Both clients exhausted during drain");
                return;
            }

            // Poll both worker and planner clients
            tokio::select! {
                worker_result = self.acp_worker.recv(), if !worker_exhausted => {
                    match worker_result {
                        Ok(msg) => {
                            if self.handle_drain_message(&msg, session_id, writer).await {
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Worker channel closed during drain: {}", e);
                            worker_exhausted = true;
                        }
                    }
                }
                planner_result = self.acp_planner.recv(), if !planner_exhausted => {
                    match planner_result {
                        Ok(msg) => {
                            if self.handle_drain_message(&msg, session_id, writer).await {
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Planner channel closed during drain: {}", e);
                            planner_exhausted = true;
                        }
                    }
                }
            }
        }
    }

    /// Handle a single message during drain. Returns true if session is finished.
    async fn handle_drain_message(
        &self,
        msg: &serde_json::Value,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> bool {
        let method = msg.get("method").and_then(|v| v.as_str());
        if method == Some("session/update") {
            if let Some(params) = msg.get("params") {
                let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

                // Only process messages for this session
                if msg_session_id == Some(session_id) {
                    if let Some(update) = params.get("update") {
                        let session_update = update.get("sessionUpdate").and_then(|v| v.as_str());

                        if let Some(update_type) = session_update {
                            match update_type {
                                "session_finished" | "agent_turn_finished" => {
                                    tracing::debug!("✅ Session {} properly finished", session_id);
                                    return true;
                                }
                                "agent_message_chunk" | "agent_thought_chunk" => {
                                    if let Some(text) = update
                                        .get("content")
                                        .and_then(|c| c.get("text"))
                                        .and_then(|t| t.as_str())
                                    {
                                        let _ = writer.write_message_chunk(text).await;
                                    }
                                }
                                _ => {
                                    // Silently ignore other update types during drain
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }
}
