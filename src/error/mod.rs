//! Error types for the Paperboat orchestrator.
//!
//! This module provides a granular error hierarchy for better error handling
//! and debugging. Each subsystem has its own error type with clear, actionable
//! error messages.
//!
//! # Error Hierarchy
//!
//! ```text
//! PaperboatError (top-level)
//! ├── AcpError (connection, timeout, invalid response, session errors)
//! ├── McpError (tool not found, invalid arguments, handler failed, protocol errors)
//! ├── TaskError (task not found, invalid status, dependency failed, validation failed)
//! ├── ConfigError (file not found, parse error, invalid model, merge conflict)
//! └── OrchestratorError (timeout, internal errors)
//! ```
//!
//! # Example
//!
//! ```ignore
//! use paperboat::error::{AcpError, PaperboatError};
//!
//! fn connect_to_agent() -> Result<(), PaperboatError> {
//!     Err(AcpError::ConnectionFailed {
//!         message: "Failed to spawn auggie process".to_string(),
//!         suggestion: Some("Is auggie installed and in your PATH?".to_string()),
//!     }.into())
//! }
//! ```

mod acp;
mod config;
mod mcp;
mod orchestrator;
mod task;

// Re-export all error types for convenient access
pub use acp::AcpError;
pub use config::{suggest_model_alias, ConfigError, KNOWN_MODEL_ALIASES};
pub use mcp::McpError;
pub use orchestrator::{OrchestratorError, TimeoutConfig, TimeoutOperation};
pub use task::TaskError;

use thiserror::Error;

/// Top-level error type for all Paperboat operations.
///
/// This enum encompasses all error types in the Paperboat system,
/// allowing for unified error handling at the application level.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum PaperboatError {
    /// ACP (Agent Communication Protocol) errors
    #[error(transparent)]
    Acp(#[from] AcpError),

    /// MCP (Model Context Protocol) errors
    #[error(transparent)]
    Mcp(#[from] McpError),

    /// Task management errors
    #[error(transparent)]
    Task(#[from] TaskError),

    /// Configuration errors
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// Orchestrator-level errors
    #[error(transparent)]
    Orchestrator(#[from] OrchestratorError),

    /// Catch-all for other errors (wraps `anyhow::Error`)
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_paperboat_error_from_acp() {
        let acp_err = AcpError::ConnectionFailed {
            message: "test error".to_string(),
            suggestion: None,
        };
        let err: PaperboatError = acp_err.into();
        assert!(matches!(err, PaperboatError::Acp(_)));
    }

    #[test]
    fn test_paperboat_error_from_mcp() {
        let mcp_err = McpError::ToolNotFound {
            tool_name: "unknown".to_string(),
            available_tools: vec!["spawn_agents".to_string()],
        };
        let err: PaperboatError = mcp_err.into();
        assert!(matches!(err, PaperboatError::Mcp(_)));
    }

    #[test]
    fn test_paperboat_error_from_task() {
        let task_err = TaskError::NotFound {
            task_id: "task001".to_string(),
            suggestion: None,
        };
        let err: PaperboatError = task_err.into();
        assert!(matches!(err, PaperboatError::Task(_)));
    }

    #[test]
    fn test_paperboat_error_from_config() {
        let config_err = ConfigError::FileNotFound {
            path: "/some/path".into(),
        };
        let err: PaperboatError = config_err.into();
        assert!(matches!(err, PaperboatError::Config(_)));
    }

    #[test]
    fn test_paperboat_error_from_orchestrator() {
        let orch_err = OrchestratorError::Timeout {
            operation: TimeoutOperation::WaitForSession,
            duration: Duration::from_secs(30),
            context: None,
        };
        let err: PaperboatError = orch_err.into();
        assert!(matches!(err, PaperboatError::Orchestrator(_)));
    }
}
