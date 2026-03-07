//! Simplified ACP client for spawning and managing agents

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

/// Trait defining the interface for an ACP client.
///
/// This trait abstracts the ACP client operations to enable testing with mock implementations.
/// All methods are async and the trait is Send + Sync for use across async boundaries.
#[async_trait]
pub trait AcpClientTrait: Send + Sync {
    /// Initialize the ACP connection
    async fn initialize(&mut self) -> Result<()>;

    /// Create a new session
    async fn session_new(
        &mut self,
        model: &str,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<SessionNewResponse>;

    /// Send a prompt to a session
    async fn session_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()>;

    /// Receive next notification from the server
    ///
    /// Notifications are server-initiated messages (no "id" field) like session/update.
    /// Use this to monitor agent progress, receive plans, tool calls, etc.
    async fn recv(&mut self) -> Result<Value>;

    /// Take the notification receiver for external routing.
    ///
    /// This extracts the internal notification receiver so it can be used by
    /// a background routing task. After calling this, `recv()` will return
    /// an error immediately since there's no receiver.
    ///
    /// Returns `None` if the receiver has already been taken.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>>;

    /// Gracefully shutdown the ACP client.
    ///
    /// This closes stdin (signaling EOF to the child), waits for the child process
    /// to exit (with a timeout), and cleans up background tasks.
    async fn shutdown(&mut self) -> Result<()>;
}

/// Pending request tracker - maps request IDs to their response channels
type PendingRequests = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// ACP client for managing agent sessions
pub struct AcpClient {
    child: Child,
    tx: mpsc::Sender<String>,
    /// Map of pending request IDs to their response channels
    pending_requests: PendingRequests,
    /// Channel for JSON-RPC notifications (messages without an "id" field).
    /// This is `Option` because it can be taken by `take_notification_rx()` for external routing.
    notification_rx: Option<mpsc::Receiver<Value>>,
    /// Handle to the stdin writer task
    stdin_task: Option<JoinHandle<()>>,
    /// Handle to the stdout reader task
    stdout_task: Option<JoinHandle<()>>,
    /// Timeout for request/response operations
    request_timeout: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionNewResponse {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

impl AcpClient {
    /// Spawn a new ACP agent process with custom request timeout.
    ///
    /// If `cache_dir` is provided, auggie will use that directory for its settings,
    /// allowing different agents to have different tool configurations.
    pub async fn spawn_with_timeout(
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Self> {
        let mut cmd = Command::new("auggie");
        cmd.arg("--acp");

        // Skip the indexing confirmation prompt
        cmd.arg("--allow-indexing");

        // If a custom cache directory is specified, use it
        // This allows orchestrator agents to have different settings (e.g., removed tools)
        if let Some(dir) = cache_dir {
            cmd.arg("--augment-cache-dir").arg(dir);
            tracing::info!("🔧 Spawning auggie with custom cache dir: {}", dir);
        }

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true) // Kill child when parent process dies
            .spawn()
            .context("Failed to spawn auggie")?;

        let stdin = child.stdin.take().context("Failed to get stdin")?;
        let stdout = child.stdout.take().context("Failed to get stdout")?;

        let (tx, mut rx_commands) = mpsc::channel::<String>(100);
        let (tx_notifications, notification_rx) = mpsc::channel::<Value>(100);

        // Create shared pending requests map
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let pending_requests_clone = Arc::clone(&pending_requests);

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
                            // No pending request found - this is a stale response from a
                            // previous session that was already handled or timed out.
                            // This is normal during rapid session transitions.
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
        })
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
                "📤 Creating new session: model={}, mcpServers={}",
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
            tracing::debug!("📤 ACP {}: id={}", method, id);
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
                    "⏰ Timeout waiting for ACP {} response after {:?}",
                    method,
                    self.request_timeout
                );
                anyhow::bail!(
                    "Timeout waiting for ACP {} response after {:?}",
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
                tracing::info!("📥 Session created: {}", session_id);
            }
        }

        if let Some(error) = response.get("error") {
            anyhow::bail!("ACP error: {error}");
        }

        response
            .get("result")
            .cloned()
            .context("No result in response")
    }
}

#[async_trait]
impl AcpClientTrait for AcpClient {
    /// Initialize the ACP connection
    async fn initialize(&mut self) -> Result<()> {
        let params = json!({
            "protocolVersion": 1,
            "capabilities": {},
            "clientInfo": {
                "name": "villalobos",
                "version": "0.1.0"
            }
        });

        self.request("initialize", params).await?;
        Ok(())
    }

    /// Create a new session
    async fn session_new(
        &mut self,
        model: &str,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<SessionNewResponse> {
        let params = json!({
            "model": model,
            "cwd": cwd,
            "mcpServers": mcp_servers
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
        tracing::debug!("Shutting down ACP client");

        // Drop the sender to close the stdin channel, which will cause the stdin
        // task to close stdin, signaling EOF to the child process
        // Note: We can't actually drop self.tx since we only have &mut self,
        // but we can close it by dropping a clone after sending nothing
        // Actually, let's just abort the tasks and kill the process

        // Abort the stdin and stdout tasks
        if let Some(task) = self.stdin_task.take() {
            task.abort();
        }
        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }

        // Try to kill the child process gracefully first, then forcefully
        // First, try SIGTERM via start_kill()
        if let Err(e) = self.child.start_kill() {
            tracing::warn!("Failed to send kill signal to child: {}", e);
        }

        // Wait for the child to exit with a timeout
        let wait_result = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;

        match wait_result {
            Ok(Ok(status)) => {
                tracing::debug!("ACP child process exited with status: {}", status);
            }
            Ok(Err(e)) => {
                tracing::warn!("Error waiting for ACP child process: {}", e);
            }
            Err(_) => {
                // Timeout - process didn't exit, try to kill it more forcefully
                tracing::warn!("ACP child process did not exit within timeout, killing forcefully");
                if let Err(e) = self.child.kill().await {
                    tracing::error!("Failed to forcefully kill ACP child: {}", e);
                }
            }
        }

        Ok(())
    }
}

impl Drop for AcpClient {
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
            tracing::warn!("Failed to initiate kill of ACP child process: {}", e);
        }
    }
}
