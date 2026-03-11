//! MCP (Model Context Protocol) error types.
//!
//! Errors related to MCP server operations and tool handling.

use thiserror::Error;

/// Errors that can occur during MCP operations.
#[derive(Debug, Error)]
#[allow(dead_code)] // Error enum with variants for MCP protocol errors
pub enum McpError {
    /// Requested tool was not found.
    #[error("Tool not found: '{tool_name}'. Available tools: {}", .available_tools.join(", "))]
    ToolNotFound {
        /// The name of the tool that was requested.
        tool_name: String,
        /// List of available tool names.
        available_tools: Vec<String>,
    },

    /// Invalid arguments were passed to a tool.
    #[error("Invalid arguments for tool '{tool_name}': {reason}")]
    InvalidArguments {
        /// The name of the tool.
        tool_name: String,
        /// Why the arguments were invalid.
        reason: String,
        /// The invalid arguments as JSON string, if available.
        arguments: Option<String>,
    },

    /// Tool handler failed during execution.
    #[error("Handler failed for tool '{tool_name}': {message}")]
    HandlerFailed {
        /// The name of the tool that failed.
        tool_name: String,
        /// Description of what went wrong.
        message: String,
    },

    /// Protocol-level error (invalid JSON-RPC, etc).
    #[error("MCP protocol error: {message}")]
    ProtocolError {
        /// The JSON-RPC error code, if applicable.
        code: i32,
        /// Description of the protocol error.
        message: String,
    },

    /// Parse error while processing request.
    #[error("Failed to parse MCP request: {message}")]
    ParseError {
        /// What went wrong during parsing.
        message: String,
        /// Preview of the input that couldn't be parsed.
        input_preview: Option<String>,
    },

    /// Invalid request structure.
    #[error("Invalid MCP request: {reason}")]
    InvalidRequest {
        /// Why the request was invalid.
        reason: String,
    },

    /// Socket communication error.
    #[error("MCP socket error: {message}")]
    SocketError {
        /// Description of the socket error.
        message: String,
    },

    /// Response channel error.
    #[error("Failed to send tool response: {0}")]
    ResponseFailed(String),
}

// JSON-RPC 2.0 standard error codes (re-exported for convenience)
#[allow(dead_code)] // Standard JSON-RPC 2.0 error codes
impl McpError {
    /// JSON-RPC 2.0: Parse error
    pub const CODE_PARSE_ERROR: i32 = -32700;
    /// JSON-RPC 2.0: Invalid request
    pub const CODE_INVALID_REQUEST: i32 = -32600;
    /// JSON-RPC 2.0: Method not found
    pub const CODE_METHOD_NOT_FOUND: i32 = -32601;
    /// JSON-RPC 2.0: Invalid params
    pub const CODE_INVALID_PARAMS: i32 = -32602;
    /// JSON-RPC 2.0: Internal error
    pub const CODE_INTERNAL_ERROR: i32 = -32603;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_not_found_display() {
        let err = McpError::ToolNotFound {
            tool_name: "unknown_tool".to_string(),
            available_tools: vec!["spawn_agents".to_string(), "complete".to_string()],
        };
        let display = format!("{err}");
        assert!(display.contains("unknown_tool"));
        assert!(display.contains("spawn_agents"));
        assert!(display.contains("complete"));
    }

    #[test]
    fn test_invalid_arguments_display() {
        let err = McpError::InvalidArguments {
            tool_name: "spawn_agents".to_string(),
            reason: "missing 'task' field".to_string(),
            arguments: Some("{}".to_string()),
        };
        let display = format!("{err}");
        assert!(display.contains("spawn_agents"));
        assert!(display.contains("missing 'task' field"));
    }

    #[test]
    fn test_handler_failed_display() {
        let err = McpError::HandlerFailed {
            tool_name: "decompose".to_string(),
            message: "task not found".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("decompose"));
        assert!(display.contains("task not found"));
    }

    #[test]
    fn test_protocol_error_display() {
        let err = McpError::ProtocolError {
            code: McpError::CODE_INVALID_REQUEST,
            message: "Missing jsonrpc field".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("Missing jsonrpc field"));
    }

    #[test]
    fn test_error_codes() {
        assert_eq!(McpError::CODE_PARSE_ERROR, -32700);
        assert_eq!(McpError::CODE_INVALID_REQUEST, -32600);
        assert_eq!(McpError::CODE_METHOD_NOT_FOUND, -32601);
        assert_eq!(McpError::CODE_INVALID_PARAMS, -32602);
        assert_eq!(McpError::CODE_INTERNAL_ERROR, -32603);
    }
}
