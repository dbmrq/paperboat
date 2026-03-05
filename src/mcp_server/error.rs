//! JSON-RPC 2.0 error handling for the MCP server.
//!
//! This module provides error code constants and builder functions for constructing
//! JSON-RPC 2.0 compliant error responses with additional structured data.

use serde_json::{json, Value};

// JSON-RPC 2.0 standard error codes
pub(crate) const PARSE_ERROR: i32 = -32700;
pub(crate) const INVALID_REQUEST: i32 = -32600;
pub(crate) const METHOD_NOT_FOUND: i32 = -32601;
pub(crate) const INVALID_PARAMS: i32 = -32602;
pub(crate) const INTERNAL_ERROR: i32 = -32603;

/// Create a JSON-RPC 2.0 error response without additional data.
#[allow(dead_code)]
pub(crate) fn json_rpc_error(id: Option<Value>, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// Create a JSON-RPC 2.0 error response with additional structured data.
pub(crate) fn json_rpc_error_with_data(id: Option<Value>, code: i32, message: &str, data: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message, "data": data }
    })
}

/// Create a parse error response (-32700).
///
/// Used when the server receives invalid JSON or cannot parse the request.
pub(crate) fn parse_error(parse_err: &str, input_preview: Option<&str>) -> Value {
    let data = match input_preview {
        Some(preview) => json!({ "parse_error": parse_err, "input_preview": preview }),
        None => json!({ "parse_error": parse_err }),
    };
    json_rpc_error_with_data(None, PARSE_ERROR, &format!("Parse error: {}", parse_err), data)
}

/// Create an invalid request error response (-32600).
///
/// Used when the JSON-RPC request structure is invalid (e.g., missing required fields).
pub(crate) fn invalid_request_error(id: Option<Value>, reason: &str) -> Value {
    json_rpc_error_with_data(
        id,
        INVALID_REQUEST,
        &format!("Invalid request: {}", reason),
        json!({ "reason": reason }),
    )
}

/// Create a method not found error response (-32601).
///
/// Used when the requested method does not exist.
pub(crate) fn method_not_found_error(id: Option<Value>, method: &str, available: &[&str]) -> Value {
    json_rpc_error_with_data(
        id,
        METHOD_NOT_FOUND,
        &format!("Method not found: {}", method),
        json!({ "requested_method": method, "available_methods": available }),
    )
}

/// Create an invalid params error response (-32602).
///
/// Used when the method parameters are invalid or missing required fields.
pub(crate) fn invalid_params_error(id: Option<Value>, tool_name: &str, reason: &str) -> Value {
    json_rpc_error_with_data(
        id,
        INVALID_PARAMS,
        &format!("Invalid params for '{}': {}", tool_name, reason),
        json!({ "tool": tool_name, "reason": reason }),
    )
}

/// Create an internal error response (-32603).
///
/// Used when an internal server error occurs during request processing.
pub(crate) fn internal_error(id: Option<Value>, operation: &str, error: &str) -> Value {
    json_rpc_error_with_data(
        id,
        INTERNAL_ERROR,
        &format!("Internal error during {}: {}", operation, error),
        json!({ "operation": operation, "error": error }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Error Response Builder Tests
    // ========================================================================

    #[test]
    fn test_json_rpc_error_with_id() {
        let response = json_rpc_error(Some(json!(1)), PARSE_ERROR, "Test error");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["error"]["code"], PARSE_ERROR);
        assert_eq!(response["error"]["message"], "Test error");
    }

    #[test]
    fn test_json_rpc_error_without_id() {
        let response = json_rpc_error(None, PARSE_ERROR, "Test error");

        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response["id"].is_null());
        assert_eq!(response["error"]["code"], PARSE_ERROR);
    }

    #[test]
    fn test_json_rpc_error_with_data() {
        let data = json!({"key": "value"});
        let response = json_rpc_error_with_data(Some(json!(2)), INTERNAL_ERROR, "Error msg", data);

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 2);
        assert_eq!(response["error"]["code"], INTERNAL_ERROR);
        assert_eq!(response["error"]["message"], "Error msg");
        assert_eq!(response["error"]["data"]["key"], "value");
    }

    #[test]
    fn test_parse_error_with_preview() {
        let response = parse_error("unexpected token", Some("{invalid"));

        assert_eq!(response["error"]["code"], PARSE_ERROR);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unexpected token"));
        assert_eq!(response["error"]["data"]["input_preview"], "{invalid");
    }

    #[test]
    fn test_parse_error_without_preview() {
        let response = parse_error("unexpected EOF", None);

        assert_eq!(response["error"]["code"], PARSE_ERROR);
        assert!(response["error"]["data"]["input_preview"].is_null());
    }

    #[test]
    fn test_invalid_request_error() {
        let response = invalid_request_error(Some(json!(3)), "missing method field");

        assert_eq!(response["error"]["code"], INVALID_REQUEST);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing method field"));
        assert_eq!(response["error"]["data"]["reason"], "missing method field");
    }

    #[test]
    fn test_method_not_found_error() {
        let response = method_not_found_error(Some(json!(4)), "unknown_method", &["method1", "method2"]);

        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown_method"));
        assert_eq!(response["error"]["data"]["requested_method"], "unknown_method");
        assert_eq!(response["error"]["data"]["available_methods"][0], "method1");
        assert_eq!(response["error"]["data"]["available_methods"][1], "method2");
    }

    #[test]
    fn test_invalid_params_error() {
        let response = invalid_params_error(Some(json!(5)), "decompose", "missing task argument");

        assert_eq!(response["error"]["code"], INVALID_PARAMS);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("decompose"));
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing task argument"));
        assert_eq!(response["error"]["data"]["tool"], "decompose");
        assert_eq!(response["error"]["data"]["reason"], "missing task argument");
    }

    #[test]
    fn test_internal_error() {
        let response = internal_error(Some(json!(6)), "socket write", "connection refused");

        assert_eq!(response["error"]["code"], INTERNAL_ERROR);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("socket write"));
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("connection refused"));
        assert_eq!(response["error"]["data"]["operation"], "socket write");
        assert_eq!(response["error"]["data"]["error"], "connection refused");
    }

    // ========================================================================
    // Standard Error Codes Verification
    // ========================================================================

    #[test]
    fn test_error_codes_are_standard_json_rpc() {
        // Standard JSON-RPC 2.0 error codes
        assert_eq!(PARSE_ERROR, -32700);
        assert_eq!(INVALID_REQUEST, -32600);
        assert_eq!(METHOD_NOT_FOUND, -32601);
        assert_eq!(INVALID_PARAMS, -32602);
        assert_eq!(INTERNAL_ERROR, -32603);
    }
}

