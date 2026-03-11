//! Platform-agnostic IPC address type.
//!
//! This module provides `IpcAddress`, a unified way to identify IPC endpoints
//! across different platforms.

use std::fmt;
use std::path::PathBuf;

/// Platform-agnostic IPC endpoint address.
///
/// Encapsulates the platform-specific addressing scheme:
/// - **Unix**: File system path (e.g., `/tmp/vl-abc12345.sock`)
/// - **Windows**: Named pipe path (e.g., `\\.\pipe\vl-abc12345`)
///
/// # Generation
///
/// Use `IpcAddress::generate()` to create a unique address with a prefix:
///
/// ```ignore
/// let addr = IpcAddress::generate("agent-abc123");
/// // Unix:   /tmp/vl-agent-ab.sock (truncated for SUN_LEN limit)
/// // Windows: \\.\pipe\vl-agent-abc123
/// ```
///
/// # String Representation
///
/// The address can be converted to a string for passing to child processes
/// via environment variables or command line arguments.
#[derive(Debug, Clone)]
pub struct IpcAddress {
    /// The underlying path/name representation.
    ///
    /// - Unix: `PathBuf` pointing to socket file
    /// - Windows: `PathBuf` containing the pipe name (\\.\pipe\...)
    inner: PathBuf,
}

impl IpcAddress {
    /// Generate a new unique IPC address with the given prefix.
    ///
    /// The prefix is used to identify the purpose (e.g., "agent-", "selfimprove-").
    /// A short UUID segment is appended to ensure uniqueness.
    ///
    /// # Platform Behavior
    ///
    /// - **Unix**: Creates a path in the temp directory. The prefix is truncated
    ///   to 8 characters to stay within macOS's ~104 byte socket path limit.
    /// - **Windows**: Creates a named pipe path. No truncation needed.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let addr = IpcAddress::generate("agent-abc12345");
    /// println!("{}", addr);  // /tmp/vl-agent-ab.sock (Unix)
    /// ```
    #[must_use]
    pub fn generate(prefix: &str) -> Self {
        let short_uuid = &uuid::Uuid::new_v4().to_string()[..8];

        #[cfg(unix)]
        {
            // Truncate prefix to 8 chars to avoid exceeding macOS SUN_LEN limit (~104 bytes)
            // Format: /tmp/vl-{prefix}-{uuid}.sock
            let short_prefix = &prefix[..8.min(prefix.len())];
            let path = std::env::temp_dir().join(format!("vl-{short_prefix}-{short_uuid}.sock"));
            Self { inner: path }
        }

        #[cfg(windows)]
        {
            // Windows named pipes don't have the same length restrictions
            // Format: \\.\pipe\vl-{prefix}-{uuid}
            let path = PathBuf::from(format!(r"\\.\pipe\vl-{prefix}-{short_uuid}"));
            Self { inner: path }
        }
    }

    /// Create an IPC address from a string representation.
    ///
    /// Used when receiving an address from a child process or command line.
    /// The string is interpreted according to the current platform.
    #[must_use]
    pub fn from_string(s: &str) -> Self {
        Self {
            inner: PathBuf::from(s),
        }
    }

    /// Get the address as a string for serialization.
    ///
    /// This is used to pass the address to child processes via environment
    /// variables or command line arguments.
    #[must_use]
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        self.inner.to_string_lossy()
    }

    /// Get the underlying path representation.
    ///
    /// On Unix, this is the socket file path.
    /// On Windows, this is the named pipe path.
    #[must_use]
    pub const fn as_path(&self) -> &PathBuf {
        &self.inner
    }

    /// Check if the address endpoint exists.
    ///
    /// On Unix, checks if the socket file exists.
    /// On Windows, this always returns true (named pipes don't have file presence).
    #[must_use]
    pub fn exists(&self) -> bool {
        #[cfg(unix)]
        {
            self.inner.exists()
        }

        #[cfg(windows)]
        {
            // Named pipes don't have a file system presence to check
            // The existence is determined by whether a server is listening
            true
        }
    }

    /// Clean up the address endpoint if applicable.
    ///
    /// On Unix, removes the socket file.
    /// On Windows, this is a no-op (named pipes are cleaned up automatically).
    pub fn cleanup(&self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.inner);
        }

        #[cfg(windows)]
        {
            // Named pipes are automatically cleaned up when the server closes
        }
    }
}

impl fmt::Display for IpcAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner.display())
    }
}

impl From<PathBuf> for IpcAddress {
    fn from(path: PathBuf) -> Self {
        Self { inner: path }
    }
}

impl From<&str> for IpcAddress {
    fn from(s: &str) -> Self {
        Self::from_string(s)
    }
}
