//! Stdio MCP server for orchestrator tools.
//!
//! Provides decompose, implement, and complete tools via JSON-RPC over stdin/stdout.
//! Communicates tool calls back to the main app via a Unix socket.

use super::error::{internal_error, parse_error};
use super::handlers::handle_request;
use super::socket::{connect_with_retry, send_response};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Run the MCP server in stdio mode
///
/// This is called when the binary is invoked with --mcp-server flag.
/// It reads JSON-RPC requests from stdin, writes responses to stdout,
/// and sends tool calls to the app via a Unix socket.
pub async fn run_stdio_server(socket_path: PathBuf) -> Result<()> {
    eprintln!(
        "🔌 MCP server starting, connecting to socket: {:?}",
        socket_path
    );

    // Connect to the app's Unix socket with retry logic
    let mut socket = connect_with_retry(&socket_path)
        .await
        .context("Failed to connect to app socket after retries")?;

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    eprintln!("✅ MCP server started and connected");

    loop {
        // Read the next line, handling stdin errors gracefully
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                // EOF reached, clean shutdown
                eprintln!("🔌 MCP server shutting down (stdin closed)");
                break;
            }
            Err(e) => {
                tracing::error!("❌ Error reading from stdin: {}. Shutting down.", e);
                return Err(e.into());
            }
        };

        eprintln!("📥 MCP server received: {}", line);

        // Parse the JSON request, sending error response on failure
        let request: Value = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let input_preview = if line.len() > 200 {
                    &line[..200]
                } else {
                    &line
                };
                tracing::warn!(
                    "⚠️  Failed to parse JSON-RPC request: {}. Input preview: {}",
                    e,
                    input_preview
                );
                let error_response = parse_error(&e.to_string(), Some(input_preview));
                if let Err(send_err) = send_response(&mut stdout, &error_response).await {
                    tracing::error!("❌ Failed to send parse error response: {}", send_err);
                }
                continue;
            }
        };

        // Handle the request, catching any errors
        let response = match handle_request(&request, &mut socket, &socket_path).await {
            Ok(resp) => resp,
            Err(e) => {
                let id = request.get("id").cloned();
                let method = request
                    .get("method")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown");
                tracing::error!("❌ Error handling request: {}. Method: {}", e, method);
                Some(internal_error(
                    id,
                    &format!("handling {} request", method),
                    &e.to_string(),
                ))
            }
        };

        // Send the response if there is one
        if let Some(resp) = response {
            if let Err(e) = send_response(&mut stdout, &resp).await {
                tracing::error!("❌ Failed to send response: {}. Continuing...", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::error::{INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND};
    use crate::mcp_server::handlers::handle_request;
    use crate::mcp_server::types::ToolCall;
    use serde_json::json;

    // ========================================================================
    // ToolCall Serialization Tests
    // ========================================================================

    #[test]
    fn test_tool_call_decompose_serialization() {
        let tool_call = ToolCall::Decompose {
            task: "Build a feature".to_string(),
        };
        let json = serde_json::to_string(&tool_call).unwrap();
        assert!(json.contains("Decompose"));
        assert!(json.contains("Build a feature"));
    }

    #[test]
    fn test_tool_call_implement_serialization() {
        let tool_call = ToolCall::Implement {
            task: "Fix the bug".to_string(),
        };
        let json = serde_json::to_string(&tool_call).unwrap();
        assert!(json.contains("Implement"));
        assert!(json.contains("Fix the bug"));
    }

    #[test]
    fn test_tool_call_complete_serialization() {
        let tool_call = ToolCall::Complete {
            success: true,
            message: Some("All done".to_string()),
        };
        let json = serde_json::to_string(&tool_call).unwrap();
        assert!(json.contains("Complete"));
        assert!(json.contains("true"));
        assert!(json.contains("All done"));
    }

    #[test]
    fn test_tool_call_complete_without_message() {
        let tool_call = ToolCall::Complete {
            success: false,
            message: None,
        };
        let json = serde_json::to_string(&tool_call).unwrap();
        assert!(json.contains("Complete"));
        assert!(json.contains("false"));
    }

    // ========================================================================
    // ToolCall Deserialization/Round-trip Tests
    // ========================================================================

    #[test]
    fn test_tool_call_decompose_round_trip() {
        let original = ToolCall::Decompose {
            task: "Build a REST API".to_string(),
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ToolCall = serde_json::from_str(&json_str).unwrap();

        match deserialized {
            ToolCall::Decompose { task } => assert_eq!(task, "Build a REST API"),
            _ => panic!("Expected Decompose variant"),
        }
    }

    #[test]
    fn test_tool_call_implement_round_trip() {
        let original = ToolCall::Implement {
            task: "Add user model".to_string(),
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ToolCall = serde_json::from_str(&json_str).unwrap();

        match deserialized {
            ToolCall::Implement { task } => assert_eq!(task, "Add user model"),
            _ => panic!("Expected Implement variant"),
        }
    }

    #[test]
    fn test_tool_call_complete_round_trip_with_message() {
        let original = ToolCall::Complete {
            success: true,
            message: Some("All done!".to_string()),
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ToolCall = serde_json::from_str(&json_str).unwrap();

        match deserialized {
            ToolCall::Complete { success, message } => {
                assert!(success);
                assert_eq!(message, Some("All done!".to_string()));
            }
            _ => panic!("Expected Complete variant"),
        }
    }

    #[test]
    fn test_tool_call_complete_round_trip_without_message() {
        let original = ToolCall::Complete {
            success: false,
            message: None,
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ToolCall = serde_json::from_str(&json_str).unwrap();

        match deserialized {
            ToolCall::Complete { success, message } => {
                assert!(!success);
                assert!(message.is_none());
            }
            _ => panic!("Expected Complete variant"),
        }
    }

    #[test]
    fn test_tool_call_json_format_matches_app_expectations() {
        // Verify the serialized format can be parsed by handle_mcp_connection
        let decompose = ToolCall::Decompose {
            task: "Test task".to_string(),
        };
        let json_str = serde_json::to_string(&decompose).unwrap();

        // The app uses serde_json::from_str to parse, verify it works
        let parsed: ToolCall = serde_json::from_str(&json_str).unwrap();
        match parsed {
            ToolCall::Decompose { task } => assert_eq!(task, "Test task"),
            _ => panic!("Unexpected variant"),
        }
    }

    // ========================================================================
    // Request Builder Helpers for Testing
    // ========================================================================

    fn make_json_rpc_request(id: impl Into<Value>, method: &str, params: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id.into(),
            "method": method,
            "params": params
        })
    }

    fn make_tool_call_request(id: impl Into<Value>, tool_name: &str, arguments: Value) -> Value {
        make_json_rpc_request(
            id,
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments
            }),
        )
    }

    // ========================================================================
    // Initialize Method Tests
    // ========================================================================

    #[tokio::test]
    async fn test_initialize_returns_correct_protocol_version() {
        // Create a mock socket path and socket
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        // Spawn a task to accept the connection
        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        // Give listener time to start
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_json_rpc_request(1, "initialize", json!({}));
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert!(resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(resp["result"]["serverInfo"]["name"], "villalobos-orchestrator");

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    // ========================================================================
    // tools/list Method Tests
    // ========================================================================

    #[tokio::test]
    async fn test_tools_list_returns_all_three_tools() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_json_rpc_request(2, "tools/list", json!({}));
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();

        assert_eq!(tools.len(), 3);

        let tool_names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();

        assert!(tool_names.contains(&"decompose"));
        assert!(tool_names.contains(&"implement"));
        assert!(tool_names.contains(&"complete"));

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_tools_list_has_correct_schemas() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_json_rpc_request(3, "tools/list", json!({}));
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();

        // Find decompose tool and verify schema
        let decompose = tools.iter().find(|t| t["name"] == "decompose").unwrap();
        assert_eq!(decompose["inputSchema"]["type"], "object");
        assert!(decompose["inputSchema"]["properties"]["task"].is_object());
        assert!(decompose["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("task")));

        // Find complete tool and verify schema
        let complete = tools.iter().find(|t| t["name"] == "complete").unwrap();
        assert!(complete["inputSchema"]["properties"]["success"].is_object());
        assert!(complete["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("success")));

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    // ========================================================================
    // Unknown Method Tests
    // ========================================================================

    #[tokio::test]
    async fn test_unknown_method_with_id_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_json_rpc_request(4, "nonexistent/method", json!({}));
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent/method"));

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_notification_without_id_is_ignored() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        // Notification (no id field)
        let request = json!({
            "jsonrpc": "2.0",
            "method": "some/notification",
            "params": {}
        });
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        // Notifications should return None
        assert!(response.is_none());

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_request_missing_method_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "params": {}
        });
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST);
        assert!(resp["error"]["message"].as_str().unwrap().contains("method"));

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    // ========================================================================
    // Tool Call Validation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_tools_call_missing_params_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        // tools/call without params
        let request = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call"
        });
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST);

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_tools_call_missing_name_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_json_rpc_request(
            11,
            "tools/call",
            json!({
                "arguments": {"task": "test"}
            }),
        );
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_tools_call_missing_arguments_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_json_rpc_request(
            12,
            "tools/call",
            json!({
                "name": "decompose"
            }),
        );
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_decompose_missing_task_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_tool_call_request(13, "decompose", json!({}));
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert!(resp["error"]["data"]["tool"].as_str().unwrap() == "decompose");

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_implement_missing_task_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_tool_call_request(14, "implement", json!({}));
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert!(resp["error"]["data"]["tool"].as_str().unwrap() == "implement");

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_complete_missing_success_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_tool_call_request(
            15,
            "complete",
            json!({
                "message": "done"
            }),
        );
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert!(resp["error"]["data"]["tool"].as_str().unwrap() == "complete");

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error() {
        let socket_path =
            std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let socket_path_clone = socket_path.clone();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut socket = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

        let request = make_tool_call_request(
            16,
            "nonexistent_tool",
            json!({
                "arg": "value"
            }),
        );
        let response = handle_request(&request, &mut socket, &socket_path_clone)
            .await
            .unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
        assert!(
            resp["error"]["data"]["requested_method"]
                .as_str()
                .unwrap()
                == "nonexistent_tool"
        );

        // Cleanup
        let _ = std::fs::remove_file(&socket_path);
    }
}

