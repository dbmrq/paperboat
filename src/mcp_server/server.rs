//! Stdio MCP server for orchestrator tools.
//!
//! Provides decompose, implement, and complete tools via JSON-RPC over stdin/stdout.
//! Communicates tool calls back to the main app via a Unix socket.
//!
//! The server supports concurrent tool calls: each tool call opens its own socket
//! connection to the app, sends the request, and waits for a response. This allows
//! the orchestrator to make multiple parallel tool calls.

use super::error::{internal_error, parse_error};
use super::handlers::handle_request;
use super::socket::send_response;
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

/// Run the MCP server in stdio mode
///
/// This is called when the binary is invoked with --mcp-server flag.
/// It reads JSON-RPC requests from stdin, writes responses to stdout,
/// and sends tool calls to the app via a Unix socket.
///
/// Tool calls are handled concurrently - each spawns a task that opens its own
/// socket connection, sends the request, waits for the response, and sends
/// the MCP response back to stdout.
pub async fn run_stdio_server(socket_path: PathBuf) -> Result<()> {
    eprintln!("🔌 MCP server starting with socket path: {socket_path:?}");

    let stdin = tokio::io::stdin();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let socket_path = Arc::new(socket_path);

    eprintln!("✅ MCP server started");

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

        eprintln!("📥 MCP server received: {line}");

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
                let mut stdout = stdout.lock().await;
                if let Err(send_err) = send_response(&mut stdout, &error_response).await {
                    tracing::error!("❌ Failed to send parse error response: {}", send_err);
                }
                continue;
            }
        };

        // Spawn a task to handle the request concurrently
        // This allows multiple tool calls to be processed in parallel
        let stdout_clone = Arc::clone(&stdout);
        let socket_path_clone = Arc::clone(&socket_path);
        tokio::spawn(async move {
            // Handle the request, catching any errors
            let response = match handle_request(&request, &socket_path_clone).await {
                Ok(resp) => resp,
                Err(e) => {
                    let id = request.get("id");
                    let method = request
                        .get("method")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown");
                    tracing::error!("❌ Error handling request: {}. Method: {}", e, method);
                    Some(internal_error(
                        id,
                        &format!("handling {method} request"),
                        &e.to_string(),
                    ))
                }
            };

            // Send the response if there is one
            if let Some(resp) = response {
                let mut stdout = stdout_clone.lock().await;
                if let Err(e) = send_response(&mut stdout, &resp).await {
                    tracing::error!("❌ Failed to send response: {}. Continuing...", e);
                }
            }
        });
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
    fn test_tool_call_spawn_agents_serialization() {
        let tool_call = ToolCall::SpawnAgents {
            agents: vec![super::super::types::AgentSpec {
                role: "implementer".to_string(),
                task: "Fix the bug".to_string(),
                prompt: None,
                tools: None,
            }],
            wait: super::super::types::WaitMode::All,
        };
        let json = serde_json::to_string(&tool_call).unwrap();
        assert!(json.contains("SpawnAgents"));
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
    fn test_tool_call_spawn_agents_round_trip() {
        let original = ToolCall::SpawnAgents {
            agents: vec![super::super::types::AgentSpec {
                role: "implementer".to_string(),
                task: "Add user model".to_string(),
                prompt: None,
                tools: None,
            }],
            wait: super::super::types::WaitMode::All,
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ToolCall = serde_json::from_str(&json_str).unwrap();

        match deserialized {
            ToolCall::SpawnAgents { agents, wait } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].task, "Add user model");
                assert_eq!(wait, super::super::types::WaitMode::All);
            }
            _ => panic!("Expected SpawnAgents variant"),
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
        // For non-tool-call methods, we don't need a real socket - just a dummy path
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_json_rpc_request(1, "initialize", json!({}));
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert!(resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(
            resp["result"]["serverInfo"]["name"],
            "villalobos-orchestrator"
        );
    }

    // ========================================================================
    // tools/list Method Tests
    // ========================================================================

    #[tokio::test]
    async fn test_tools_list_returns_all_three_tools() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_json_rpc_request(2, "tools/list", json!({}));
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();

        assert_eq!(tools.len(), 3);

        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

        assert!(tool_names.contains(&"decompose"));
        assert!(tool_names.contains(&"spawn_agents"));
        assert!(tool_names.contains(&"complete"));
    }

    #[tokio::test]
    async fn test_tools_list_has_correct_schemas() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_json_rpc_request(3, "tools/list", json!({}));
        let response = handle_request(&request, &socket_path).await.unwrap();

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
    }

    // ========================================================================
    // Unknown Method Tests
    // ========================================================================

    #[tokio::test]
    async fn test_unknown_method_with_id_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_json_rpc_request(4, "nonexistent/method", json!({}));
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent/method"));
    }

    #[tokio::test]
    async fn test_notification_without_id_is_ignored() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        // Notification (no id field)
        let request = json!({
            "jsonrpc": "2.0",
            "method": "some/notification",
            "params": {}
        });
        let response = handle_request(&request, &socket_path).await.unwrap();

        // Notifications should return None
        assert!(response.is_none());
    }

    #[tokio::test]
    async fn test_request_missing_method_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "params": {}
        });
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("method"));
    }

    // ========================================================================
    // Tool Call Validation Tests
    // Note: These tests validate parameter parsing BEFORE socket communication,
    // so they don't need a real socket - validation errors are returned directly.
    // ========================================================================

    #[tokio::test]
    async fn test_tools_call_missing_params_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        // tools/call without params
        let request = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call"
        });
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_REQUEST);
    }

    #[tokio::test]
    async fn test_tools_call_missing_name_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_json_rpc_request(
            11,
            "tools/call",
            json!({
                "arguments": {"task": "test"}
            }),
        );
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn test_tools_call_missing_arguments_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_json_rpc_request(
            12,
            "tools/call",
            json!({
                "name": "decompose"
            }),
        );
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
    }

    #[tokio::test]
    async fn test_decompose_missing_task_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_tool_call_request(13, "decompose", json!({}));
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert!(resp["error"]["data"]["tool"].as_str().unwrap() == "decompose");
    }

    #[tokio::test]
    async fn test_spawn_agents_missing_agents_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_tool_call_request(14, "spawn_agents", json!({}));
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert!(resp["error"]["data"]["tool"].as_str().unwrap() == "spawn_agents");
    }

    #[tokio::test]
    async fn test_complete_missing_success_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_tool_call_request(
            15,
            "complete",
            json!({
                "message": "done"
            }),
        );
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], INVALID_PARAMS);
        assert!(resp["error"]["data"]["tool"].as_str().unwrap() == "complete");
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error() {
        let socket_path = std::env::temp_dir().join(format!("test-{}.sock", uuid::Uuid::new_v4()));

        let request = make_tool_call_request(
            16,
            "nonexistent_tool",
            json!({
                "arg": "value"
            }),
        );
        let response = handle_request(&request, &socket_path).await.unwrap();

        let resp = response.unwrap();
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
        assert!(resp["error"]["data"]["requested_method"].as_str().unwrap() == "nonexistent_tool");
    }
}
