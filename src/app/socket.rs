//! Unix socket handling for MCP server communication.

use super::types::{ToolMessage, ToolReceiver, ToolSender};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::mcp_server::{ToolRequest, ToolResponse};

/// Handle for a per-agent socket, including cleanup resources.
pub struct AgentSocketHandle {
    /// Path to the Unix socket
    pub socket_path: PathBuf,
    /// Receiver for tool messages from this agent
    pub tool_rx: ToolReceiver,
    /// Handle to the listener task (for cleanup)
    listener_task: JoinHandle<()>,
}

impl AgentSocketHandle {
    /// Clean up the socket, removing the file and aborting the listener task.
    pub fn cleanup(self) {
        // Abort the listener task
        self.listener_task.abort();

        // Remove the socket file
        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove agent socket file {:?}: {}", self.socket_path, e);
            }
        }
    }
}

impl Drop for AgentSocketHandle {
    fn drop(&mut self) {
        // Abort the listener task on drop
        self.listener_task.abort();

        // Try to remove the socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Set up a Unix socket for a single agent.
///
/// Creates a unique socket path for the agent and starts a listener task.
/// Returns an `AgentSocketHandle` that can be used to receive tool messages
/// and clean up resources when done.
///
/// # Arguments
///
/// * `agent_id` - A unique identifier for the agent (used in socket path)
pub async fn setup_agent_socket(agent_id: &str) -> Result<AgentSocketHandle> {
    // Use only first 8 chars of agent_id to keep socket path short
    // macOS has a ~104 byte limit on Unix socket paths (SUN_LEN)
    let short_id = &agent_id[..8.min(agent_id.len())];
    let socket_path = std::env::temp_dir().join(format!("vl-{short_id}.sock"));

    // Remove socket file if it exists
    if let Err(e) = std::fs::remove_file(&socket_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Failed to remove existing socket file {:?}: {}", socket_path, e);
        }
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("Failed to bind agent socket at {:?} (agent_id={})", socket_path, agent_id))?;

    tracing::debug!("Agent socket listening at: {:?} (agent_id={})", socket_path, agent_id);

    // Create channel for tool messages
    let (tool_tx, tool_rx) = mpsc::channel::<ToolMessage>(100);

    let listener_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tool_tx = tool_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_mcp_connection(stream, tool_tx).await {
                            tracing::error!("Agent MCP connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    // This error is expected when the listener is dropped during cleanup
                    tracing::debug!("Agent socket listener stopped: {}", e);
                    break;
                }
            }
        }
    });

    Ok(AgentSocketHandle {
        socket_path,
        tool_rx,
        listener_task,
    })
}

/// Set up a Unix socket for MCP server communication.
///
/// Returns the socket path and tool receiver channel.
pub async fn setup_socket() -> Result<(PathBuf, ToolReceiver, JoinHandle<()>)> {
    // Use shortened socket name to avoid exceeding macOS SUN_LEN limit (~104 bytes)
    let uuid = uuid::Uuid::new_v4();
    let short_id = &uuid.to_string()[..8];
    let socket_path = std::env::temp_dir().join(format!("vl-{short_id}.sock"));

    // Remove socket file if it exists
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).context("Failed to bind Unix socket")?;

    tracing::info!("Unix socket listening at: {:?}", socket_path);

    // Spawn task to accept connections and forward tool requests
    let (tool_tx, tool_rx) = mpsc::channel::<ToolMessage>(100);

    let listener_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tool_tx = tool_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_mcp_connection(stream, tool_tx).await {
                            tracing::error!("MCP connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    // This error is expected when the listener is dropped during shutdown
                    tracing::debug!("Socket listener stopped: {}", e);
                    break;
                }
            }
        }
    });

    Ok((socket_path, tool_rx, listener_task))
}

/// Clean up a socket file and abort the listener task.
pub fn cleanup_socket(socket_path: &PathBuf, listener_task: Option<JoinHandle<()>>) {
    // Abort the socket listener task
    if let Some(task) = listener_task {
        task.abort();
    }

    // Remove socket file
    if let Err(e) = std::fs::remove_file(socket_path) {
        tracing::warn!("Failed to remove socket file: {}", e);
    }
}

/// Handle an MCP connection from the MCP server.
///
/// Each connection represents a single tool call. The connection stays open
/// until the tool call completes, allowing the response to be sent back.
pub async fn handle_mcp_connection(stream: UnixStream, tool_tx: ToolSender) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read the tool request (one line of JSON)
    if reader.read_line(&mut line).await? == 0 {
        return Ok(()); // Connection closed
    }

    let request: ToolRequest =
        serde_json::from_str(&line).context("Failed to parse tool request")?;

    tracing::trace!("🔌 Socket received: {:?}", request.tool_call);

    // Create a oneshot channel for the response
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();

    // Send the request to the app for processing
    tool_tx
        .send(ToolMessage::Request {
            request,
            response_tx,
        })
        .await
        .context("Failed to send tool request to app")?;

    // Wait for the response
    let response: ToolResponse = response_rx
        .await
        .context("Failed to receive response from app")?;

    // Send the response back to the MCP server
    let response_json = serde_json::to_string(&response)?;
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    Ok(())
}
