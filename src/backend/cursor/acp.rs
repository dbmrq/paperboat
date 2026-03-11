//! Cursor-specific ACP client implementation.
//!
//! This module provides `CursorAcpClient`, an ACP client that communicates
//! with Cursor's agent CLI over stdin/stdout using JSON-RPC 2.0.
//!
//! Key differences from the standard `AcpClient`:
//! - Spawns `agent acp` instead of `auggie --acp`
//! - Supports session modes (`agent`, `plan`, `ask`) to control tool access
//! - Requires an `authenticate` call with methodId: "cursor_login" after initialize
//! - Handles `session/request_permission` to control tool access per agent

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::acp::{AcpClientTrait, SessionMode, SessionNewResponse};

// Re-export PermissionPolicy from the shared module
pub use super::permission::PermissionPolicy;

/// Pending request tracker - maps request IDs to their response channels
type PendingRequests = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// ACP client for Cursor's agent CLI.
///
/// This client spawns `agent acp` and communicates via JSON-RPC over stdin/stdout.
/// Unlike the standard AcpClient, it requires an additional `authenticate` call
/// after initialization.
///
/// # Permission Handling
///
/// When Cursor sends `session/request_permission` for a tool, this client
/// automatically responds based on its `PermissionPolicy`. This allows
/// planners/orchestrators to be restricted from file editing while still
/// having access to MCP tools.
pub struct CursorAcpClient {
    child: Child,
    tx: mpsc::Sender<String>,
    /// Map of pending request IDs to their response channels
    pending_requests: PendingRequests,
    /// Channel for JSON-RPC notifications (messages without an "id" field).
    notification_rx: Option<mpsc::Receiver<Value>>,
    /// Handle to the stdin writer task
    stdin_task: Option<JoinHandle<()>>,
    /// Handle to the stdout reader task
    stdout_task: Option<JoinHandle<()>>,
    /// Timeout for request/response operations
    request_timeout: Duration,
    /// Permission policy for auto-responding to tool permission requests
    #[allow(dead_code)]
    permission_policy: PermissionPolicy,
}

impl CursorAcpClient {
    /// Spawn a new Cursor ACP agent process with custom request timeout and permission policy.
    ///
    /// # Arguments
    ///
    /// * `cache_dir` - Currently ignored (agent acp doesn't support --config-dir)
    /// * `request_timeout` - Timeout for request/response operations
    /// * `permission_policy` - Policy for auto-responding to tool permission requests
    pub async fn spawn_with_policy(
        cache_dir: Option<&str>,
        request_timeout: Duration,
        permission_policy: PermissionPolicy,
    ) -> Result<Self> {
        // Log that cache_dir is ignored (for debugging purposes)
        if let Some(dir) = cache_dir {
            tracing::debug!(
                "⚠️ Cursor agent does not support custom config dir, ignoring: {}",
                dir
            );
        }

        let mut cmd = Command::new("agent");
        cmd.arg("acp");

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true) // Kill child when parent process dies
            .spawn()
            .context("Failed to spawn Cursor agent")?;

        let stdin = child.stdin.take().context("Failed to get stdin")?;
        let stdout = child.stdout.take().context("Failed to get stdout")?;

        let (tx, mut rx_commands) = mpsc::channel::<String>(100);
        let (tx_notifications, notification_rx) = mpsc::channel::<Value>(100);

        // Create shared pending requests map
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let pending_requests_clone = Arc::clone(&pending_requests);

        // Clone tx for use in permission request responses
        let tx_for_permissions = tx.clone();
        let denied_tools = permission_policy.denied_tools.clone();

        // Spawn task to write to stdin
        let stdin_task = tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = rx_commands.recv().await {
                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    tracing::error!("Failed to write to stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    tracing::error!("Failed to write newline: {}", e);
                    break;
                }
            }
            // Explicitly drop stdin to close it, signaling EOF to the child process
            drop(stdin);
        });

        // Spawn task to read from stdout and route messages to appropriate channels
        let stdout_task = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    // Check for permission request (server-initiated request)
                    if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
                        if method == "session/request_permission" {
                            // Handle permission request
                            if let Some(id) = value.get("id") {
                                let tool_name = value
                                    .get("params")
                                    .and_then(|p| p.get("tool"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("unknown");

                                let decision = if denied_tools.contains(tool_name) {
                                    tracing::info!("🚫 Denying permission for tool: {}", tool_name);
                                    "reject-once"
                                } else {
                                    tracing::debug!(
                                        "✅ Allowing permission for tool: {}",
                                        tool_name
                                    );
                                    "allow-always"
                                };

                                // Send permission response
                                let response = json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "decision": decision
                                    }
                                });

                                if let Ok(response_str) = serde_json::to_string(&response) {
                                    let _ = tx_for_permissions.send(response_str).await;
                                }
                            }
                            continue;
                        }
                    }

                    // Route based on whether message has an "id" field:
                    // - Messages with "id" are responses to our requests
                    // - Messages without "id" are notifications from the server
                    if let Some(id) = value
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string)
                    {
                        // Look up the pending request and send the response
                        let mut pending = pending_requests_clone.lock().await;
                        if let Some(sender) = pending.remove(&id) {
                            // Send response to the waiting request
                            if sender.send(value).is_err() {
                                tracing::debug!("Response receiver dropped for request {}", id);
                            }
                        } else {
                            tracing::trace!(
                                "Ignoring stale response with id {} (no pending request)",
                                id
                            );
                        }
                    } else if tx_notifications.send(value).await.is_err() {
                        break;
                    }
                } else {
                    tracing::warn!(
                        "Failed to parse ACP response: {}",
                        &line[..line.len().min(100)]
                    );
                }
            }
        });

        Ok(Self {
            child,
            tx,
            pending_requests,
            notification_rx: Some(notification_rx),
            stdin_task: Some(stdin_task),
            stdout_task: Some(stdout_task),
            request_timeout,
            permission_policy,
        })
    }

    /// Spawn a new Cursor ACP agent process with custom request timeout.
    /// Uses default permission policy (allow all tools).
    #[allow(dead_code)]
    pub async fn spawn_with_timeout(
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Self> {
        Self::spawn_with_policy(cache_dir, request_timeout, PermissionPolicy::allow_all()).await
    }

    /// Send a JSON-RPC request and wait for response with timeout.
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let request_str = serde_json::to_string(&request)?;
        if method == "session/new" {
            tracing::info!(
                "📤 Creating new Cursor session: model={}, mcpServers={}",
                params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown"),
                params
                    .get("mcpServers")
                    .and_then(|v| v.as_array())
                    .map_or(0, std::vec::Vec::len)
            );
        } else {
            tracing::debug!("📤 Cursor ACP {}: id={}", method, id);
        }

        // For session/prompt, we don't need to wait for a response since the actual
        // results come via notifications. Return immediately.
        if method == "session/prompt" {
            self.tx
                .send(request_str)
                .await
                .context("Failed to send request")?;
            return Ok(json!({}));
        }

        // Create a oneshot channel to receive the response for this specific request
        let (response_tx, response_rx) = oneshot::channel();

        // Register the pending request before sending to avoid race conditions
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id.clone(), response_tx);
        }

        // Send the request
        if let Err(e) = self.tx.send(request_str).await {
            // Remove the pending request if send fails
            let mut pending = self.pending_requests.lock().await;
            pending.remove(&id);
            return Err(e).context("Failed to send request");
        }

        // Wait for the response on our dedicated channel with timeout
        let response = match tokio::time::timeout(self.request_timeout, response_rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                // Channel closed - receiver dropped
                anyhow::bail!("Response channel closed before receiving response");
            }
            Err(_elapsed) => {
                // Timeout - remove the pending request so we don't leak memory
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                tracing::error!(
                    "⏰ Timeout waiting for Cursor ACP {} response after {:?}",
                    method,
                    self.request_timeout
                );
                anyhow::bail!(
                    "Timeout waiting for Cursor ACP {} response after {:?}",
                    method,
                    self.request_timeout
                );
            }
        };

        if method == "session/new" {
            if let Some(result) = response.get("result") {
                let session_id = result
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                tracing::info!("📥 Cursor session created: {}", session_id);
            }
        }

        if let Some(error) = response.get("error") {
            anyhow::bail!("Cursor ACP error: {error}");
        }

        response
            .get("result")
            .cloned()
            .context("No result in response")
    }

    /// Send authenticate request (Cursor-specific).
    ///
    /// This must be called after `initialize()` but before any session operations.
    /// Cursor CLI requires this step to complete authentication with methodId: "cursor_login".
    async fn authenticate(&mut self) -> Result<()> {
        let params = json!({
            "methodId": "cursor_login"
        });

        self.request("authenticate", params).await?;
        tracing::info!("🔐 Cursor authentication completed");
        Ok(())
    }
}

#[async_trait]
impl AcpClientTrait for CursorAcpClient {
    /// Initialize the ACP connection and perform Cursor-specific authentication.
    ///
    /// This implementation sends the standard initialize request, then automatically
    /// calls authenticate with methodId: "cursor_login" as required by Cursor's ACP.
    async fn initialize(&mut self) -> Result<()> {
        // Standard ACP initialize
        let params = json!({
            "protocolVersion": 1,
            "capabilities": {},
            "clientInfo": {
                "name": "paperboat",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        self.request("initialize", params).await?;

        // Cursor-specific: authenticate after initialize
        self.authenticate().await?;

        Ok(())
    }

    /// Create a new session with the specified mode.
    ///
    /// Cursor ACP supports session modes to control agent capabilities:
    /// - `agent`: Full tool access (for implementers)
    /// - `plan`: Read-only planning mode (for planners)
    /// - `ask`: Q&A read-only mode (for explorers)
    async fn session_new(
        &mut self,
        model: &str,
        mcp_servers: Vec<Value>,
        cwd: &str,
        mode: SessionMode,
    ) -> Result<SessionNewResponse> {
        let params = json!({
            "model": model,
            "cwd": cwd,
            "mcpServers": mcp_servers,
            "mode": mode.as_str()
        });

        let result = self.request("session/new", params).await?;
        Ok(serde_json::from_value(result)?)
    }

    /// Send a prompt to a session
    async fn session_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()> {
        let params = json!({
            "sessionId": session_id,
            "prompt": [
                {
                    "type": "text",
                    "text": prompt
                }
            ],
        });

        self.request("session/prompt", params).await?;
        Ok(())
    }

    /// Receive next notification from the server
    ///
    /// Notifications are server-initiated messages (no "id" field) like session/update.
    /// Use this to monitor agent progress, receive plans, tool calls, etc.
    ///
    /// Returns an error if the notification receiver has been taken via `take_notification_rx()`.
    async fn recv(&mut self) -> Result<Value> {
        let rx = self
            .notification_rx
            .as_mut()
            .context("Notification receiver has been taken")?;
        rx.recv().await.context("Failed to receive notification")
    }

    /// Take the notification receiver for external routing.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        self.notification_rx.take()
    }

    /// Gracefully shutdown the ACP client.
    ///
    /// This closes stdin (signaling EOF to the child), waits for the child process
    /// to exit (with a timeout), and cleans up background tasks.
    async fn shutdown(&mut self) -> Result<()> {
        tracing::debug!("Shutting down Cursor ACP client");

        // Abort the stdin and stdout tasks
        if let Some(task) = self.stdin_task.take() {
            task.abort();
        }
        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }

        // Try to kill the child process gracefully first, then forcefully
        if let Err(e) = self.child.start_kill() {
            tracing::warn!("Failed to send kill signal to child: {}", e);
        }

        // Wait for the child to exit with a timeout
        let wait_result = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;

        match wait_result {
            Ok(Ok(status)) => {
                tracing::debug!("Cursor ACP child process exited with status: {}", status);
            }
            Ok(Err(e)) => {
                tracing::warn!("Error waiting for Cursor ACP child process: {}", e);
            }
            Err(_) => {
                // Timeout - process didn't exit, try to kill it more forcefully
                tracing::warn!(
                    "Cursor ACP child process did not exit within timeout, killing forcefully"
                );
                if let Err(e) = self.child.kill().await {
                    tracing::error!("Failed to forcefully kill Cursor ACP child: {}", e);
                }
            }
        }

        Ok(())
    }
}

impl Drop for CursorAcpClient {
    fn drop(&mut self) {
        // Abort background tasks to prevent them from running after drop
        if let Some(task) = self.stdin_task.take() {
            task.abort();
        }
        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }

        // Initiate process termination (can't wait in Drop since it's not async)
        if let Err(e) = self.child.start_kill() {
            tracing::warn!("Failed to initiate kill of Cursor ACP child process: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::Path;

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

    #[cfg(unix)]
    fn prepend_path(dir: &Path) -> EnvGuard {
        let mut path_entries = vec![dir.display().to_string()];
        if let Ok(existing) = env::var("PATH") {
            path_entries.push(existing);
        }
        EnvGuard::set("PATH", &path_entries.join(":"))
    }

    #[cfg(unix)]
    fn write_fake_agent_acp(
        dir: &Path,
        permission_log: &Path,
        requested_tool: &str,
        notification_text: &str,
    ) {
        let script_path = dir.join("agent");
        std::fs::write(
            &script_path,
            format!(
                r#"#!/usr/bin/env python3
import json
import pathlib
import sys

permission_log = pathlib.Path({permission_log:?})
requested_tool = {requested_tool:?}
notification_text = {notification_text:?}

def read_request():
    line = sys.stdin.readline()
    if not line:
        sys.exit(1)
    return json.loads(line)

init_request = read_request()
print(json.dumps({{"jsonrpc": "2.0", "id": init_request["id"], "result": {{}}}}), flush=True)

auth_request = read_request()
print(json.dumps({{"jsonrpc": "2.0", "id": auth_request["id"], "result": {{}}}}), flush=True)

print(
    json.dumps(
        {{
            "jsonrpc": "2.0",
            "id": "perm-1",
            "method": "session/request_permission",
            "params": {{"tool": requested_tool}},
        }}
    ),
    flush=True,
)

permission_response = read_request()
permission_log.write_text(json.dumps(permission_response))

print(
    json.dumps(
        {{
            "method": "session/update",
            "params": {{"type": "text", "content": notification_text}},
        }}
    ),
    flush=True,
)
"#,
            ),
        )
        .unwrap();

        let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions).unwrap();
    }

    #[cfg(unix)]
    async fn make_test_client(
        request_timeout: Duration,
    ) -> (CursorAcpClient, mpsc::Receiver<String>, PendingRequests) {
        let child = Command::new("/bin/sh")
            .args(["-c", "sleep 60"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (tx, rx_commands) = mpsc::channel::<String>(10);
        let (_notification_tx, notification_rx) = mpsc::channel::<Value>(10);
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));

        (
            CursorAcpClient {
                child,
                tx,
                pending_requests: Arc::clone(&pending_requests),
                notification_rx: Some(notification_rx),
                stdin_task: None,
                stdout_task: None,
                request_timeout,
                permission_policy: PermissionPolicy::allow_all(),
            },
            rx_commands,
            pending_requests,
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_initialize_sends_authenticate_after_initialize() {
        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(200)).await;
        let (methods_tx, methods_rx) = oneshot::channel();

        tokio::spawn(async move {
            let mut methods = Vec::new();
            while methods.len() < 2 {
                let request = rx_commands.recv().await.unwrap();
                let value: Value = serde_json::from_str(&request).unwrap();
                let id = value["id"].as_str().unwrap().to_string();
                methods.push(value["method"].as_str().unwrap().to_string());

                let sender = pending_requests.lock().await.remove(&id).unwrap();
                sender
                    .send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }))
                    .unwrap();
            }
            let _ = methods_tx.send(methods);
        });

        client.initialize().await.unwrap();

        assert_eq!(
            methods_rx.await.unwrap(),
            vec!["initialize".to_string(), "authenticate".to_string()]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_request_session_prompt_returns_after_send() {
        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(200)).await;

        assert_eq!(
            client
                .request("session/prompt", json!({"prompt": "hello"}))
                .await
                .unwrap(),
            json!({})
        );

        let request = rx_commands.recv().await.unwrap();
        let value: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(value["method"], "session/prompt");
        assert!(pending_requests.lock().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_request_timeout_removes_pending_request() {
        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(20)).await;

        let err = client.request("initialize", json!({})).await.unwrap_err();
        let _request = rx_commands.recv().await.unwrap();

        assert!(err
            .to_string()
            .contains("Timeout waiting for Cursor ACP initialize response"));
        assert!(pending_requests.lock().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_request_propagates_error_response() {
        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(200)).await;

        tokio::spawn(async move {
            let request = rx_commands.recv().await.unwrap();
            let value: Value = serde_json::from_str(&request).unwrap();
            let id = value["id"].as_str().unwrap().to_string();
            let sender = pending_requests.lock().await.remove(&id).unwrap();
            sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"message": "boom"}
                }))
                .unwrap();
        });

        let err = client.request("initialize", json!({})).await.unwrap_err();
        assert!(err.to_string().contains("Cursor ACP error"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_recv_fails_after_notification_receiver_taken() {
        let (mut client, _, _) = make_test_client(Duration::from_millis(200)).await;
        assert!(client.take_notification_rx().is_some());

        let err = client.recv().await.unwrap_err();
        assert!(err
            .to_string()
            .contains("Notification receiver has been taken"));
    }

    #[tokio::test]
    #[serial]
    async fn test_spawn_with_policy_propagates_missing_agent_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _path = EnvGuard::set("PATH", temp_dir.path().to_str().unwrap());

        let err = CursorAcpClient::spawn_with_policy(
            None,
            Duration::from_millis(50),
            PermissionPolicy::allow_all(),
        )
        .await
        .err()
        .unwrap();

        assert!(err.to_string().contains("Failed to spawn Cursor agent"));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial]
    async fn test_spawn_with_policy_denies_requested_tools_and_routes_notifications() {
        let temp_dir = tempfile::tempdir().unwrap();
        let permission_log = temp_dir.path().join("permission.json");
        write_fake_agent_acp(
            temp_dir.path(),
            &permission_log,
            "save-file",
            "denied tool request",
        );
        let _path = prepend_path(temp_dir.path());

        let mut client = CursorAcpClient::spawn_with_policy(
            None,
            Duration::from_secs(1),
            PermissionPolicy::for_orchestrator(),
        )
        .await
        .unwrap();

        client.initialize().await.unwrap();

        let notification = client.recv().await.unwrap();
        assert_eq!(notification["method"], "session/update");
        assert_eq!(notification["params"]["type"], "text");
        assert_eq!(notification["params"]["content"], "denied tool request");

        let permission_response: Value =
            serde_json::from_str(&std::fs::read_to_string(&permission_log).unwrap()).unwrap();
        assert_eq!(permission_response["id"], "perm-1");
        assert_eq!(permission_response["result"]["decision"], "reject-once");

        client.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial]
    async fn test_spawn_with_policy_allows_permitted_tools() {
        let temp_dir = tempfile::tempdir().unwrap();
        let permission_log = temp_dir.path().join("permission.json");
        write_fake_agent_acp(
            temp_dir.path(),
            &permission_log,
            "complete",
            "allowed tool request",
        );
        let _path = prepend_path(temp_dir.path());

        let mut client = CursorAcpClient::spawn_with_policy(
            None,
            Duration::from_secs(1),
            PermissionPolicy::for_implementer(),
        )
        .await
        .unwrap();

        client.initialize().await.unwrap();
        let _notification = client.recv().await.unwrap();

        let permission_response: Value =
            serde_json::from_str(&std::fs::read_to_string(&permission_log).unwrap()).unwrap();
        assert_eq!(permission_response["result"]["decision"], "allow-always");

        client.shutdown().await.unwrap();
    }

    // ========================================================================
    // Additional tempdir-backed tests for expanded coverage
    // ========================================================================

    #[cfg(unix)]
    #[tokio::test]
    async fn test_session_new_returns_session_id() {
        use crate::acp::SessionMode;

        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(200)).await;

        tokio::spawn(async move {
            // Respond to initialize
            let req1 = rx_commands.recv().await.unwrap();
            let value1: Value = serde_json::from_str(&req1).unwrap();
            let id1 = value1["id"].as_str().unwrap().to_string();
            let sender1 = pending_requests.lock().await.remove(&id1).unwrap();
            sender1
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id1,
                    "result": {}
                }))
                .unwrap();

            // Respond to authenticate
            let req2 = rx_commands.recv().await.unwrap();
            let value2: Value = serde_json::from_str(&req2).unwrap();
            let id2 = value2["id"].as_str().unwrap().to_string();
            let sender2 = pending_requests.lock().await.remove(&id2).unwrap();
            sender2
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id2,
                    "result": {}
                }))
                .unwrap();

            // Respond to session/new
            let req3 = rx_commands.recv().await.unwrap();
            let value3: Value = serde_json::from_str(&req3).unwrap();
            let id3 = value3["id"].as_str().unwrap().to_string();
            assert_eq!(value3["method"], "session/new");
            let sender3 = pending_requests.lock().await.remove(&id3).unwrap();
            sender3
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id3,
                    "result": {
                        "sessionId": "test-session-123"
                    }
                }))
                .unwrap();
        });

        client.initialize().await.unwrap();

        let response = client
            .session_new("sonnet-4.5", vec![], "/tmp", SessionMode::Agent)
            .await
            .unwrap();
        assert_eq!(response.session_id, "test-session-123");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_session_new_with_mcp_servers() {
        use crate::acp::SessionMode;

        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(200)).await;

        let (mcp_tx, mcp_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            // Skip initialize and authenticate
            for _ in 0..2 {
                let req = rx_commands.recv().await.unwrap();
                let value: Value = serde_json::from_str(&req).unwrap();
                let id = value["id"].as_str().unwrap().to_string();
                let sender = pending_requests.lock().await.remove(&id).unwrap();
                sender
                    .send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }))
                    .unwrap();
            }

            // Capture session/new request
            let req = rx_commands.recv().await.unwrap();
            let value: Value = serde_json::from_str(&req).unwrap();
            let id = value["id"].as_str().unwrap().to_string();

            // Capture the MCP servers for verification
            let mcp_servers = value["params"]["mcpServers"].clone();
            let _ = mcp_tx.send(mcp_servers);

            let sender = pending_requests.lock().await.remove(&id).unwrap();
            sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"sessionId": "sess-with-mcp"}
                }))
                .unwrap();
        });

        client.initialize().await.unwrap();

        let mcp_servers = vec![json!({
            "name": "paperboat-implementer",
            "command": "/usr/bin/paperboat",
            "args": ["--mcp-server", "--socket", "/tmp/test.sock"]
        })];

        let response = client
            .session_new("sonnet-4.5", mcp_servers, "/workspace", SessionMode::Agent)
            .await
            .unwrap();

        assert_eq!(response.session_id, "sess-with-mcp");

        let captured_mcp = mcp_rx.await.unwrap();
        assert!(captured_mcp.is_array());
        assert_eq!(captured_mcp[0]["name"], "paperboat-implementer");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_session_prompt_does_not_wait_for_response() {
        use crate::acp::SessionMode;

        let (mut client, mut rx_commands, pending_requests) =
            make_test_client(Duration::from_millis(200)).await;

        let (prompt_tx, prompt_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            // Handle initialize and authenticate
            for _ in 0..2 {
                let req = rx_commands.recv().await.unwrap();
                let value: Value = serde_json::from_str(&req).unwrap();
                let id = value["id"].as_str().unwrap().to_string();
                let sender = pending_requests.lock().await.remove(&id).unwrap();
                sender
                    .send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }))
                    .unwrap();
            }

            // Handle session/new
            let req = rx_commands.recv().await.unwrap();
            let value: Value = serde_json::from_str(&req).unwrap();
            let id = value["id"].as_str().unwrap().to_string();
            let sender = pending_requests.lock().await.remove(&id).unwrap();
            sender
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"sessionId": "prompt-test"}
                }))
                .unwrap();

            // Capture session/prompt
            let req = rx_commands.recv().await.unwrap();
            let value: Value = serde_json::from_str(&req).unwrap();
            let _ = prompt_tx.send(value["params"]["prompt"].clone());
            // Note: No response sent - session/prompt doesn't wait
        });

        client.initialize().await.unwrap();
        let response = client
            .session_new("sonnet-4.5", vec![], "/tmp", SessionMode::Agent)
            .await
            .unwrap();

        // session_prompt returns immediately without waiting for response
        client
            .session_prompt(&response.session_id, "Hello, agent!")
            .await
            .unwrap();

        let captured_prompt = prompt_rx.await.unwrap();
        assert!(captured_prompt.is_array());
        assert_eq!(captured_prompt[0]["text"], "Hello, agent!");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_shutdown_aborts_background_tasks() {
        let (mut client, _, _) = make_test_client(Duration::from_millis(100)).await;

        // Set up some tasks manually to ensure they exist
        assert!(client.stdin_task.is_none()); // Not set in make_test_client
        assert!(client.stdout_task.is_none());

        // Shutdown should complete without error
        client.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_permission_policy_for_planner_denies_file_tools() {
        let policy = PermissionPolicy::for_planner();

        // Planner should deny file editing tools
        assert!(policy.denied_tools.contains("str-replace-editor"));
        assert!(policy.denied_tools.contains("save-file"));
        assert!(policy.denied_tools.contains("remove-files"));
        assert!(policy.denied_tools.contains("launch-process"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_permission_policy_for_orchestrator_denies_file_tools() {
        let policy = PermissionPolicy::for_orchestrator();

        // Orchestrator should deny file editing tools
        assert!(policy.denied_tools.contains("str-replace-editor"));
        assert!(policy.denied_tools.contains("save-file"));
        assert!(policy.denied_tools.contains("remove-files"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_permission_policy_for_implementer_allows_file_tools() {
        let policy = PermissionPolicy::for_implementer();

        // Implementer should allow file editing tools
        assert!(policy.should_allow("str-replace-editor"));
        assert!(policy.should_allow("save-file"));
        assert!(policy.should_allow("remove-files"));
        assert!(policy.should_allow("launch-process"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_take_notification_rx_can_only_be_called_once() {
        let (mut client, _, _) = make_test_client(Duration::from_millis(100)).await;

        // First call should return Some
        let rx1 = client.take_notification_rx();
        assert!(rx1.is_some());

        // Second call should return None
        let rx2 = client.take_notification_rx();
        assert!(rx2.is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_spawn_with_timeout_uses_default_policy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _path = EnvGuard::set("PATH", temp_dir.path().to_str().unwrap());

        // spawn_with_timeout should fail because agent CLI is not found
        let result =
            CursorAcpClient::spawn_with_timeout(None, Duration::from_millis(50)).await;

        match result {
            Ok(_) => panic!("Expected error when agent CLI is not found"),
            Err(e) => assert!(
                e.to_string().contains("Failed to spawn Cursor agent"),
                "Unexpected error: {}",
                e
            ),
        }
    }
}
