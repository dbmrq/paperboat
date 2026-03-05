//! Error types for the Villalobos orchestrator.
//!
//! This module defines error types for orchestrator-level operations,
//! including timeout handling for planning and session completion.

use std::fmt;
use std::time::Duration;

/// Errors that can occur during orchestrator operations.
#[derive(Debug)]
pub enum OrchestratorError {
    /// A timeout occurred while waiting for an operation to complete.
    Timeout {
        /// Description of the operation that timed out.
        operation: TimeoutOperation,
        /// The duration after which the timeout occurred.
        duration: Duration,
        /// Optional additional context (e.g., session ID).
        context: Option<String>,
    },
    /// An internal error occurred (wraps anyhow::Error for compatibility).
    Internal(anyhow::Error),
}

/// Types of operations that can time out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutOperation {
    /// Waiting for a plan from a planner session.
    WaitForPlan,
    /// Waiting for a session to complete (implementer finishing work).
    WaitForSessionComplete,
}

impl fmt::Display for TimeoutOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeoutOperation::WaitForPlan => write!(f, "waiting for plan"),
            TimeoutOperation::WaitForSessionComplete => write!(f, "waiting for session completion"),
        }
    }
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrchestratorError::Timeout {
                operation,
                duration,
                context,
            } => {
                write!(
                    f,
                    "Timeout after {:?} while {operation}",
                    duration,
                )?;
                if let Some(ctx) = context {
                    write!(f, " ({})", ctx)?;
                }
                Ok(())
            }
            OrchestratorError::Internal(e) => write!(f, "Internal error: {}", e),
        }
    }
}

impl std::error::Error for OrchestratorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OrchestratorError::Internal(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for OrchestratorError {
    fn from(e: anyhow::Error) -> Self {
        OrchestratorError::Internal(e)
    }
}

/// Configuration for timeout durations.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Timeout for waiting for a plan from a planner session.
    /// Default: 5 minutes.
    pub plan_timeout: Duration,
    /// Timeout for waiting for a session to complete (e.g., implementer finishing).
    /// Default: 30 minutes.
    pub session_complete_timeout: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            plan_timeout: Duration::from_secs(5 * 60),           // 5 minutes
            session_complete_timeout: Duration::from_secs(30 * 60), // 30 minutes
        }
    }
}

impl TimeoutConfig {
    /// Create a new TimeoutConfig with custom durations.
    pub fn new(plan_timeout: Duration, session_complete_timeout: Duration) -> Self {
        Self {
            plan_timeout,
            session_complete_timeout,
        }
    }

    /// Create a TimeoutConfig with no timeouts (infinite wait).
    /// Useful for debugging or long-running tasks.
    pub fn no_timeout() -> Self {
        Self {
            plan_timeout: Duration::MAX,
            session_complete_timeout: Duration::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_config_default() {
        let config = TimeoutConfig::default();
        assert_eq!(config.plan_timeout, Duration::from_secs(300));
        assert_eq!(config.session_complete_timeout, Duration::from_secs(1800));
    }

    #[test]
    fn test_timeout_config_custom() {
        let config = TimeoutConfig::new(
            Duration::from_secs(60),
            Duration::from_secs(120),
        );
        assert_eq!(config.plan_timeout, Duration::from_secs(60));
        assert_eq!(config.session_complete_timeout, Duration::from_secs(120));
    }

    #[test]
    fn test_timeout_config_no_timeout() {
        let config = TimeoutConfig::no_timeout();
        assert_eq!(config.plan_timeout, Duration::MAX);
        assert_eq!(config.session_complete_timeout, Duration::MAX);
    }

    #[test]
    fn test_timeout_error_display() {
        let error = OrchestratorError::Timeout {
            operation: TimeoutOperation::WaitForPlan,
            duration: Duration::from_secs(300),
            context: Some("session-123".to_string()),
        };
        let display = format!("{}", error);
        assert!(display.contains("300"));
        assert!(display.contains("waiting for plan"));
        assert!(display.contains("session-123"));
    }

    #[test]
    fn test_timeout_operation_display() {
        assert_eq!(format!("{}", TimeoutOperation::WaitForPlan), "waiting for plan");
        assert_eq!(
            format!("{}", TimeoutOperation::WaitForSessionComplete),
            "waiting for session completion"
        );
    }
}

