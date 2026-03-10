//! Mock transport for testing.
//!
//! Provides a mock implementation of `AgentTransport` that returns scripted
//! responses from a `MockScenario`, enabling deterministic testing without
//! requiring a live agent process.
//!
//! This wraps and delegates to `MockAcpClient` to preserve backward compatibility
//! while providing the new `AgentTransport` interface.

use crate::acp::AcpClientTrait;
#[cfg(test)]
use crate::acp::SessionMode;
use crate::app::ToolMessage;
use crate::backend::transport::{
    AgentTransport, SessionConfig, SessionInfo, SessionUpdate, ToolResult,
};
use crate::testing::{MockAcpClient, MockScenario, MockToolInterceptor};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Mock transport that implements `AgentTransport` for testing.
///
/// This wraps `MockAcpClient` and converts between the old and new interfaces,
/// allowing tests written for the ACP client interface to continue working
/// while the codebase migrates to the transport abstraction.
pub struct MockTransport {
    /// The underlying mock ACP client
    inner: MockAcpClient,
    /// Sender for session updates
    update_tx: mpsc::Sender<SessionUpdate>,
    /// Receiver for session updates (taken once)
    #[allow(dead_code)]
    update_rx: Option<mpsc::Receiver<SessionUpdate>>,
    /// Current session ID
    current_session_id: Option<String>,
}

impl MockTransport {
    /// Create a new mock transport from a scenario.
    pub fn from_scenario(scenario: &MockScenario) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            inner: MockAcpClient::from_scenario(scenario),
            update_tx: tx,
            update_rx: Some(rx),
            current_session_id: None,
        }
    }

    /// Create an empty mock transport (no sessions).
    pub fn empty() -> Self {
        Self::from_scenario(&MockScenario::default())
    }

    /// Set the tool channel for injecting tool calls.
    pub fn with_tool_channel(
        mut self,
        tool_tx: mpsc::Sender<ToolMessage>,
        tool_interceptor: Arc<Mutex<MockToolInterceptor>>,
    ) -> Self {
        self.inner = self.inner.with_tool_channel(tool_tx, tool_interceptor);
        self
    }

    /// Get all captured prompts as (`session_id`, prompt) pairs.
    pub fn captured_prompts(&self) -> &[(String, String)] {
        self.inner.captured_prompts()
    }

    /// Get the number of sessions created.
    pub const fn sessions_created_count(&self) -> usize {
        self.inner.sessions_created_count()
    }

    /// Check if all sessions have been used.
    pub fn is_exhausted(&self) -> bool {
        self.inner.is_exhausted()
    }

    /// Convert ACP notification JSON to `SessionUpdate`.
    fn convert_to_session_update(
        notification: &Value,
        default_session_id: &str,
    ) -> Option<SessionUpdate> {
        let method = notification.get("method")?.as_str()?;
        if method != "session/update" {
            return Some(SessionUpdate::Raw {
                session_id: None,
                data: notification.clone(),
            });
        }

        let params = notification.get("params")?;
        let session_id = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or(default_session_id)
            .to_string();

        // Get update field and sessionUpdate type
        let update = params.get("update")?;
        let session_update_type = update.get("sessionUpdate").and_then(|v| v.as_str())?;

        match session_update_type {
            "agent_message_chunk" | "agent_thought_chunk" => {
                let content = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SessionUpdate::Text {
                    session_id,
                    content,
                })
            }
            "tool_call" => {
                let tool_name = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                Some(SessionUpdate::ToolUse {
                    session_id,
                    tool_use_id: String::new(),
                    tool_name,
                    input: Value::Null,
                })
            }
            "agent_turn_finished" | "session_finished" => Some(SessionUpdate::Completion {
                session_id,
                result: None,
                success: true,
            }),
            _ => Some(SessionUpdate::Raw {
                session_id: Some(session_id),
                data: notification.clone(),
            }),
        }
    }
}

#[async_trait]
impl AgentTransport for MockTransport {
    async fn initialize(&mut self) -> Result<()> {
        self.inner.initialize().await
    }

    async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo> {
        let response = self
            .inner
            .session_new(
                &config.model,
                config.mcp_servers,
                &config.workspace,
                config.mode,
            )
            .await?;
        self.current_session_id = Some(response.session_id.clone());
        Ok(SessionInfo::new(response.session_id))
    }

    async fn send_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()> {
        self.inner.session_prompt(session_id, prompt).await
    }

    fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>> {
        self.update_rx.take()
    }

    async fn respond_to_tool(
        &mut self,
        _session_id: &str,
        _tool_use_id: &str,
        _result: ToolResult,
    ) -> Result<()> {
        // Mock transport doesn't need to handle tool responses -
        // they're handled by the MockToolInterceptor
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.inner.shutdown().await
    }

    /// Receive the next raw notification (legacy compatibility).
    ///
    /// This method supports the legacy polling pattern used in App code.
    /// It gets the next notification from the inner `MockAcpClient` and optionally
    /// converts it to a `SessionUpdate` that's sent to the update channel.
    async fn recv(&mut self) -> Result<Value> {
        let notification = self.inner.recv().await?;

        // Also convert to SessionUpdate and send to channel
        if let Some(update) = Self::convert_to_session_update(
            &notification,
            self.current_session_id.as_deref().unwrap_or(""),
        ) {
            let _ = self.update_tx.send(update).await;
        }

        Ok(notification)
    }

    /// Take the raw notification receiver from the inner client.
    ///
    /// This is used for backward compatibility with the existing router pattern.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        self.inner.take_notification_rx()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockSessionBuilder;

    // ========================================================================
    // Basic Functionality Tests
    // ========================================================================

    #[tokio::test]
    async fn test_mock_transport_basic() {
        let scenario = MockScenario {
            planner_sessions: vec![MockSessionBuilder::new("planner-001")
                .with_message_chunk("Planning...", 100)
                .with_turn_finished(50)
                .build()],
            ..Default::default()
        };

        let mut transport = MockTransport::from_scenario(&scenario);

        // Initialize should succeed
        transport.initialize().await.unwrap();

        // Create a session with proper SessionConfig
        let config = SessionConfig::new("planner-model", "/tmp").with_mode(SessionMode::Plan);
        let session = transport.create_session(config).await.unwrap();
        assert_eq!(session.session_id, "planner-001");
        assert_eq!(transport.sessions_created_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_transport_recv() {
        let scenario = MockScenario {
            implementer_sessions: vec![MockSessionBuilder::new("impl-001")
                .with_message_chunk("Working...", 100)
                .build()],
            ..Default::default()
        };

        let mut transport = MockTransport::from_scenario(&scenario);
        transport.initialize().await.unwrap();

        let config = SessionConfig::new("implementer", "/tmp").with_mode(SessionMode::Agent);
        let _session = transport.create_session(config).await.unwrap();

        // Should be able to receive notification
        let notification = transport.recv().await.unwrap();
        assert_eq!(notification["method"], "session/update");
    }

    #[tokio::test]
    async fn test_mock_transport_shutdown() {
        let mut transport = MockTransport::empty();
        transport.initialize().await.unwrap();
        transport.shutdown().await.unwrap();
    }

    // ========================================================================
    // AgentTransport Trait Tests
    // ========================================================================

    #[tokio::test]
    async fn test_mock_transport_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockTransport>();
    }

    #[tokio::test]
    async fn test_mock_transport_empty() {
        let transport = MockTransport::empty();
        assert_eq!(transport.sessions_created_count(), 0);
        // Empty scenario has no sessions to exhaust, so is_exhausted returns true
        // (all 0 planner, 0 orchestrator, 0 implementer sessions are "used")
        assert!(transport.is_exhausted());
    }

    #[tokio::test]
    async fn test_mock_transport_take_notifications_returns_once() {
        let mut transport = MockTransport::empty();

        // First call returns Some
        let rx1 = transport.take_notifications();
        assert!(rx1.is_some());

        // Second call returns None
        let rx2 = transport.take_notifications();
        assert!(rx2.is_none());
    }

    #[tokio::test]
    async fn test_mock_transport_respond_to_tool_succeeds() {
        let mut transport = MockTransport::empty();
        transport.initialize().await.unwrap();

        // respond_to_tool should succeed (it's a no-op for mock)
        let result = transport
            .respond_to_tool("session-1", "tool-1", ToolResult::success("done"))
            .await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Session Config Tests
    // ========================================================================

    #[tokio::test]
    async fn test_mock_transport_stores_session_id() {
        let scenario = MockScenario {
            implementer_sessions: vec![MockSessionBuilder::new("impl-123").build()],
            ..Default::default()
        };

        let mut transport = MockTransport::from_scenario(&scenario);
        transport.initialize().await.unwrap();

        let config = SessionConfig::new("model", "/workspace").with_mode(SessionMode::Agent);
        let session = transport.create_session(config).await.unwrap();

        assert_eq!(session.session_id, "impl-123");
        assert_eq!(transport.current_session_id, Some("impl-123".to_string()));
    }

    #[tokio::test]
    async fn test_mock_transport_send_prompt_captures_prompts() {
        let scenario = MockScenario {
            implementer_sessions: vec![MockSessionBuilder::new("impl-001")
                .with_message_chunk("Response", 50)
                .build()],
            ..Default::default()
        };

        let mut transport = MockTransport::from_scenario(&scenario);
        transport.initialize().await.unwrap();

        let config = SessionConfig::new("model", "/workspace").with_mode(SessionMode::Agent);
        let session = transport.create_session(config).await.unwrap();

        transport
            .send_prompt(&session.session_id, "Hello world")
            .await
            .unwrap();

        let prompts = transport.captured_prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].1, "Hello world");
    }

    // ========================================================================
    // SessionUpdate Conversion Tests
    // ========================================================================

    #[test]
    fn test_convert_text_update() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "sessionId": "sess-1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "Hello"}
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());

        if let Some(SessionUpdate::Text {
            session_id,
            content,
        }) = update
        {
            assert_eq!(session_id, "sess-1");
            assert_eq!(content, "Hello");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_convert_tool_call_update() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "sessionId": "sess-1",
                "update": {
                    "sessionUpdate": "tool_call",
                    "title": "codebase-retrieval"
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());

        if let Some(SessionUpdate::ToolUse { tool_name, .. }) = update {
            assert_eq!(tool_name, "codebase-retrieval");
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_convert_turn_finished_update() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "sessionId": "sess-1",
                "update": {
                    "sessionUpdate": "agent_turn_finished"
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Completion { .. })));
    }

    #[test]
    fn test_convert_session_finished_update() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "sessionId": "sess-1",
                "update": {
                    "sessionUpdate": "session_finished"
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Completion { .. })));
    }

    #[test]
    fn test_convert_unknown_update_type_returns_raw() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "sessionId": "sess-1",
                "update": {
                    "sessionUpdate": "some_unknown_type"
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Raw { .. })));
    }

    #[test]
    fn test_convert_non_session_update_returns_raw() {
        let notification = serde_json::json!({
            "method": "some/other/method",
            "params": {}
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Raw { .. })));
    }

    #[test]
    fn test_convert_uses_default_session_id() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "Hello"}
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default-sess");
        if let Some(SessionUpdate::Text { session_id, .. }) = update {
            assert_eq!(session_id, "default-sess");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_convert_thought_chunk_to_text() {
        let notification = serde_json::json!({
            "method": "session/update",
            "params": {
                "sessionId": "sess-1",
                "update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"text": "Thinking..."}
                }
            }
        });

        let update = MockTransport::convert_to_session_update(&notification, "default");
        assert!(update.is_some());

        if let Some(SessionUpdate::Text { content, .. }) = update {
            assert_eq!(content, "Thinking...");
        } else {
            panic!("Expected Text update for thought chunk");
        }
    }

    // ========================================================================
    // Edge Cases
    // ========================================================================

    #[tokio::test]
    async fn test_mock_transport_multiple_sessions() {
        let scenario = MockScenario {
            planner_sessions: vec![MockSessionBuilder::new("plan-001").build()],
            orchestrator_sessions: vec![MockSessionBuilder::new("orch-001").build()],
            implementer_sessions: vec![
                MockSessionBuilder::new("impl-001").build(),
                MockSessionBuilder::new("impl-002").build(),
            ],
            ..Default::default()
        };

        let mut transport = MockTransport::from_scenario(&scenario);
        transport.initialize().await.unwrap();

        // Create planner session (model name contains "planner")
        let config = SessionConfig::new("planner-model", "/tmp").with_mode(SessionMode::Plan);
        let session = transport.create_session(config).await.unwrap();
        assert_eq!(session.session_id, "plan-001");
        assert_eq!(transport.sessions_created_count(), 1);

        // Create orchestrator session (model name must contain "orchestrat")
        let config = SessionConfig::new("orchestrator-model", "/tmp").with_mode(SessionMode::Agent);
        let session = transport.create_session(config).await.unwrap();
        assert_eq!(session.session_id, "orch-001");
        assert_eq!(transport.sessions_created_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_transport_is_exhausted() {
        let scenario = MockScenario {
            implementer_sessions: vec![MockSessionBuilder::new("impl-001").build()],
            ..Default::default()
        };

        let mut transport = MockTransport::from_scenario(&scenario);
        transport.initialize().await.unwrap();

        assert!(!transport.is_exhausted());

        let config = SessionConfig::new("model", "/tmp").with_mode(SessionMode::Agent);
        let _session = transport.create_session(config).await.unwrap();

        // After using the only session, it should be exhausted
        assert!(transport.is_exhausted());
    }
}
