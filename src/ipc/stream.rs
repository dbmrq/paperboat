//! IPC stream and listener types.
//!
//! This module provides the core abstractions for bidirectional IPC communication:
//! - `IpcListener`: Accepts incoming connections (server-side)
//! - `IpcStream`: A bidirectional async stream for reading/writing
//!
//! Both types wrap platform-specific implementations and expose a unified API.

use super::address::IpcAddress;
use super::error::IpcError;
use anyhow::Result;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

// ============================================================================
// IpcStream - Bidirectional async stream
// ============================================================================

/// A bidirectional async stream for IPC communication.
///
/// This type wraps platform-specific stream implementations:
/// - **Unix**: `tokio::net::UnixStream`
/// - **Windows**: `tokio::net::windows::named_pipe::NamedPipeClient/Server`
///
/// It implements `AsyncRead` and `AsyncWrite`, making it compatible with
/// tokio's async I/O ecosystem (e.g., `BufReader`, `BufWriter`).
///
/// # Protocol
///
/// The stream is used for newline-delimited JSON messages:
/// 1. Read a line (JSON request)
/// 2. Parse and process the request
/// 3. Write a line (JSON response)
///
/// # Example
///
/// ```ignore
/// use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
/// use paperboat::ipc::IpcStream;
///
/// async fn handle_connection(stream: IpcStream) -> Result<()> {
///     let (reader, mut writer) = stream.into_split();
///     let mut reader = BufReader::new(reader);
///     
///     let mut line = String::new();
///     reader.read_line(&mut line).await?;
///     
///     let response = process_request(&line)?;
///     writer.write_all(response.as_bytes()).await?;
///     writer.write_all(b"\n").await?;
///     writer.flush().await?;
///     
///     Ok(())
/// }
/// ```
pub struct IpcStream {
    #[cfg(unix)]
    pub(crate) inner: tokio::net::UnixStream,

    #[cfg(windows)]
    pub(crate) inner: WindowsStream,
}

/// Windows stream type - either client or server half of a named pipe.
#[cfg(windows)]
pub enum WindowsStream {
    Client(tokio::net::windows::named_pipe::NamedPipeClient),
    Server(tokio::net::windows::named_pipe::NamedPipeServer),
}

impl IpcStream {
    /// Connect to an IPC endpoint as a client.
    ///
    /// This is used by the MCP server to connect back to the main app.
    ///
    /// # Errors
    ///
    /// Returns `IpcError::ConnectionFailed` if the connection cannot be established.
    pub async fn connect(address: &IpcAddress) -> Result<Self, IpcError> {
        #[cfg(unix)]
        {
            tokio::net::UnixStream::connect(address.as_path())
                .await
                .map(|stream| Self { inner: stream })
                .map_err(|e| IpcError::ConnectionFailed {
                    address: address.to_string(),
                    source: e,
                })
        }

        #[cfg(windows)]
        {
            super::windows::connect(address).await
        }
    }

    /// Split the stream into read and write halves.
    ///
    /// This allows concurrent reading and writing on the same connection.
    /// Uses `tokio::io::split()` which works with any `AsyncRead + AsyncWrite`.
    pub fn into_split(self) -> (IpcReadHalf, IpcWriteHalf) {
        let (read, write) = tokio::io::split(self);
        (IpcReadHalf { inner: read }, IpcWriteHalf { inner: write })
    }

    /// Get the underlying Unix stream (Unix only).
    ///
    /// This is a temporary escape hatch for gradual migration.
    /// It will be removed once all callers use the abstraction.
    #[cfg(unix)]
    #[doc(hidden)]
    pub fn into_inner(self) -> tokio::net::UnixStream {
        self.inner
    }
}

// ============================================================================
// IpcReadHalf / IpcWriteHalf - Split halves of an IpcStream
// ============================================================================

/// The read half of an `IpcStream`, created by `IpcStream::into_split()`.
///
/// This type implements `AsyncRead` and can be used with buffered readers.
pub struct IpcReadHalf {
    inner: tokio::io::ReadHalf<IpcStream>,
}

/// The write half of an `IpcStream`, created by `IpcStream::into_split()`.
///
/// This type implements `AsyncWrite` and can be used with buffered writers.
pub struct IpcWriteHalf {
    inner: tokio::io::WriteHalf<IpcStream>,
}

impl AsyncRead for IpcReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for IpcWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// Implement AsyncRead for IpcStream
impl AsyncRead for IpcStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        #[cfg(unix)]
        {
            Pin::new(&mut self.inner).poll_read(cx, buf)
        }

        #[cfg(windows)]
        {
            match &mut self.inner {
                WindowsStream::Client(c) => Pin::new(c).poll_read(cx, buf),
                WindowsStream::Server(s) => Pin::new(s).poll_read(cx, buf),
            }
        }
    }
}

// Implement AsyncWrite for IpcStream
impl AsyncWrite for IpcStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        #[cfg(unix)]
        {
            Pin::new(&mut self.inner).poll_write(cx, buf)
        }

        #[cfg(windows)]
        {
            match &mut self.inner {
                WindowsStream::Client(c) => Pin::new(c).poll_write(cx, buf),
                WindowsStream::Server(s) => Pin::new(s).poll_write(cx, buf),
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        #[cfg(unix)]
        {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        #[cfg(windows)]
        {
            match &mut self.inner {
                WindowsStream::Client(c) => Pin::new(c).poll_flush(cx),
                WindowsStream::Server(s) => Pin::new(s).poll_flush(cx),
            }
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        #[cfg(unix)]
        {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }

        #[cfg(windows)]
        {
            match &mut self.inner {
                WindowsStream::Client(c) => Pin::new(c).poll_shutdown(cx),
                WindowsStream::Server(s) => Pin::new(s).poll_shutdown(cx),
            }
        }
    }
}

// ============================================================================
// IpcListener - Server-side connection acceptor
// ============================================================================

/// A listener that accepts incoming IPC connections.
///
/// This type wraps platform-specific listener implementations:
/// - **Unix**: `tokio::net::UnixListener`
/// - **Windows**: `tokio::net::windows::named_pipe::NamedPipeServer` (recreated per accept)
///
/// # Server Pattern
///
/// The listener follows the standard server pattern:
/// 1. Bind to an address
/// 2. Accept connections in a loop
/// 3. Spawn a task for each connection
///
/// # Example
///
/// ```ignore
/// use paperboat::ipc::{IpcAddress, IpcListener};
///
/// async fn run_server() -> Result<()> {
///     let addr = IpcAddress::generate("my-server");
///     let listener = IpcListener::bind(&addr).await?;
///
///     loop {
///         match listener.accept().await {
///             Ok(stream) => {
///                 tokio::spawn(handle_connection(stream));
///             }
///             Err(e) => {
///                 tracing::error!("Accept failed: {}", e);
///                 break;
///             }
///         }
///     }
///
///     Ok(())
/// }
/// ```
pub struct IpcListener {
    #[cfg(unix)]
    pub(crate) inner: tokio::net::UnixListener,

    #[cfg(windows)]
    pub(crate) inner: WindowsListener,
}

/// Windows listener state - manages named pipe server instances.
///
/// Uses interior mutability because accept() needs to create new pipe instances.
#[cfg(windows)]
pub struct WindowsListener {
    pub(crate) address: IpcAddress,
    pub(crate) current_server: std::sync::Mutex<Option<tokio::net::windows::named_pipe::NamedPipeServer>>,
}

impl IpcListener {
    /// Bind to an IPC address and start listening.
    ///
    /// On Unix, this creates a socket file at the specified path.
    /// On Windows, this creates a named pipe server.
    ///
    /// # Cleanup
    ///
    /// On Unix, any existing socket file at the path is removed before binding.
    /// This handles the case where a previous process crashed without cleanup.
    ///
    /// # Errors
    ///
    /// Returns `IpcError::BindFailed` if the listener cannot be created.
    pub async fn bind(address: &IpcAddress) -> Result<Self, IpcError> {
        #[cfg(unix)]
        {
            // Remove existing socket file (handles previous crash)
            address.cleanup();

            tokio::net::UnixListener::bind(address.as_path())
                .map(|listener| Self { inner: listener })
                .map_err(|e| IpcError::BindFailed {
                    address: address.to_string(),
                    source: e,
                })
        }

        #[cfg(windows)]
        {
            super::windows::bind(address).await
        }
    }

    /// Accept a new connection.
    ///
    /// This method waits for an incoming connection and returns the stream.
    /// The returned `IpcStream` can be used for bidirectional communication.
    ///
    /// # Errors
    ///
    /// Returns `IpcError::AcceptFailed` if accepting fails. This typically
    /// indicates the listener has been closed or an OS-level error occurred.
    pub async fn accept(&self) -> Result<IpcStream, IpcError> {
        #[cfg(unix)]
        {
            self.inner
                .accept()
                .await
                .map(|(stream, _addr)| IpcStream { inner: stream })
                .map_err(|e| IpcError::AcceptFailed { source: e })
        }

        #[cfg(windows)]
        {
            super::windows::accept(&self.inner).await
        }
    }

    /// Get the address this listener is bound to.
    #[allow(dead_code)]
    pub fn address(&self) -> Option<IpcAddress> {
        #[cfg(unix)]
        {
            // Unix sockets don't easily expose their path after binding
            None
        }

        #[cfg(windows)]
        {
            Some(self.inner.address.clone())
        }
    }
}

