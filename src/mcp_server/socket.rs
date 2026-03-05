//! Unix socket communication utilities for the MCP server.

use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

/// Connect to Unix socket with retry logic
pub(crate) async fn connect_with_retry(socket_path: &PathBuf) -> Result<UnixStream> {
    let delays = [100, 500, 2000];
    let mut last_error = None;

    for (attempt, delay_ms) in delays.iter().enumerate() {
        match UnixStream::connect(socket_path).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                tracing::warn!(
                    "Connection attempt {} failed: {}. Retrying...",
                    attempt + 1,
                    e
                );
                last_error = Some(e);
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
        }
    }

    // Final attempt
    UnixStream::connect(socket_path)
        .await
        .map_err(|e| last_error.unwrap_or(e).into())
}

/// Send a response to stdout, handling errors gracefully
pub(crate) async fn send_response(stdout: &mut tokio::io::Stdout, response: &Value) -> Result<()> {
    let resp_str = serde_json::to_string(response)?;
    eprintln!("📤 MCP server sending: {}", resp_str);
    stdout.write_all(resp_str.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

/// Send data to socket, attempting reconnection on failure
pub(crate) async fn send_to_socket_with_reconnect(
    socket: &mut UnixStream,
    socket_path: &PathBuf,
    message: &str,
) -> Result<()> {
    // First attempt
    if send_to_socket(socket, message).await.is_ok() {
        return Ok(());
    }

    // Reconnect and retry
    tracing::warn!("Socket write failed, reconnecting...");
    *socket = UnixStream::connect(socket_path).await?;
    send_to_socket(socket, message).await
}

pub(crate) async fn send_to_socket(socket: &mut UnixStream, message: &str) -> Result<()> {
    socket.write_all(message.as_bytes()).await?;
    socket.write_all(b"\n").await?;
    socket.flush().await?;
    Ok(())
}

