//! Unix socket handling for MCP server communication.

use super::types::{ToolMessage, ToolReceiver, ToolSender};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::mcp_server::{ToolRequest, ToolResponse};

/// Set up a Unix socket for MCP server communication.
///
/// Returns the socket path and tool receiver channel.
pub async fn setup_socket() -> Result<(PathBuf, ToolReceiver, JoinHandle<()>)> {
    let socket_path =
        std::env::temp_dir().join(format!("villalobos-{}.sock", uuid::Uuid::new_v4()));

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
