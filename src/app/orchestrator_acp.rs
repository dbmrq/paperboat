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

        // Support both ACP format (sessionUpdate) and CLI format (type)
        let Some(session_update) = update
            .get("sessionUpdate")
            .or_else(|| update.get("type"))
            .and_then(|v| v.as_str())
        else {
            tracing::trace!("📨 Orchestrator update missing sessionUpdate/type field");
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
    ///
    /// This is critical for nested orchestrator scenarios: we must stop draining
    /// when we see a message for a DIFFERENT session, as that indicates our
    /// session's updates are exhausted and we'd be consuming another session's
    /// updates (e.g., the parent orchestrator's updates).
    pub(crate) async fn drain_orchestrator_messages(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) {
        loop {
            match self.acp_orchestrator.recv().await {
                Ok(msg) => {
                    let method = msg.get("method").and_then(|v| v.as_str());
                    if method != Some("session/update") {
                        // Non-update message, ignore and continue
                        tracing::trace!("Drain ignoring non-update message: {:?}", method);
                        continue;
                    }

                    let Some(params) = msg.get("params") else {
                        tracing::trace!("Drain: session/update missing params, continuing");
                        continue;
                    };

                    // Support both ACP format (sessionId) and CLI format (session_id)
                    let msg_session_id = params
                        .get("sessionId")
                        .or_else(|| params.get("session_id"))
                        .and_then(|v| v.as_str());

                    // Check if this message is for our session
                    if msg_session_id != Some(session_id) {
                        // Message for a different session - our session is done.
                        // This happens in nested orchestrator scenarios where after
                        // the child orchestrator finishes, recv() returns the parent's
                        // pending updates. We must NOT consume those.
                        tracing::debug!(
                            "Drain received message for different session {} (expected {}), stopping",
                            msg_session_id.unwrap_or("unknown"),
                            session_id
                        );
                        return;
                    }

                    // Message is for our session - process it
                    let Some(update) = params.get("update") else {
                        tracing::trace!("Drain: update field missing, continuing");
                        continue;
                    };

                    // Support both ACP format (sessionUpdate) and CLI format (type)
                    let session_update = update
                        .get("sessionUpdate")
                        .or_else(|| update.get("type"))
                        .and_then(|v| v.as_str());

                    match session_update {
                        // ACP: "session_finished", "agent_turn_finished", CLI: "complete"
                        Some("session_finished" | "agent_turn_finished" | "complete") => {
                            tracing::debug!(
                                "✅ Orchestrator session {} properly finished",
                                session_id
                            );
                            return;
                        }
                        Some("agent_message_chunk" | "agent_thought_chunk") => {
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
                Err(e) => {
                    tracing::debug!("Orchestrator channel closed during drain: {}", e);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::transport::{
        AgentTransport, SessionConfig, SessionInfo, SessionUpdate, ToolResult,
    };
    use crate::logging::{AgentType, RunLogManager};
    use crate::models::{ModelConfig, ModelTier};
    use crate::testing::{MockBackend, MockTransport};
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    struct QueueTransport {
        messages: Arc<Mutex<VecDeque<serde_json::Value>>>,
    }

    impl QueueTransport {
        fn new(messages: Vec<serde_json::Value>) -> Self {
            Self {
                messages: Arc::new(Mutex::new(messages.into())),
            }
        }
    }

    #[async_trait]
    impl AgentTransport for QueueTransport {
        async fn initialize(&mut self) -> Result<()> {
            Ok(())
        }

        async fn create_session(&mut self, _config: SessionConfig) -> Result<SessionInfo> {
            Ok(SessionInfo::new("unused"))
        }

        async fn send_prompt(&mut self, _session_id: &str, _prompt: &str) -> Result<()> {
            Ok(())
        }

        fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>> {
            None
        }

        async fn respond_to_tool(
            &mut self,
            _session_id: &str,
            _tool_use_id: &str,
            _result: ToolResult,
        ) -> Result<()> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }

        async fn recv(&mut self) -> Result<serde_json::Value> {
            self.messages
                .lock()
                .expect("pop queued ACP message")
                .pop_front()
                .ok_or_else(|| anyhow!("No queued orchestrator messages"))
        }
    }

    fn test_model_config() -> ModelConfig {
        ModelConfig::new(
            [ModelTier::Sonnet, ModelTier::Opus, ModelTier::Haiku]
                .into_iter()
                .collect(),
        )
    }

    #[tokio::test]
    async fn drain_orchestrator_messages_stops_after_cross_session_message() {
        let dir = tempdir().expect("create temp dir");
        let run_dir = dir.path().join("logs");
        let log_manager =
            Arc::new(RunLogManager::with_run_dir(run_dir.clone()).expect("create run dir"));
        let log_path = run_dir.join("orchestrator.log");

        let queued_messages = vec![
            serde_json::json!({
                "method": "session/update",
                "params": {
                    "sessionId": "child-session",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "text": "child output" }
                    }
                }
            }),
            serde_json::json!({
                "method": "session/update",
                "params": {
                    "sessionId": "parent-session",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "text": "parent output" }
                    }
                }
            }),
        ];

        let mut app = App::with_mock_transports(
            Box::new(MockBackend::new()),
            Box::new(QueueTransport::new(queued_messages)),
            Box::new(MockTransport::empty()),
            Box::new(MockTransport::empty()),
            test_model_config(),
            log_manager,
        );

        let (event_tx, _) = tokio::sync::broadcast::channel(8);
        let mut writer = AgentWriter::new(log_path.clone(), AgentType::Orchestrator, event_tx, 0)
            .await
            .expect("create orchestrator writer");

        app.drain_orchestrator_messages("child-session", &mut writer)
            .await;
        writer.finalize(true).await.expect("flush orchestrator log");

        let log_contents =
            std::fs::read_to_string(log_path).expect("read orchestrator log after drain");
        assert!(log_contents.contains("child output"));
        assert!(
            !log_contents.contains("parent output"),
            "drain should stop before logging another session's output"
        );
    }
}
