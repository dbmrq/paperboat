//! Auggie ACP transport implementation.
//!
//! This module provides `AuggieAcpTransport`, a transport that wraps the
//! existing `AcpClient` and implements the `AgentTransport` trait.
//!
//! # Design
//!
//! This is an adapter that bridges the `AcpClientTrait` interface used by
//! the legacy code with the new `AgentTransport` interface used by the
//! transport abstraction layer.
//!
//! The wrapper delegates all operations to the underlying `AcpClient` and
//! converts ACP notifications to `SessionUpdate` events.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::acp::{AcpClient, AcpClientTrait};
use crate::backend::transport::{
    AgentTransport, SessionConfig, SessionInfo, SessionUpdate, ToolResult,
};
use serde_json::json;

/// Auggie ACP transport wrapping the existing `AcpClient`.
///
/// This transport provides the `AgentTransport` interface while delegating
/// actual ACP communication to the wrapped `AcpClient`.
///
/// # Lifecycle
///
/// 1. `initialize()` - Delegates to `AcpClient::initialize()`
/// 2. `create_session()` - Delegates to `AcpClient::session_new()`
/// 3. `send_prompt()` - Delegates to `AcpClient::session_prompt()`
/// 4. `take_notifications()` - Returns receiver with converted `SessionUpdate` events
/// 5. `shutdown()` - Delegates to `AcpClient::shutdown()`
pub struct AuggieAcpTransport {
    /// The wrapped ACP client
    client: AcpClient,
    /// Sender for converted session updates
    update_tx: mpsc::Sender<SessionUpdate>,
    /// Receiver for session updates (taken once by caller)
    #[allow(dead_code)]
    update_rx: Option<mpsc::Receiver<SessionUpdate>>,
    /// Background task converting notifications to session updates
    converter_task: Option<JoinHandle<()>>,
    /// Current session ID (set after `create_session`)
    current_session_id: Option<String>,
}

impl AuggieAcpTransport {
    /// Create a new Auggie ACP transport.
    ///
    /// # Arguments
    ///
    /// * `cache_dir` - Optional cache directory for tool filtering
    /// * `request_timeout` - Timeout for ACP requests
    ///
    /// # Errors
    ///
    /// Returns an error if the ACP client cannot be spawned.
    pub async fn new(cache_dir: Option<&str>, request_timeout: Duration) -> Result<Self> {
        let client = AcpClient::spawn_with_timeout(cache_dir, request_timeout).await?;
        let (tx, rx) = mpsc::channel(100);

        Ok(Self {
            client,
            update_tx: tx,
            update_rx: Some(rx),
            converter_task: None,
            current_session_id: None,
        })
    }

    /// Start the background task that converts ACP notifications to `SessionUpdate`s.
    fn start_notification_converter(&mut self) {
        // Take the notification receiver from the client
        let Some(mut notification_rx) = self.client.take_notification_rx() else {
            tracing::warn!("Notification receiver already taken");
            return;
        };

        let update_tx = self.update_tx.clone();
        let session_id = self.current_session_id.clone().unwrap_or_default();

        self.converter_task = Some(tokio::spawn(async move {
            while let Some(notification) = notification_rx.recv().await {
                if let Some(update) = convert_notification_to_update(&notification, &session_id) {
                    if update_tx.send(update).await.is_err() {
                        tracing::debug!("Update receiver dropped, stopping converter");
                        break;
                    }
                }
            }
            tracing::debug!("Notification converter task completed");
        }));
    }

    /// Convert a typed `SessionUpdate` into the legacy raw JSON format expected
    /// by the existing session router and handler code.
    fn session_update_to_json(update: SessionUpdate) -> Value {
        match update {
            SessionUpdate::Text {
                session_id,
                content,
            } => json!({
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {
                            "text": content
                        }
                    }
                }
            }),
            SessionUpdate::ToolUse {
                session_id,
                tool_use_id,
                tool_name,
                input,
            } => json!({
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "tool_call",
                        "title": tool_name,
                        "toolUseId": tool_use_id,
                        "input": input
                    }
                }
            }),
            SessionUpdate::ToolResult {
                session_id,
                tool_use_id,
                content,
                is_success,
            } => json!({
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "tool_result",
                        "toolUseId": tool_use_id,
                        "isSuccess": is_success,
                        "content": content
                    }
                }
            }),
            SessionUpdate::Completion {
                session_id,
                result,
                success,
            } => json!({
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "session_finished",
                        "result": result,
                        "success": success
                    }
                }
            }),
            SessionUpdate::Raw { data, .. } => data,
        }
    }
}

/// Convert an ACP notification to a `SessionUpdate`.
///
/// ACP notifications have the format:
/// ```json
/// {
///   "method": "session/update",
///   "params": {
///     "sessionId": "...",
///     "type": "text|tool_use|tool_result|completion|end",
///     ...
///   }
/// }
/// ```
fn convert_notification_to_update(
    notification: &Value,
    default_session_id: &str,
) -> Option<SessionUpdate> {
    let method = notification.get("method")?.as_str()?;
    if method != "session/update" {
        // Non-update notifications are returned as raw
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
    let update_type = params.get("type")?.as_str()?;

    match update_type {
        "text" => {
            let content = params.get("content")?.as_str()?.to_string();
            Some(SessionUpdate::Text {
                session_id,
                content,
            })
        }
        "tool_use" => {
            let tool_use_id = params.get("toolUseId")?.as_str()?.to_string();
            let tool_name = params.get("toolName")?.as_str()?.to_string();
            let input = params.get("input").cloned().unwrap_or(Value::Null);
            Some(SessionUpdate::ToolUse {
                session_id,
                tool_use_id,
                tool_name,
                input,
            })
        }
        "tool_result" => {
            let tool_use_id = params.get("toolUseId")?.as_str()?.to_string();
            let content = params.get("content")?.as_str()?.to_string();
            let is_success = params
                .get("isSuccess")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            Some(SessionUpdate::ToolResult {
                session_id,
                tool_use_id,
                content,
                is_success,
            })
        }
        "completion" | "end" => {
            let result = params
                .get("result")
                .and_then(|v| v.as_str())
                .map(String::from);
            let success = params
                .get("success")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            Some(SessionUpdate::Completion {
                session_id,
                result,
                success,
            })
        }
        _ => {
            // Unknown type - return as raw for forward compatibility
            Some(SessionUpdate::Raw {
                session_id: Some(session_id),
                data: notification.clone(),
            })
        }
    }
}

#[async_trait]
impl AgentTransport for AuggieAcpTransport {
    /// Initialize the transport connection.
    ///
    /// Delegates to `AcpClient::initialize()` to establish the ACP connection.
    async fn initialize(&mut self) -> Result<()> {
        self.client.initialize().await
    }

    /// Create a new agent session.
    ///
    /// Delegates to `AcpClient::session_new()` and starts the notification
    /// converter task.
    async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo> {
        let response = self
            .client
            .session_new(
                &config.model,
                config.mcp_servers,
                &config.workspace,
                config.mode,
            )
            .await?;

        self.current_session_id = Some(response.session_id.clone());

        // Start converting notifications to SessionUpdates
        self.start_notification_converter();

        Ok(SessionInfo::new(response.session_id))
    }

    /// Send a prompt to the agent session.
    ///
    /// Delegates to `AcpClient::session_prompt()`.
    async fn send_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()> {
        self.client.session_prompt(session_id, prompt).await
    }

    /// Take the notification receiver for streaming updates.
    ///
    /// Returns the receiver for `SessionUpdate` messages. This can only be
    /// called once; subsequent calls return `None`.
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>> {
        self.update_rx.take()
    }

    /// Respond to a tool call with the result.
    ///
    /// Note: The current `AcpClient` doesn't have a method for this, so
    /// for now this is a no-op. Tool results are typically handled
    /// internally by the ACP server.
    async fn respond_to_tool(
        &mut self,
        _session_id: &str,
        _tool_use_id: &str,
        _result: ToolResult,
    ) -> Result<()> {
        // ACP tool responses would be sent via a session/toolResult request
        // but the current AcpClient doesn't expose this. For now, this is a no-op.
        // MCP tools handled by paperboat are responded to via the MCP server.
        tracing::trace!("respond_to_tool called (no-op for Auggie ACP)");
        Ok(())
    }

    /// Gracefully shutdown the transport.
    ///
    /// Aborts the converter task and delegates to `AcpClient::shutdown()`.
    async fn shutdown(&mut self) -> Result<()> {
        // Abort the converter task
        if let Some(task) = self.converter_task.take() {
            task.abort();
        }

        self.client.shutdown().await
    }

    /// Receive the next raw notification (legacy compatibility).
    ///
    /// This delegates to `AcpClient::recv()` for backward compatibility
    /// with code that polls for notifications.
    async fn recv(&mut self) -> Result<Value> {
        self.client.recv().await
    }

    /// Take the raw notification receiver (legacy compatibility).
    ///
    /// This bridges the typed `SessionUpdate` channel into the legacy JSON
    /// notification format used by the existing session router. We must not
    /// delegate directly to the wrapped ACP client here because the converter
    /// task takes that receiver when a session starts.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        let typed_rx = self.update_rx.take()?;
        let (tx, rx) = mpsc::channel::<Value>(100);

        tokio::spawn(async move {
            let mut typed_rx = typed_rx;
            while let Some(update) = typed_rx.recv().await {
                let json = Self::session_update_to_json(update);
                if tx.send(json).await.is_err() {
                    break;
                }
            }
            tracing::debug!("Auggie ACP notification bridge task ended");
        });

        Some(rx)
    }
}

impl Drop for AuggieAcpTransport {
    fn drop(&mut self) {
        // Abort the converter task to prevent it from running after drop
        if let Some(task) = self.converter_task.take() {
            task.abort();
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_convert_text_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test-123",
                "type": "text",
                "content": "Hello, world!"
            }
        });

        let update = convert_notification_to_update(&notification, "default");
        assert!(update.is_some());
        if let Some(SessionUpdate::Text {
            session_id,
            content,
        }) = update
        {
            assert_eq!(session_id, "test-123");
            assert_eq!(content, "Hello, world!");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_convert_tool_use_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test-123",
                "type": "tool_use",
                "toolUseId": "call-456",
                "toolName": "complete",
                "input": {"success": true}
            }
        });

        let update = convert_notification_to_update(&notification, "default");
        assert!(update.is_some());
        if let Some(SessionUpdate::ToolUse {
            session_id,
            tool_use_id,
            tool_name,
            input,
        }) = update
        {
            assert_eq!(session_id, "test-123");
            assert_eq!(tool_use_id, "call-456");
            assert_eq!(tool_name, "complete");
            assert_eq!(input["success"], true);
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_session_update_to_json_uses_legacy_router_shape() {
        let bridged = AuggieAcpTransport::session_update_to_json(SessionUpdate::Text {
            session_id: "sess-1".to_string(),
            content: "hello".to_string(),
        });

        assert_eq!(bridged["method"], "session/update");
        assert_eq!(bridged["params"]["sessionId"], "sess-1");
        assert_eq!(
            bridged["params"]["update"]["sessionUpdate"],
            "agent_message_chunk"
        );
        assert_eq!(bridged["params"]["update"]["content"]["text"], "hello");
    }

    #[test]
    fn test_convert_completion_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test-123",
                "type": "completion",
                "result": "Task completed",
                "success": true
            }
        });

        let update = convert_notification_to_update(&notification, "default");
        assert!(update.is_some());
        if let Some(SessionUpdate::Completion {
            session_id,
            result,
            success,
        }) = update
        {
            assert_eq!(session_id, "test-123");
            assert_eq!(result, Some("Task completed".to_string()));
            assert!(success);
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_convert_end_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test-123",
                "type": "end"
            }
        });

        let update = convert_notification_to_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Completion { .. })));
    }

    #[test]
    fn test_convert_unknown_type_returns_raw() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test-123",
                "type": "unknown_future_type",
                "data": "something"
            }
        });

        let update = convert_notification_to_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Raw { .. })));
    }

    #[test]
    fn test_convert_non_update_notification_returns_raw() {
        let notification = json!({
            "method": "session/other",
            "params": {"data": "something"}
        });

        let update = convert_notification_to_update(&notification, "default");
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Raw { .. })));
    }

    #[test]
    fn test_convert_uses_default_session_id() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "text",
                "content": "Hello"
            }
        });

        let update = convert_notification_to_update(&notification, "default-session");
        assert!(update.is_some());
        if let Some(SessionUpdate::Text { session_id, .. }) = update {
            assert_eq!(session_id, "default-session");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_auggie_acp_transport_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AuggieAcpTransport>();
    }
}
