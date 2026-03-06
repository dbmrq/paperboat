//! Unix socket communication utilities for the MCP server.

use super::types::{ToolRequest, ToolResponse};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Connect to Unix socket with retry logic
pub async fn connect_with_retry(socket_path: &PathBuf) -> Result<UnixStream> {
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
pub async fn send_response(stdout: &mut tokio::io::Stdout, response: &Value) -> Result<()> {
    let resp_str = serde_json::to_string(response)?;
    eprintln!("📤 MCP server sending: {resp_str}");
    stdout.write_all(resp_str.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}

/// Send a tool request to the app and wait for the response.
///
/// This opens a new connection for each request to allow concurrent tool calls.
/// The app will process the request and send back a `ToolResponse`.
pub async fn send_request_and_wait(
    socket_path: &PathBuf,
    request: &ToolRequest,
) -> Result<ToolResponse> {
    // Connect to app socket
    let mut stream = connect_with_retry(socket_path).await?;

    // Send the request
    let request_json = serde_json::to_string(request)?;
    eprintln!("📨 Sending to app: {request_json}");
    stream.write_all(request_json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;

    // Wait for response
    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .context("Failed to read response from app")?;

    eprintln!("📥 Received from app: {}", response_line.trim());

    let response: ToolResponse =
        serde_json::from_str(&response_line).context("Failed to parse ToolResponse from app")?;

    Ok(response)
}
