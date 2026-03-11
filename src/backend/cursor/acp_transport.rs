//! Cursor ACP transport wrapper.
//!
//! This module provides `CursorAcpTransport`, an adapter that wraps the existing
//! `CursorAcpClient` to implement the `AgentTransport` trait. This allows the
//! ACP protocol to be used through the unified transport interface.
//!
//! # Note on MCP Support
//!
//! **Warning:** Cursor's ACP mode has broken MCP tool support. Use `CursorCliTransport`
//! (via `TransportKind::Cli`) for proper MCP server integration. This transport is
//! provided for compatibility and for when Cursor fixes the ACP MCP issues.
//!
//! # Usage
//!
//! Typically created via `CursorBackend::create_transport()`:
//!
//! ```ignore
//! let backend = CursorBackend::new();
//! let config = TransportConfig::new("/workspace").with_model("sonnet-4.6");
//! let transport = backend.create_transport(
//!     TransportKind::Acp,
//!     AgentType::Implementer,
//!     config,
//! ).await?;
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use super::acp::CursorAcpClient;
use super::permission::PermissionPolicy;
use crate::acp::AcpClientTrait;
use crate::backend::transport::{
    AgentTransport, SessionConfig, SessionInfo, SessionUpdate, ToolResult,
};

/// Cursor ACP transport implementing the `AgentTransport` trait.
///
/// This is an adapter that wraps `CursorAcpClient` to provide a unified
/// transport interface. It converts between the ACP notification format
/// and `SessionUpdate` events.
///
/// # Permission Handling
///
/// The transport uses a `PermissionPolicy` to control which tools are
/// allowed for each agent type. This is applied during ACP permission
/// requests from the agent.
pub struct CursorAcpTransport {
    /// The wrapped ACP client (created on initialize)
    client: Option<CursorAcpClient>,
    /// Permission policy for tool access control
    permission_policy: PermissionPolicy,
    /// Request timeout for ACP operations
    request_timeout: Duration,
    /// Workspace path (stored for session creation)
    workspace: PathBuf,
    /// Model to use (stored for session creation)
    model: Option<String>,
    /// MCP servers to configure
    #[allow(dead_code)] // Reserved for MCP server configuration
    mcp_servers: Vec<Value>,
    /// Sender for converted session updates
    notification_tx: mpsc::Sender<SessionUpdate>,
    /// Receiver for converted session updates (taken once by caller)
    #[allow(dead_code)] // Taken by caller via take_notification_rx
    notification_rx: Option<mpsc::Receiver<SessionUpdate>>,
}

impl CursorAcpTransport {
    /// Create a new ACP transport with the given permission policy.
    ///
    /// The transport is not yet connected; call `initialize()` to spawn
    /// the ACP client.
    #[must_use]
    pub fn new(permission_policy: PermissionPolicy, request_timeout: Duration) -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            client: None,
            permission_policy,
            request_timeout,
            workspace: PathBuf::new(),
            model: None,
            mcp_servers: Vec::new(),
            notification_tx: tx,
            notification_rx: Some(rx),
        }
    }

    /// Create a transport for orchestrator agents.
    #[must_use]
    #[allow(dead_code)] // Factory method for orchestrator transport
    pub fn for_orchestrator(timeout: Duration) -> Self {
        Self::new(PermissionPolicy::for_orchestrator(), timeout)
    }

    /// Create a transport for planner agents.
    #[must_use]
    #[allow(dead_code)] // Factory method for planner transport
    pub fn for_planner(timeout: Duration) -> Self {
        Self::new(PermissionPolicy::for_planner(), timeout)
    }

    /// Create a transport for implementer agents.
    #[must_use]
    #[allow(dead_code)] // Factory method for implementer transport
    pub fn for_implementer(timeout: Duration) -> Self {
        Self::new(PermissionPolicy::for_implementer(), timeout)
    }

    /// Set the workspace path.
    pub fn set_workspace(&mut self, workspace: PathBuf) {
        self.workspace = workspace;
    }

    /// Set the model.
    pub fn set_model(&mut self, model: String) {
        self.model = Some(model);
    }

    /// Set MCP servers.
    pub fn set_mcp_servers(&mut self, servers: Vec<Value>) {
        self.mcp_servers = servers;
    }

    /// Convert an ACP notification to a SessionUpdate.
    fn convert_notification(notification: &Value, session_id: &str) -> Option<SessionUpdate> {
        // ACP notifications have method: "session/update" and params with the update data
        let params = notification.get("params")?;
        let update_type = params.get("type")?.as_str()?;

        match update_type {
            "text" => {
                let content = params.get("content")?.as_str()?;
                Some(SessionUpdate::Text {
                    session_id: session_id.to_string(),
                    content: content.to_string(),
                })
            }
            "tool_use" => {
                let id = params.get("id")?.as_str()?;
                let name = params.get("name")?.as_str()?;
                let input = params.get("input")?.clone();
                Some(SessionUpdate::ToolUse {
                    session_id: session_id.to_string(),
                    tool_use_id: id.to_string(),
                    tool_name: name.to_string(),
                    input,
                })
            }
            "tool_result" => {
                let tool_use_id = params.get("toolUseId")?.as_str()?;
                let content = params.get("content")?.as_str()?;
                let is_error = params
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(SessionUpdate::ToolResult {
                    session_id: session_id.to_string(),
                    tool_use_id: tool_use_id.to_string(),
                    content: content.to_string(),
                    is_success: !is_error,
                })
            }
            "end" | "completion" => {
                let result = params
                    .get("result")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let success = params
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                Some(SessionUpdate::Completion {
                    session_id: session_id.to_string(),
                    result,
                    success,
                })
            }
            _ => {
                // Unknown type - return as raw
                Some(SessionUpdate::Raw {
                    session_id: Some(session_id.to_string()),
                    data: params.clone(),
                })
            }
        }
    }
}

#[async_trait]
impl AgentTransport for CursorAcpTransport {
    /// Initialize the ACP connection.
    ///
    /// Spawns the Cursor ACP client and performs authentication.
    async fn initialize(&mut self) -> Result<()> {
        tracing::debug!("Initializing Cursor ACP transport");

        // Spawn the ACP client with the configured permission policy
        let mut client = CursorAcpClient::spawn_with_policy(
            None, // cache_dir is ignored by Cursor
            self.request_timeout,
            self.permission_policy.clone(),
        )
        .await?;

        // Initialize the ACP connection (includes Cursor-specific authentication)
        client.initialize().await?;

        self.client = Some(client);
        tracing::info!("✅ Cursor ACP transport initialized");
        Ok(())
    }

    /// Create a new agent session.
    ///
    /// Uses the stored workspace, model, and MCP server configuration.
    async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo> {
        let client = self.client.as_mut().ok_or_else(|| {
            anyhow::anyhow!("Transport not initialized. Call initialize() first.")
        })?;

        // Create session with the provided configuration
        let response = client
            .session_new(
                &config.model,
                config.mcp_servers,
                &config.workspace,
                config.mode,
            )
            .await?;

        let session_id = response.session_id;

        // Start background task to convert ACP notifications to SessionUpdates
        if let Some(mut acp_rx) = client.take_notification_rx() {
            let tx = self.notification_tx.clone();
            let session_id_clone = session_id.clone();

            tokio::spawn(async move {
                while let Some(notification) = acp_rx.recv().await {
                    // Convert ACP notification to SessionUpdate
                    if let Some(update) =
                        CursorAcpTransport::convert_notification(&notification, &session_id_clone)
                    {
                        if tx.send(update).await.is_err() {
                            tracing::debug!(
                                "Notification receiver dropped, stopping ACP notification reader"
                            );
                            break;
                        }
                    }
                }
            });
        }

        Ok(SessionInfo::new(session_id))
    }

    /// Send a prompt to the agent.
    async fn send_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()> {
        let client = self.client.as_mut().ok_or_else(|| {
            anyhow::anyhow!("Transport not initialized. Call initialize() first.")
        })?;

        client.session_prompt(session_id, prompt).await
    }

    /// Take the notification receiver for streaming updates.
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>> {
        self.notification_rx.take()
    }

    /// Respond to a tool call with the result.
    ///
    /// For ACP transport, this sends a tool response via the ACP protocol.
    /// Note: Currently uses recv() to handle tool responses inline.
    ///
    /// Tool responses are handled internally by the ACP client via its permission system.
    /// This method exists for trait compatibility but does not need to send responses
    /// externally since the ACP protocol manages tool execution permissions directly.
    async fn respond_to_tool(
        &mut self,
        _session_id: &str,
        _tool_use_id: &str,
        _result: ToolResult,
    ) -> Result<()> {
        // ACP client handles tool responses internally via its permission system.
        // This no-op implementation is intentional for trait compatibility.
        tracing::trace!("respond_to_tool called on ACP transport (handled internally)");
        Ok(())
    }

    /// Shutdown the transport.
    async fn shutdown(&mut self) -> Result<()> {
        tracing::debug!("Shutting down Cursor ACP transport");
        if let Some(mut client) = self.client.take() {
            client.shutdown().await?;
        }
        Ok(())
    }

    /// Receive the next raw notification (legacy compatibility).
    ///
    /// This delegates to `CursorAcpClient::recv()` for backward compatibility
    /// with code that polls for notifications.
    async fn recv(&mut self) -> Result<Value> {
        let client = self.client.as_mut().ok_or_else(|| {
            anyhow::anyhow!("Transport not initialized. Call initialize() first.")
        })?;
        client.recv().await
    }

    /// Take the raw notification receiver (legacy compatibility).
    ///
    /// This delegates to `CursorAcpClient::take_notification_rx()` for backward
    /// compatibility with the existing notification routing pattern.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        self.client.as_mut().and_then(|c| c.take_notification_rx())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // Notification Conversion Tests
    // ========================================================================

    #[test]
    fn test_convert_text_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "text",
                "content": "Hello, world!"
            }
        });

        let update =
            CursorAcpTransport::convert_notification(&notification, "session-123").unwrap();

        if let SessionUpdate::Text {
            session_id,
            content,
        } = update
        {
            assert_eq!(session_id, "session-123");
            assert_eq!(content, "Hello, world!");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_convert_text_notification_with_empty_content() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "text",
                "content": ""
            }
        });

        let update =
            CursorAcpTransport::convert_notification(&notification, "session-123").unwrap();

        if let SessionUpdate::Text { content, .. } = update {
            assert_eq!(content, "");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_convert_tool_use_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "tool_use",
                "id": "call-456",
                "name": "complete",
                "input": {"success": true}
            }
        });

        let update =
            CursorAcpTransport::convert_notification(&notification, "session-123").unwrap();

        if let SessionUpdate::ToolUse {
            session_id,
            tool_use_id,
            tool_name,
            input,
        } = update
        {
            assert_eq!(session_id, "session-123");
            assert_eq!(tool_use_id, "call-456");
            assert_eq!(tool_name, "complete");
            assert_eq!(input["success"], true);
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_convert_tool_use_with_complex_input() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "tool_use",
                "id": "call-789",
                "name": "spawn_agents",
                "input": {
                    "agents": [
                        {"role": "implementer", "task": "Write tests"}
                    ],
                    "wait": "all"
                }
            }
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1").unwrap();

        if let SessionUpdate::ToolUse {
            tool_name, input, ..
        } = update
        {
            assert_eq!(tool_name, "spawn_agents");
            assert_eq!(input["wait"], "all");
            assert!(input["agents"].is_array());
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_convert_tool_result_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "tool_result",
                "toolUseId": "call-456",
                "content": "File saved successfully",
                "isError": false
            }
        });

        let update =
            CursorAcpTransport::convert_notification(&notification, "session-123").unwrap();

        if let SessionUpdate::ToolResult {
            session_id,
            tool_use_id,
            content,
            is_success,
        } = update
        {
            assert_eq!(session_id, "session-123");
            assert_eq!(tool_use_id, "call-456");
            assert_eq!(content, "File saved successfully");
            assert!(is_success);
        } else {
            panic!("Expected ToolResult update");
        }
    }

    #[test]
    fn test_convert_tool_result_error() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "tool_result",
                "toolUseId": "call-err",
                "content": "Permission denied",
                "isError": true
            }
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1").unwrap();

        if let SessionUpdate::ToolResult {
            is_success,
            content,
            ..
        } = update
        {
            assert!(!is_success);
            assert_eq!(content, "Permission denied");
        } else {
            panic!("Expected ToolResult update");
        }
    }

    #[test]
    fn test_convert_completion_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "end",
                "result": "Task completed",
                "success": true
            }
        });

        let update =
            CursorAcpTransport::convert_notification(&notification, "session-123").unwrap();

        if let SessionUpdate::Completion {
            session_id,
            result,
            success,
        } = update
        {
            assert_eq!(session_id, "session-123");
            assert_eq!(result, Some("Task completed".to_string()));
            assert!(success);
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_convert_completion_notification_with_type_completion() {
        // Test that "completion" type is also handled
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "completion",
                "success": true
            }
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1").unwrap();

        assert!(matches!(update, SessionUpdate::Completion { .. }));
    }

    #[test]
    fn test_convert_completion_failure() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "end",
                "result": "Error occurred",
                "success": false
            }
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1").unwrap();

        if let SessionUpdate::Completion {
            success, result, ..
        } = update
        {
            assert!(!success);
            assert_eq!(result, Some("Error occurred".to_string()));
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_convert_completion_defaults_to_success() {
        // When success field is missing, it should default to true
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "end"
            }
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1").unwrap();

        if let SessionUpdate::Completion {
            success, result, ..
        } = update
        {
            assert!(success);
            assert!(result.is_none());
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_convert_unknown_notification() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "type": "unknown_type",
                "data": "something"
            }
        });

        let update =
            CursorAcpTransport::convert_notification(&notification, "session-123").unwrap();

        if let SessionUpdate::Raw { session_id, data } = update {
            assert_eq!(session_id, Some("session-123".to_string()));
            assert_eq!(data["type"], "unknown_type");
        } else {
            panic!("Expected Raw update");
        }
    }

    // ========================================================================
    // Edge Cases for Notification Conversion
    // ========================================================================

    #[test]
    fn test_convert_notification_wrong_method_returns_none() {
        let notification = json!({
            "method": "some/other/method",
            "params": {}
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1");
        assert!(update.is_none());
    }

    #[test]
    fn test_convert_notification_missing_params_returns_none() {
        let notification = json!({
            "method": "session/update"
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1");
        assert!(update.is_none());
    }

    #[test]
    fn test_convert_notification_missing_type_returns_none() {
        let notification = json!({
            "method": "session/update",
            "params": {
                "content": "No type field"
            }
        });

        let update = CursorAcpTransport::convert_notification(&notification, "sess-1");
        assert!(update.is_none());
    }

    // ========================================================================
    // Transport Construction Tests
    // ========================================================================

    #[test]
    fn test_acp_transport_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CursorAcpTransport>();
    }

    #[test]
    fn test_new_creates_channel() {
        let transport =
            CursorAcpTransport::new(PermissionPolicy::allow_all(), Duration::from_secs(60));
        assert!(transport.notification_rx.is_some());
        assert!(transport.client.is_none()); // Not initialized yet
    }

    #[test]
    fn test_factory_methods() {
        let timeout = Duration::from_secs(60);

        let orchestrator = CursorAcpTransport::for_orchestrator(timeout);
        assert!(orchestrator.notification_rx.is_some());

        let planner = CursorAcpTransport::for_planner(timeout);
        assert!(planner.notification_rx.is_some());

        let implementer = CursorAcpTransport::for_implementer(timeout);
        assert!(implementer.notification_rx.is_some());
    }

    #[test]
    fn test_take_notifications_returns_once() {
        let mut transport =
            CursorAcpTransport::new(PermissionPolicy::allow_all(), Duration::from_secs(60));

        // First call returns Some
        let rx1 = transport.take_notifications();
        assert!(rx1.is_some());

        // Second call returns None
        let rx2 = transport.take_notifications();
        assert!(rx2.is_none());
    }

    #[test]
    fn test_transport_stores_timeout() {
        let timeout = Duration::from_secs(120);
        let transport = CursorAcpTransport::new(PermissionPolicy::allow_all(), timeout);
        assert_eq!(transport.request_timeout, Duration::from_secs(120));
    }

    #[tokio::test]
    async fn test_shutdown_on_uninitialized_transport() {
        let mut transport =
            CursorAcpTransport::new(PermissionPolicy::allow_all(), Duration::from_secs(60));

        // Shutdown on uninitialized transport should succeed (no-op)
        let result = transport.shutdown().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_respond_to_tool_succeeds() {
        let mut transport =
            CursorAcpTransport::new(PermissionPolicy::allow_all(), Duration::from_secs(60));

        // respond_to_tool should succeed even without initialization
        // (it's a no-op for ACP transport)
        let result = transport
            .respond_to_tool("session-1", "tool-1", ToolResult::success("done"))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_prompt_without_init_fails() {
        let mut transport =
            CursorAcpTransport::new(PermissionPolicy::allow_all(), Duration::from_secs(60));

        // send_prompt without initialization should fail
        let result = transport.send_prompt("session-1", "test prompt").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not initialized"));
    }

    #[tokio::test]
    async fn test_create_session_without_init_fails() {
        let mut transport =
            CursorAcpTransport::new(PermissionPolicy::allow_all(), Duration::from_secs(60));

        // create_session without initialization should fail
        let config = SessionConfig::new("model", "/workspace");
        let result = transport.create_session(config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not initialized"));
    }
}
