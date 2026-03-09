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
