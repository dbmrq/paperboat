//! Windows-specific IPC implementation using named pipes.
//!
//! This module provides named pipe implementation details for Windows.
//! It is only compiled on Windows platforms.
//!
//! # Windows Named Pipes vs Unix Sockets
//!
//! Named pipes on Windows have different semantics than Unix sockets:
//!
//! - **Naming**: Pipes use `\\.\pipe\name` format instead of file paths
//! - **Creation**: Server creates pipe, client connects; no file system presence
//! - **Accept pattern**: Each accept requires creating a new pipe server instance
//! - **Cleanup**: Automatic when server handle is closed (no file to delete)
//!
//! # Implementation Notes
//!
//! The named pipe implementation requires careful handling:
//!
//! 1. **Server instance per connection**: Unlike Unix listeners that can accept
//!    multiple connections from one listener, Windows requires creating a new
//!    `NamedPipeServer` for each subsequent connection.
//!
//! 2. **First-instance flag**: The first server instance uses `first_pipe_instance(true)`
//!    which fails if the pipe already exists (preventing name collisions).
//!
//! 3. **Async connect**: Clients use `ClientOptions::open()` which may need retries
//!    if the server isn't ready yet.
//!
//! 4. **Server availability**: We create the next server instance BEFORE returning
//!    the connected client to ensure clients never get `NotFound` errors. This follows
//!    the pattern from tokio's official documentation.
//!
//! 5. **Remote client rejection**: By default, we reject remote clients for security
//!    (local IPC only).
//!
//! # Platform-Specific Limitations
//!
//! - **No existence check**: Unlike Unix sockets which are files, named pipes don't
//!   have a file system presence to check. `IpcAddress::exists()` returns `true` on
//!   Windows as a consequence. Connection attempts are the only way to verify a
//!   pipe server is running.
//!
//! - **No cleanup needed**: Named pipes are automatically cleaned up when all handles
//!   are closed. `IpcAddress::cleanup()` is a no-op on Windows.

#![cfg(windows)]

use super::address::IpcAddress;
use super::error::IpcError;
use super::stream::{IpcListener, IpcStream, WindowsListener, WindowsStream};
use std::io;
use std::time::Duration;
use tokio::net::windows::named_pipe::{ClientOptions, PipeMode, ServerOptions};

/// Connect to a named pipe server.
///
/// This function handles the Windows-specific connection logic.
/// It may return `WouldBlock` if the server has no available instances;
/// use `connect_with_retry` for automatic retry handling.
pub async fn connect(address: &IpcAddress) -> Result<IpcStream, IpcError> {
    let pipe_name = address.as_str();

    // Note: ClientOptions::open() is synchronous - it returns immediately
    // with success or error. We may need to handle ERROR_PIPE_BUSY by retrying.
    ClientOptions::new()
        .open(&*pipe_name)
        .map(|client| IpcStream {
            inner: WindowsStream::Client(client),
        })
        .map_err(|e| IpcError::ConnectionFailed {
            address: address.to_string(),
            source: e,
        })
}

/// Connect to a named pipe with retry logic.
///
/// Uses exponential backoff to handle timing issues when the server
/// might not be ready yet, or when all server instances are busy
/// (ERROR_PIPE_BUSY).
pub async fn connect_with_retry(address: &IpcAddress) -> anyhow::Result<IpcStream> {
    use anyhow::Context;

    let delays = [50, 100, 200, 500, 1000, 2000];
    let mut last_error = None;

    for (attempt, delay_ms) in delays.iter().enumerate() {
        match connect(address).await {
            Ok(stream) => {
                if attempt > 0 {
                    let attempt_num = attempt + 1;
                    eprintln!("✅ Pipe connection succeeded on attempt {attempt_num}");
                }
                return Ok(stream);
            }
            Err(e) => {
                let attempt_num = attempt + 1;
                eprintln!(
                    "⚠️  Pipe connection attempt {attempt_num} failed: {e}. Retrying in {delay_ms}ms..."
                );
                last_error = Some(e);
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
        }
    }

    eprintln!("🔌 Final pipe connection attempt to {address}");
    connect(address).await.with_context(|| {
        format!(
            "Failed to connect to pipe at {} after {} retries. Last error: {:?}",
            address,
            delays.len(),
            last_error
        )
    })
}

/// Create a named pipe server (listener).
///
/// The server is configured with:
/// - `first_pipe_instance(true)` to ensure we're the only server with this name
/// - `reject_remote_clients(true)` for security (local IPC only)
/// - `pipe_mode(Byte)` for stream-oriented communication (matching Unix sockets)
pub async fn bind(address: &IpcAddress) -> Result<IpcListener, IpcError> {
    let pipe_name = address.as_str();

    let server = ServerOptions::new()
        .first_pipe_instance(true) // Fail if pipe already exists
        .reject_remote_clients(true) // Security: only local connections
        .pipe_mode(PipeMode::Byte) // Stream mode, like Unix sockets
        .create(&*pipe_name)
        .map_err(|e| IpcError::BindFailed {
            address: address.to_string(),
            source: e,
        })?;

    Ok(IpcListener {
        inner: WindowsListener {
            address: address.clone(),
            current_server: std::sync::Mutex::new(Some(server)),
        },
    })
}

/// Accept a connection on a named pipe server.
///
/// Windows named pipes require creating a new server instance for each
/// subsequent connection, so this function:
/// 1. Waits for a client to connect to the current server
/// 2. Creates a new server instance for the next connection BEFORE returning
/// 3. Returns the connected stream
///
/// The order is important: we must create the next server instance before
/// returning, so clients never encounter `NotFound` errors when connecting.
pub async fn accept(listener: &WindowsListener) -> Result<IpcStream, IpcError> {
    // Take the current server (we'll replace it)
    let server = {
        let mut guard = listener.current_server.lock().unwrap();
        guard.take().expect("WindowsListener should always have a server")
    };

    // Wait for client to connect
    // This is cancellation-safe according to tokio docs
    server.connect().await.map_err(|e| IpcError::AcceptFailed { source: e })?;

    // Create a new server instance for the next connection BEFORE returning.
    // This ensures that there's always a server instance available for new
    // clients to connect to, preventing sporadic `NotFound` errors.
    let pipe_name = listener.address.as_str();
    let new_server = ServerOptions::new()
        .reject_remote_clients(true) // Match bind() settings
        .pipe_mode(PipeMode::Byte) // Match bind() settings
        .create(&*pipe_name)
        .map_err(|e| IpcError::BindFailed {
            address: listener.address.to_string(),
            source: e,
        })?;

    // Store the new server for the next accept() call
    {
        let mut guard = listener.current_server.lock().unwrap();
        *guard = Some(new_server);
    }

    // Return the connected stream
    Ok(IpcStream {
        inner: WindowsStream::Server(server),
    })
}

/// Check if a Windows error indicates the pipe is busy (all instances in use).
///
/// This is useful for implementing retry logic when connecting.
#[allow(dead_code)]
pub fn is_pipe_busy_error(error: &io::Error) -> bool {
    // ERROR_PIPE_BUSY = 231
    const ERROR_PIPE_BUSY: i32 = 231;
    error.raw_os_error() == Some(ERROR_PIPE_BUSY)
}

#[cfg(test)]
mod tests {
    // Note: These tests can only be run on Windows.
    // The module is conditionally compiled with #![cfg(windows)], so these tests
    // will only be compiled and run when targeting Windows.
    //
    // To run on Windows:
    //   cargo test --target x86_64-pc-windows-msvc
    //
    // Or if cross-compiling from Unix, tests would need to be run in a Windows
    // environment (e.g., CI with Windows runner, or Windows VM).

    use super::*;
    use crate::ipc::IpcListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_windows_ipc_round_trip() {
        // Create a unique address
        let addr = IpcAddress::generate("win-test");

        // Set up listener
        let listener = IpcListener::bind(&addr).await.unwrap();

        // Spawn server task
        let server = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];

            // Read using AsyncRead trait
            let n = stream.read(&mut buf).await.unwrap();
            let line = String::from_utf8_lossy(&buf[..n]);
            assert_eq!(line.trim(), "hello");

            // Write response
            stream.write_all(b"world\n").await.unwrap();
            stream.flush().await.unwrap();
        });

        // Give server time to start
        tokio::task::yield_now().await;

        // Connect as client
        let mut stream = connect(&addr).await.unwrap();

        // Send request
        stream.write_all(b"hello\n").await.unwrap();
        stream.flush().await.unwrap();

        // Read response
        let mut buf = vec![0u8; 1024];
        let n = stream.read(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf[..n]);
        assert_eq!(response.trim(), "world");

        // Wait for server
        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_windows_multiple_connections() {
        // Test that we can accept multiple sequential connections
        let addr = IpcAddress::generate("win-multi");

        let listener = IpcListener::bind(&addr).await.unwrap();

        let server = tokio::spawn(async move {
            for i in 0..3 {
                let mut stream = listener.accept().await.unwrap();

                let mut buf = [0u8; 10];
                let n = stream.read(&mut buf).await.unwrap();
                // Note: buf contains the message but we don't need to check it
                let _ = n;

                stream.write_all(format!("ack-{}\n", i).as_bytes()).await.unwrap();
                stream.flush().await.unwrap();
            }
        });

        tokio::task::yield_now().await;

        for i in 0..3 {
            let mut stream = connect(&addr).await.unwrap();

            stream.write_all(format!("msg-{}\n", i).as_bytes()).await.unwrap();
            stream.flush().await.unwrap();

            let mut buf = [0u8; 20];
            let n = stream.read(&mut buf).await.unwrap();
            let response = String::from_utf8_lossy(&buf[..n]);
            assert!(response.contains(&format!("ack-{}", i)));
        }

        server.await.unwrap();
    }
}

