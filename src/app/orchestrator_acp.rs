//! Orchestrator ACP message handling and drain functions.

use super::App;
use crate::logging::AgentWriter;

impl App {
    /// Handle ACP messages and write to the agent's log file.
    pub(crate) async fn handle_acp_message_with_writer(
        &self,
        msg: &serde_json::Value,
        writer: &mut AgentWriter,
    ) {
        let method = msg.get("method").and_then(|v| v.as_str());

        if method != Some("session/update") {
            tracing::trace!("📨 Orchestrator received non-update message: {:?}", method);
            return;
        }

        let Some(params) = msg.get("params") else {
            tracing::trace!("📨 Orchestrator session/update missing params");
            return;
        };

        let Some(update) = params.get("update") else {
            tracing::trace!("📨 Orchestrator session/update missing update field");
            return;
        };

        let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) else {
            tracing::trace!("📨 Orchestrator update missing sessionUpdate field");
            return;
        };

        match session_update {
            "agent_message_chunk" | "agent_thought_chunk" => {
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    tracing::trace!(
                        "📨 Writing orchestrator message chunk: {} chars",
                        text.len()
                    );
                    let _ = writer.write_message_chunk(text).await;
                }
            }
            "tool_call" => {
                if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                    let _ = writer.write_tool_call(title).await;
                    tracing::info!("🔧 orchestrator tool call: {}", title);
                }
            }
            "tool_result" => {
                let title = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
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
                    tracing::error!("❌ orchestrator tool failed: {} - {}", title, content);
                }
            }
            "agent_turn_finished" | "session_finished" => {
                tracing::debug!("📨 Orchestrator session event: {}", session_update);
            }
            // Tool progress updates (streaming output from tools)
            "tool_call_update" => {
                let tool_name = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    let _ = writer.write_tool_progress(tool_name, text).await;
                    tracing::trace!(
                        "🔄 orchestrator tool progress: {} - {} chars",
                        tool_name,
                        text.len()
                    );
                }
            }
            // Silently ignore known non-essential updates
            "agent_turn_started" | "thinking_start" | "thinking_end" => {}
            _ => {
                tracing::trace!(
                    "📨 Orchestrator unhandled sessionUpdate: {}",
                    session_update
                );
            }
        }
    }

    /// Drain remaining messages for an orchestrator session after `complete()` is called.
    /// Similar to `drain_session_messages` but uses `acp_orchestrator` channel.
    pub(crate) async fn drain_orchestrator_messages(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) {
        loop {
            match self.acp_orchestrator.recv().await {
                Ok(msg) => {
                    let method = msg.get("method").and_then(|v| v.as_str());
                    if method == Some("session/update") {
                        if let Some(params) = msg.get("params") {
                            let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

                            // Only process messages for this session
                            if msg_session_id == Some(session_id) {
                                if let Some(update) = params.get("update") {
                                    let session_update =
                                        update.get("sessionUpdate").and_then(|v| v.as_str());

                                    if let Some(update_type) = session_update {
                                        match update_type {
                                            "session_finished" | "agent_turn_finished" => {
                                                tracing::debug!(
                                                    "✅ Orchestrator session {} properly finished",
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
                Err(e) => {
                    tracing::debug!("Orchestrator channel closed during drain: {}", e);
                    return;
                }
            }
        }
    }
}
