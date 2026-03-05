//! Simplified ACP client for spawning and managing agents

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// ACP client for managing agent sessions
pub struct AcpClient {
    child: Child,
    tx: mpsc::Sender<String>,
    /// Channel for JSON-RPC responses (messages with an "id" field)
    response_rx: mpsc::Receiver<Value>,
    /// Channel for JSON-RPC notifications (messages without an "id" field)
    notification_rx: mpsc::Receiver<Value>,
    /// Handle to the stdin writer task
    stdin_task: Option<JoinHandle<()>>,
    /// Handle to the stdout reader task
    stdout_task: Option<JoinHandle<()>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionNewResponse {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

impl AcpClient {
    /// Spawn a new ACP agent process
    ///
    /// If `cache_dir` is provided, auggie will use that directory for its settings,
    /// allowing different agents to have different tool configurations.
    pub async fn spawn(cache_dir: Option<&str>) -> Result<Self> {
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
            .spawn()
            .context("Failed to spawn auggie")?;

        let stdin = child.stdin.take().context("Failed to get stdin")?;
        let stdout = child.stdout.take().context("Failed to get stdout")?;

        let (tx, mut rx_commands) = mpsc::channel::<String>(100);
        let (tx_responses, response_rx) = mpsc::channel::<Value>(100);
        let (tx_notifications, notification_rx) = mpsc::channel::<Value>(100);

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
                    if value.get("id").is_some() {
                        if tx_responses.send(value).await.is_err() {
                            break;
                        }
                    } else {
                        if tx_notifications.send(value).await.is_err() {
                            break;
                        }
                    }
                } else {
                    tracing::warn!("Failed to parse ACP response: {}", &line[..line.len().min(100)]);
                }
            }
        });

        Ok(Self {
            child,
            tx,
            response_rx,
            notification_rx,
            stdin_task: Some(stdin_task),
            stdout_task: Some(stdout_task),
        })
    }

    /// Send a JSON-RPC request and wait for response
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
            tracing::info!("📤 Creating new session: model={}, mcpServers={}",
                params.get("model").and_then(|v| v.as_str()).unwrap_or("unknown"),
                params.get("mcpServers").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)
            );
        } else {
            tracing::debug!("📤 ACP {}: id={}", method, id);
        }

        self.tx
            .send(request_str)
            .await
            .context("Failed to send request")?;

        // For session/prompt, we don't need to wait for a response since the actual
        // results come via notifications. Return immediately.
        if method == "session/prompt" {
            return Ok(json!({}));
        }

        // Wait for response with matching ID from the response channel.
        // Since we now route responses and notifications to separate channels,
        // we only receive responses here - notifications are preserved in their own channel.
        loop {
            let response = self
                .response_rx
                .recv()
                .await
                .context("Failed to receive response")?;

            if response.get("id").and_then(|v| v.as_str()) == Some(&id) {
                if method == "session/new" {
                    if let Some(result) = response.get("result") {
                        let session_id = result.get("sessionId").and_then(|v| v.as_str()).unwrap_or("unknown");
                        tracing::info!("📥 Session created: {}", session_id);
                    }
                }
                if let Some(error) = response.get("error") {
                    anyhow::bail!("ACP error: {}", error);
                }
                return response
                    .get("result")
                    .cloned()
                    .context("No result in response");
            }
            // Non-matching response ID - this shouldn't happen in normal operation
            // since each client makes sequential requests, but log it just in case
            tracing::warn!("⚠️  Received response with unexpected id: {:?}", response.get("id"));
        }
    }

    /// Initialize the ACP connection
    pub async fn initialize(&mut self) -> Result<()> {
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
    pub async fn session_new(
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
    pub async fn session_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()> {
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
    pub async fn recv(&mut self) -> Result<Value> {
        self.notification_rx.recv().await.context("Failed to receive notification")
    }

    /// Gracefully shutdown the ACP client.
    ///
    /// This closes stdin (signaling EOF to the child), waits for the child process
    /// to exit (with a timeout), and cleans up background tasks.
    pub async fn shutdown(&mut self) -> Result<()> {
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
        let wait_result = tokio::time::timeout(
            Duration::from_secs(5),
            self.child.wait()
        ).await;

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
