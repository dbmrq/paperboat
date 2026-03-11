//! IPC error types.
//!
//! Provides structured error handling for IPC operations while maintaining
//! compatibility with `anyhow::Error` for the rest of the codebase.

use std::fmt;
use std::io;

/// Structured error type for IPC operations.
///
/// This enum provides semantic error variants for common IPC failure modes,
/// making it easier to handle specific errors programmatically while still
/// being convertible to `anyhow::Error` for propagation.
#[derive(Debug)]
pub enum IpcError {
    /// Failed to bind to the address (server-side).
    ///
    /// Common causes:
    /// - Address already in use
    /// - Permission denied
    /// - Invalid path (Unix) or pipe name (Windows)
    BindFailed {
        /// The address that failed to bind
        address: String,
        /// The underlying IO error
        source: io::Error,
    },

    /// Failed to connect to the address (client-side).
    ///
    /// Common causes:
    /// - Server not listening
    /// - Connection refused
    /// - Timeout
    ConnectionFailed {
        /// The address that failed to connect
        address: String,
        /// The underlying IO error
        source: io::Error,
    },

    /// Failed to accept a connection (server-side).
    ///
    /// This usually indicates the listener was closed or an OS-level error.
    AcceptFailed {
        /// The underlying IO error
        source: io::Error,
    },
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindFailed { address, source } => {
                write!(f, "Failed to bind IPC listener at '{address}': {source}")
            }
            Self::ConnectionFailed { address, source } => {
                write!(f, "Failed to connect to IPC endpoint '{address}': {source}")
            }
            Self::AcceptFailed { source } => {
                write!(f, "Failed to accept IPC connection: {source}")
            }
        }
    }
}

impl std::error::Error for IpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BindFailed { source, .. }
            | Self::ConnectionFailed { source, .. }
            | Self::AcceptFailed { source } => Some(source),
        }
    }
}

// Note: IpcError automatically converts to anyhow::Error via the blanket
// impl<E: std::error::Error + Send + Sync + 'static> From<E> for anyhow::Error.
// Use `?` operator or `anyhow::Error::from(ipc_error)` for conversion.
