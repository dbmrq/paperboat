//! IPC socket communication utilities for the MCP server.
//!
//! This module provides cross-platform IPC communication for MCP tool calls,
//! using the IPC abstraction layer for Unix sockets and Windows named pipes.

use super::types::{ToolRequest, ToolResponse};
use anyhow::{Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::ipc::{connect_with_retry, IpcAddress};

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
/// This opens a new IPC connection for each request to allow concurrent tool calls.
/// Uses Unix sockets on macOS/Linux and named pipes on Windows.
/// The app will process the request and send back a `ToolResponse`.
pub async fn send_request_and_wait(
    socket_address: &IpcAddress,
    request: &ToolRequest,
) -> Result<ToolResponse> {
    // Connect to app socket using the IPC abstraction layer
    let stream = connect_with_retry(socket_address).await?;
    let (reader, mut writer) = stream.into_split();

    // Send the request
    let request_json = serde_json::to_string(request)?;
    eprintln!("📨 Sending to app: {request_json}");
    writer.write_all(request_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    // Wait for response
    let mut reader = BufReader::new(reader);
    let mut response_line = String::new();
    let bytes_read = reader
        .read_line(&mut response_line)
        .await
        .context("Failed to read response from app")?;

    eprintln!(
        "📥 Received from app: {} bytes, content: '{}'",
        bytes_read,
        response_line.trim()
    );

    // Check if we got an empty response (socket closed before response)
    if bytes_read == 0 {
        return Err(anyhow::anyhow!(
            "Socket closed before receiving response - the app listener may have been dropped"
        ));
    }

    let response: ToolResponse = serde_json::from_str(&response_line).with_context(|| {
        format!(
            "Failed to parse ToolResponse from app. Received {} bytes: '{}'",
            bytes_read,
            response_line.trim()
        )
    })?;

    Ok(response)
}
