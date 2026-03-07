//! Orchestrator-level error types.
//!
//! Errors related to orchestrator operations, including timeout handling
//! for planning and session completion.

use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during orchestrator operations.
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// A timeout occurred while waiting for an operation to complete.
    #[error("Timeout after {duration:?} while {operation}{}", .context.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Timeout {
        /// Description of the operation that timed out.
        operation: TimeoutOperation,
        /// The duration after which the timeout occurred.
        duration: Duration,
        /// Optional additional context (e.g., session ID).
        context: Option<String>,
    },

    /// An internal error occurred (wraps `anyhow::Error` for compatibility).
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

/// Types of operations that can time out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutOperation {
    /// Waiting for a session to complete (any agent type).
    WaitForSession,
    /// Waiting for an ACP request/response (e.g., session/new, initialize).
    #[allow(dead_code)]
    AcpRequest,
}

impl std::fmt::Display for TimeoutOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitForSession => write!(f, "waiting for session"),
            Self::AcpRequest => write!(f, "waiting for ACP response"),
        }
    }
}

/// Configuration for timeout durations.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Timeout for waiting for any session to complete.
    /// Default: 30 minutes. Env: `PAPERBOAT_SESSION_TIMEOUT`
    pub session_timeout: Duration,

    /// Timeout for ACP request/response (e.g., session/new, initialize).
    /// Default: 60 seconds. Env: `PAPERBOAT_REQUEST_TIMEOUT`
    pub request_timeout: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl TimeoutConfig {
    /// Default session timeout in seconds (30 minutes)
    const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 30 * 60;

    /// Default request timeout in seconds (60 seconds)
    const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 60;

    /// Create a new `TimeoutConfig` with custom durations.
    #[allow(dead_code)]
    pub const fn new(session_timeout: Duration, request_timeout: Duration) -> Self {
        Self {
            session_timeout,
            request_timeout,
        }
    }

    /// Create a `TimeoutConfig` from environment variables.
    ///
    /// Supported environment variables:
    /// - `PAPERBOAT_SESSION_TIMEOUT`: Session timeout in seconds (default: 1800)
    /// - `PAPERBOAT_REQUEST_TIMEOUT`: ACP request timeout in seconds (default: 60)
    pub fn from_env() -> Self {
        let session_timeout = std::env::var("PAPERBOAT_SESSION_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map_or_else(
                || Duration::from_secs(Self::DEFAULT_SESSION_TIMEOUT_SECS),
                Duration::from_secs,
            );

        let request_timeout = std::env::var("PAPERBOAT_REQUEST_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map_or_else(
                || Duration::from_secs(Self::DEFAULT_REQUEST_TIMEOUT_SECS),
                Duration::from_secs,
            );

        Self {
            session_timeout,
            request_timeout,
        }
    }

    /// Create a `TimeoutConfig` with no timeouts (infinite wait).
    /// Useful for debugging or long-running tasks.
    #[allow(dead_code)]
    pub const fn no_timeout() -> Self {
        Self {
            session_timeout: Duration::MAX,
            request_timeout: Duration::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_config_default() {
        // Note: This tests the from_env() path; env vars may affect this
        let config = TimeoutConfig::default();
        // Defaults without env vars set
        assert!(config.session_timeout.as_secs() > 0);
        assert!(config.request_timeout.as_secs() > 0);
    }

    #[test]
    fn test_timeout_config_custom() {
        let config = TimeoutConfig::new(Duration::from_secs(120), Duration::from_secs(30));
        assert_eq!(config.session_timeout, Duration::from_secs(120));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_timeout_config_no_timeout() {
        let config = TimeoutConfig::no_timeout();
        assert_eq!(config.session_timeout, Duration::MAX);
        assert_eq!(config.request_timeout, Duration::MAX);
    }

    #[test]
    fn test_timeout_error_display() {
        let error = OrchestratorError::Timeout {
            operation: TimeoutOperation::WaitForSession,
            duration: Duration::from_secs(300),
            context: Some("session-123".to_string()),
        };
        let display = format!("{error}");
        assert!(display.contains("300"));
        assert!(display.contains("waiting for session"));
        assert!(display.contains("session-123"));
    }

    #[test]
    fn test_timeout_operation_display() {
        assert_eq!(
            format!("{}", TimeoutOperation::WaitForSession),
            "waiting for session"
        );
    }

    #[test]
    fn test_timeout_config_constants() {
        // Verify the default constants are reasonable
        assert_eq!(TimeoutConfig::DEFAULT_SESSION_TIMEOUT_SECS, 30 * 60);
        assert_eq!(TimeoutConfig::DEFAULT_REQUEST_TIMEOUT_SECS, 60);
    }
}
