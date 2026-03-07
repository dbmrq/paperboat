//! ACP (Agent Communication Protocol) error types.
//!
//! Errors related to communication with the auggie process via ACP.

use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during ACP operations.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum AcpError {
    /// Failed to establish connection with auggie.
    #[error("Failed to connect to auggie: {message}")]
    ConnectionFailed {
        /// Description of what went wrong.
        message: String,
        /// Optional suggestion for fixing the issue.
        #[source]
        suggestion: Option<AcpSuggestion>,
    },

    /// Request timed out waiting for response.
    #[error("Request timed out after {duration:?} while {context}")]
    RequestTimeout {
        /// How long we waited before timing out.
        duration: Duration,
        /// What operation was being attempted.
        context: String,
    },

    /// Received an invalid response from ACP.
    #[error("Invalid response from ACP: {message}")]
    InvalidResponse {
        /// Description of what was wrong with the response.
        message: String,
        /// The raw response that was invalid, if available.
        raw_response: Option<String>,
    },

    /// Error during session operations.
    #[error("Session error: {message}")]
    SessionError {
        /// Session ID if known.
        session_id: Option<String>,
        /// Description of what went wrong.
        message: String,
    },

    /// Response channel was closed unexpectedly.
    #[error("Response channel closed before receiving response")]
    ChannelClosed,

    /// Failed to serialize or send request.
    #[error("Failed to send request: {0}")]
    SendFailed(String),

    /// ACP returned an error in the response.
    #[error("ACP error: {message}")]
    ProtocolError {
        /// The error code from ACP, if any.
        code: Option<i32>,
        /// The error message from ACP.
        message: String,
    },

    /// Failed to parse response data.
    #[error("Failed to parse ACP response: {message}")]
    ParseError {
        /// What went wrong during parsing.
        message: String,
        /// The raw data that couldn't be parsed.
        raw_data: Option<String>,
    },
}

/// Suggestions for resolving ACP errors.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum AcpSuggestion {
    #[error("Is auggie installed and in your PATH?")]
    InstallAuggie,

    #[error("Try running 'auggie login' first to authenticate")]
    Authenticate,

    #[error("Check that the auggie process is running")]
    CheckProcess,

    #[error("{0}")]
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_failed_display() {
        let err = AcpError::ConnectionFailed {
            message: "spawn failed".to_string(),
            suggestion: Some(AcpSuggestion::InstallAuggie),
        };
        let display = format!("{err}");
        assert!(display.contains("spawn failed"));
    }

    #[test]
    fn test_request_timeout_display() {
        let err = AcpError::RequestTimeout {
            duration: Duration::from_secs(60),
            context: "session/new".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("60"));
        assert!(display.contains("session/new"));
    }

    #[test]
    fn test_invalid_response_display() {
        let err = AcpError::InvalidResponse {
            message: "missing sessionId field".to_string(),
            raw_response: Some("{\"result\": {}}".to_string()),
        };
        let display = format!("{err}");
        assert!(display.contains("missing sessionId"));
    }

    #[test]
    fn test_session_error_display() {
        let err = AcpError::SessionError {
            session_id: Some("session-123".to_string()),
            message: "session terminated unexpectedly".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("terminated unexpectedly"));
    }

    #[test]
    fn test_protocol_error_display() {
        let err = AcpError::ProtocolError {
            code: Some(-32600),
            message: "Invalid request".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("Invalid request"));
    }
}
