//! Unix-specific IPC implementation.
//!
//! This module provides Unix socket implementation details.
//! It is only compiled on Unix platforms (macOS, Linux, etc.).

// The Unix implementation is straightforward since tokio::net::UnixStream
// and tokio::net::UnixListener directly implement AsyncRead/AsyncWrite.
//
// Most of the implementation is in stream.rs using cfg(unix) blocks.
// This module exists for any Unix-specific utilities if needed.

use super::address::IpcAddress;
use super::stream::IpcStream;
use anyhow::{Context, Result};
use std::time::Duration;

/// Connect to an IPC endpoint with retry logic.
///
/// Uses exponential backoff to handle timing issues when the
/// main app's IPC listener might not be ready yet.
///
/// This wraps `IpcStream::connect` with retry behavior.
///
/// # Arguments
///
/// * `address` - The IPC address to connect to
///
/// # Returns
///
/// An `IpcStream` connected to the address, or an error after all retries fail.
pub async fn connect_with_retry(address: &IpcAddress) -> Result<IpcStream> {
    // Delays in ms: 50, 100, 200, 500, 1000, 2000 (total ~4s max wait)
    let delays = [50, 100, 200, 500, 1000, 2000];
    let mut last_error = None;

    // First, verify the socket file exists
    if !address.exists() {
        eprintln!("⚠️  Socket file does not exist yet: {address}");
    }

    for (attempt, delay_ms) in delays.iter().enumerate() {
        match IpcStream::connect(address).await {
            Ok(stream) => {
                if attempt > 0 {
                    let attempt_num = attempt + 1;
                    eprintln!("✅ Socket connection succeeded on attempt {attempt_num}");
                }
                return Ok(stream);
            }
            Err(e) => {
                let attempt_num = attempt + 1;
                eprintln!(
                    "⚠️  Socket connection attempt {attempt_num} failed: {e}. Retrying in {delay_ms}ms..."
                );
                last_error = Some(e);
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
        }
    }

    // Final attempt with detailed error message
    eprintln!("🔌 Final socket connection attempt to {address}");
    IpcStream::connect(address).await.with_context(|| {
        format!(
            "Failed to connect to app socket at {} after {} retries. Last error: {:?}",
            address,
            delays.len(),
            last_error
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::IpcListener;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn test_ipc_round_trip() {
        // Create a unique address
        let addr = IpcAddress::generate("test");

        // Set up listener
        let listener = IpcListener::bind(&addr).await.unwrap();

        // Spawn server task
        let server_addr = addr.clone();
        let server = tokio::spawn(async move {
            let stream = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert_eq!(line.trim(), "hello");

            writer.write_all(b"world\n").await.unwrap();
            writer.flush().await.unwrap();
        });

        // Give server time to start
        tokio::task::yield_now().await;

        // Connect as client
        let stream = IpcStream::connect(&addr).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send request
        writer.write_all(b"hello\n").await.unwrap();
        writer.flush().await.unwrap();

        // Read response
        let mut response = String::new();
        reader.read_line(&mut response).await.unwrap();
        assert_eq!(response.trim(), "world");

        // Wait for server
        server.await.unwrap();

        // Cleanup
        addr.cleanup();
    }
}
