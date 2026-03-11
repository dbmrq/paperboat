//! Cross-platform IPC abstraction layer.
//!
//! This module provides a unified interface for bidirectional stream communication
//! between the main app and MCP server processes. It abstracts over:
//!
//! - **Unix** (macOS, Linux): Unix domain sockets (`tokio::net::UnixStream`)
//! - **Windows**: Named pipes (`tokio::net::windows::named_pipe`)
//!
//! # Design Decisions
//!
//! ## Trait-based vs Enum-based Approach
//!
//! We use a **trait-based approach** with conditional compilation rather than
//! an enum-based approach for several reasons:
//!
//! 1. **Zero runtime cost**: Platform selection happens at compile time, not runtime
//! 2. **Better type safety**: Platform-specific types are hidden behind the abstraction
//! 3. **Extensibility**: Easy to add new platform implementations without touching
//!    existing code
//! 4. **Cleaner API**: Users work with `impl IpcStream` rather than matching enums
//!
//! ## Platform Identifiers
//!
//! Rather than exposing `PathBuf` (Unix) vs `String` (Windows pipe names), we use
//! a unified `IpcAddress` type that encapsulates platform-specific addressing:
//!
//! - On Unix: wraps a `PathBuf` for socket file path
//! - On Windows: wraps a `String` for pipe name (e.g., `\\.\pipe\paperboat-xyz`)
//!
//! ## Error Handling
//!
//! Platform-specific errors are converted to `anyhow::Error` with context. This
//! provides a consistent error type while preserving diagnostic information.
//! The `IpcError` enum provides structured error variants for common failure modes.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                        IPC Module                            │
//! ├──────────────────────────────────────────────────────────────┤
//! │  IpcAddress    - Platform-agnostic endpoint identifier       │
//! │  IpcListener   - Accepts incoming connections (server-side)  │
//! │  IpcStream     - Bidirectional async stream (connection)     │
//! │  IpcError      - Structured error types                      │
//! └──────────────────────────────────────────────────────────────┘
//!            │                                │
//!   ┌────────┴────────┐            ┌──────────┴──────────┐
//!   │   Unix Module   │            │   Windows Module    │
//!   │  (UnixStream)   │            │   (NamedPipe)       │
//!   └─────────────────┘            └─────────────────────┘
//! ```
//!
//! # Usage Example
//!
//! ```ignore
//! use paperboat::ipc::{IpcAddress, IpcListener};
//!
//! // Server side
//! let addr = IpcAddress::generate("agent-abc123");
//! let listener = IpcListener::bind(&addr).await?;
//!
//! tokio::spawn(async move {
//!     while let Ok(stream) = listener.accept().await {
//!         // Handle connection with newline-delimited JSON
//!         let mut reader = BufReader::new(stream);
//!         let mut line = String::new();
//!         reader.read_line(&mut line).await?;
//!         // ... process request, send response ...
//!     }
//! });
//!
//! // Client side (MCP server connecting back)
//! let stream = IpcStream::connect(&addr).await?;
//! stream.write_all(b"{\"request\": ...}\n").await?;
//! ```
//!
//! # Platform-Specific Behavior
//!
//! | Behavior | Unix | Windows |
//! |----------|------|---------|
//! | Address format | `/tmp/vl-*.sock` | `\\.\pipe\vl-*` |
//! | `IpcAddress::exists()` | Checks socket file | Always `true` |
//! | `IpcAddress::cleanup()` | Removes socket file | No-op (auto-cleaned) |
//! | Retry on busy | Connection refused | `ERROR_PIPE_BUSY` |
//!
//! # Integrated Modules
//!
//! The following modules use this abstraction for IPC:
//!
//! - `src/app/socket.rs` - App-side IPC listener for tool calls
//! - `src/mcp_server/socket.rs` - MCP server-side IPC client
//! - `src/self_improve/runner.rs` - Self-improvement agent IPC

mod address;
mod error;
mod stream;

// Platform-specific implementations
#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
pub mod windows;

// Re-export public API
pub use address::IpcAddress;
pub use stream::{IpcListener, IpcStream};

// Re-export platform-specific utilities
#[cfg(unix)]
pub use unix::connect_with_retry;

#[cfg(windows)]
pub use windows::connect_with_retry;
