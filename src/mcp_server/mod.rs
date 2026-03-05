//! MCP Server module.
//!
//! This module provides the stdio MCP server implementation for orchestrator tools.

pub mod error;
mod handlers;
mod server;
mod socket;
mod types;

// Re-export for backward compatibility and easy access
pub use server::run_stdio_server;
pub use types::{ToolCall, ToolRequest, ToolResponse};

