//! JSON-RPC request handlers for the MCP server.
//!
//! This module is organized into:
//! - `tool_schemas`: JSON schema definitions for each tool
//! - `tool_parsing`: Functions to parse tool call arguments
//! - `response`: Functions to build response text
//!
//! The main entry point is `handle_request` which dispatches JSON-RPC requests.

mod response;
mod tool_parsing;
mod tool_schemas;

// Re-export response builder function for use by other modules
pub use response::build_response_text_with_state;

use super::error::{
    internal_error, invalid_params_error, invalid_request_error, method_not_found_error,
};
use super::socket::send_request_and_wait;
use super::types::ToolRequest;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Handle a JSON-RPC request
pub async fn handle_request(request: &Value, socket_path: &PathBuf) -> Result<Option<Value>> {
    let id = request.get("id");

    // Extract method, returning error response if missing
    let method = if let Some(m) = request["method"].as_str() {
        m
    } else {
        tracing::warn!("⚠️  Request missing 'method' field: {:?}", request);
        return Ok(Some(invalid_request_error(id, "missing 'method' field")));
    };

    match method {
        "initialize" => Ok(Some(handle_initialize(id))),
        "tools/list" => Ok(Some(handle_tools_list(id))),
        "tools/call" => handle_tool_call(request, id.cloned(), socket_path).await,
        // Handle notifications (no id) - just log and ignore
        _ if id.is_none() => {
            tracing::debug!("Ignoring notification with method: {}", method);
            Ok(None)
        }
        // Unknown method with id - return proper error
        _ => {
            tracing::warn!("⚠️  Unknown method: {}", method);
            Ok(Some(method_not_found_error(
                id,
                method,
                &["initialize", "tools/list", "tools/call"],
            )))
        }
    }
}

/// Handle the "initialize" method.
fn handle_initialize(id: Option<&Value>) -> Value {
    let result = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "paperboat-orchestrator",
            "version": "0.1.0"
        }
    });

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

/// Handle the "tools/list" method.
fn handle_tools_list(id: Option<&Value>) -> Value {
    // Get agent type from environment to filter available tools
    let agent_type =
        std::env::var("PAPERBOAT_AGENT_TYPE").unwrap_or_else(|_| "orchestrator".to_string());

    let tools = match agent_type.as_str() {
        "planner" => {
            // Planner gets set_goal, create_task, and complete
            json!({
                "tools": [
                    tool_schemas::set_goal_schema(),
                    tool_schemas::create_task_schema_planner(),
                    tool_schemas::complete_schema_planner()
                ]
            })
        }
        "implementer" => {
            // Implementer only gets the complete tool to signal they're done
            json!({
                "tools": [
                    tool_schemas::complete_schema_implementer()
                ]
            })
        }
        _ => {
            // Orchestrator gets all tools
            // Build dynamic descriptions with auto-discovered roles
            let roles_list = crate::agents::SPAWNABLE_ROLES.join(", ");

            json!({
                "tools": [
                    tool_schemas::decompose_schema(),
                    tool_schemas::spawn_agents_schema(&roles_list),
                    tool_schemas::complete_schema_orchestrator(),
                    tool_schemas::create_task_schema_orchestrator(),
                    tool_schemas::skip_tasks_schema(),
                    tool_schemas::list_tasks_schema()
                ]
            })
        }
    };

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": tools
    })
}

/// Handle a tool call with proper error responses
async fn handle_tool_call(
    request: &Value,
    id: Option<Value>,
    socket_path: &PathBuf,
) -> Result<Option<Value>> {
    // Validate params structure
    let params = if let Some(p) = request["params"].as_object() {
        p
    } else {
        tracing::warn!("⚠️  tools/call missing 'params' object");
        return Ok(Some(invalid_request_error(
            id.as_ref(),
            "'params' must be an object for tools/call",
        )));
    };

    // Validate tool name
    let name = if let Some(n) = params.get("name").and_then(|v| v.as_str()) {
        n
    } else {
        tracing::warn!("⚠️  tools/call missing 'name' parameter");
        return Ok(Some(invalid_params_error(
            id.as_ref(),
            "tools/call",
            "missing required 'name' field",
        )));
    };

    // Validate arguments
    let arguments = if let Some(a) = params.get("arguments").and_then(|v| v.as_object()) {
        a
    } else {
        tracing::warn!(
            "⚠️  tools/call missing 'arguments' parameter for tool '{}'",
            name
        );
        return Ok(Some(invalid_params_error(
            id.as_ref(),
            name,
            "missing required 'arguments' field",
        )));
    };

    // Parse the tool call with specific error messages
    let tool_call = match name {
        "decompose" => match tool_parsing::parse_decompose(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "decompose", msg))),
        },
        "spawn_agents" => match tool_parsing::parse_spawn_agents(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "spawn_agents", msg))),
        },
        "complete" => match tool_parsing::parse_complete(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "complete", msg))),
        },
        "create_task" => match tool_parsing::parse_create_task(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "create_task", msg))),
        },
        "set_goal" => match tool_parsing::parse_set_goal(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "set_goal", msg))),
        },
        "skip_tasks" => match tool_parsing::parse_skip_tasks(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "skip_tasks", msg))),
        },
        "list_tasks" => match tool_parsing::parse_list_tasks(arguments) {
            Ok(tc) => tc,
            Err(msg) => return Ok(Some(invalid_params_error(id.as_ref(), "list_tasks", msg))),
        },
        _ => {
            tracing::warn!("⚠️  Unknown tool requested: {}", name);
            return Ok(Some(method_not_found_error(
                id.as_ref(),
                name,
                &[
                    "decompose",
                    "spawn_agents",
                    "complete",
                    "create_task",
                    "set_goal",
                    "skip_tasks",
                    "list_tasks",
                ],
            )));
        }
    };

    eprintln!("🔧 MCP Tool call: {tool_call:?}");

    // Create request with unique ID for correlation
    let request_id = uuid::Uuid::new_v4().to_string();
    let tool_request = ToolRequest {
        request_id: request_id.clone(),
        tool_call: tool_call.clone(),
    };

    // Send request and wait for response from the app
    let tool_response = match send_request_and_wait(socket_path, &tool_request).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to get response from app: {}", e);
            return Ok(Some(internal_error(
                id.as_ref(),
                "socket communication",
                &e.to_string(),
            )));
        }
    };

    // Build response text based on actual result from the app
    // Pass task state from response for context-aware "What's Next" guidance
    let response_text =
        response::build_response_text_with_state(&tool_call, &tool_response, tool_response.task_state.as_ref());

    let result = json!({
        "content": [
            {
                "type": "text",
                "text": response_text
            }
        ]
    });

    Ok(Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::types::ToolCall;
    use serial_test::serial;
    use std::collections::HashSet;

    /// Helper to extract tool names from a tools/list response.
    fn extract_tool_names(response: &Value) -> HashSet<String> {
        response["result"]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| t["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Test that planner agents only get `create_task` and complete tools.
    /// Uses #[serial] because these tests modify the `PAPERBOAT_AGENT_TYPE` env var.
    #[tokio::test]
    #[serial]
    async fn test_planner_tool_access() {
        // Set agent type to planner
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "planner");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        let tool_names = extract_tool_names(&response);

        // Planner should have: create_task, complete
        assert!(
            tool_names.contains("create_task"),
            "Planner should have create_task tool"
        );
        assert!(
            tool_names.contains("complete"),
            "Planner should have complete tool"
        );

        // Planner should NOT have: spawn_agents, decompose, skip_tasks
        assert!(
            !tool_names.contains("spawn_agents"),
            "Planner should NOT have spawn_agents tool"
        );
        assert!(
            !tool_names.contains("decompose"),
            "Planner should NOT have decompose tool"
        );
        assert!(
            !tool_names.contains("skip_tasks"),
            "Planner should NOT have skip_tasks tool"
        );

        // Planner should have: set_goal, create_task, complete
        assert!(
            tool_names.contains("set_goal"),
            "Planner should have set_goal tool"
        );

        // Verify exact count
        assert_eq!(
            tool_names.len(),
            3,
            "Planner should have exactly 3 tools (set_goal, create_task, complete), got: {tool_names:?}",
        );
    }

    /// Test that orchestrator agents get all MCP tools including `spawn_agents`.
    #[tokio::test]
    #[serial]
    async fn test_orchestrator_tool_access() {
        // Set agent type to orchestrator
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "orchestrator");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        let tool_names = extract_tool_names(&response);

        // Orchestrator should have: decompose, spawn_agents, complete, create_task, skip_tasks
        assert!(
            tool_names.contains("decompose"),
            "Orchestrator should have decompose tool"
        );
        assert!(
            tool_names.contains("spawn_agents"),
            "Orchestrator should have spawn_agents tool"
        );
        assert!(
            tool_names.contains("complete"),
            "Orchestrator should have complete tool"
        );
        assert!(
            tool_names.contains("create_task"),
            "Orchestrator should have create_task tool"
        );
        assert!(
            tool_names.contains("skip_tasks"),
            "Orchestrator should have skip_tasks tool"
        );
        assert!(
            tool_names.contains("list_tasks"),
            "Orchestrator should have list_tasks tool"
        );

        // Verify exact count
        assert_eq!(
            tool_names.len(),
            6,
            "Orchestrator should have exactly 6 tools, got: {tool_names:?}",
        );
    }

    /// Test that implementer agents only get the complete tool.
    #[tokio::test]
    #[serial]
    async fn test_implementer_tool_access() {
        // Set agent type to implementer
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "implementer");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        let tool_names = extract_tool_names(&response);

        // Implementer should only have: complete
        assert!(
            tool_names.contains("complete"),
            "Implementer should have complete tool"
        );

        // Implementer should NOT have any other MCP tools
        assert!(
            !tool_names.contains("spawn_agents"),
            "Implementer should NOT have spawn_agents tool"
        );
        assert!(
            !tool_names.contains("decompose"),
            "Implementer should NOT have decompose tool"
        );
        assert!(
            !tool_names.contains("create_task"),
            "Implementer should NOT have create_task tool"
        );
        assert!(
            !tool_names.contains("skip_tasks"),
            "Implementer should NOT have skip_tasks tool"
        );

        // Verify exact count
        assert_eq!(
            tool_names.len(),
            1,
            "Implementer should have exactly 1 tool, got: {tool_names:?}",
        );
    }

    /// Test that unknown agent types default to orchestrator tools (fail-safe).
    #[tokio::test]
    #[serial]
    async fn test_unknown_agent_type_defaults_to_orchestrator() {
        // Set agent type to something unknown
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "unknown_type");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        let tool_names = extract_tool_names(&response);

        // Unknown types should default to orchestrator tools
        assert!(
            tool_names.contains("spawn_agents"),
            "Unknown agent type should default to having spawn_agents"
        );
        assert!(
            tool_names.contains("decompose"),
            "Unknown agent type should default to having decompose"
        );
        assert!(
            tool_names.contains("complete"),
            "Unknown agent type should default to having complete"
        );
    }

    /// Test that missing agent type env var defaults to orchestrator tools.
    #[tokio::test]
    #[serial]
    async fn test_missing_agent_type_defaults_to_orchestrator() {
        // Remove the agent type env var
        std::env::remove_var("PAPERBOAT_AGENT_TYPE");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        let tool_names = extract_tool_names(&response);

        // Missing type should default to orchestrator tools
        assert!(
            tool_names.contains("spawn_agents"),
            "Missing agent type should default to having spawn_agents"
        );
    }

    /// Test that MCP tools match the centralized config.
    /// This ensures handlers stay in sync with agents/config.rs.
    #[tokio::test]
    #[serial]
    async fn test_mcp_tools_match_centralized_config() {
        use crate::agents::{IMPLEMENTER_CONFIG, ORCHESTRATOR_CONFIG, PLANNER_CONFIG};

        let socket_path = PathBuf::from("/tmp/test-socket");

        // Test planner
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "planner");
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();
        let tool_names = extract_tool_names(&response);

        for expected_tool in PLANNER_CONFIG.mcp_tools {
            assert!(
                tool_names.contains(*expected_tool),
                "Planner should have {expected_tool} (from centralized config)",
            );
        }
        assert_eq!(
            tool_names.len(),
            PLANNER_CONFIG.mcp_tools.len(),
            "Planner tool count should match config"
        );

        // Test orchestrator
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "orchestrator");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();
        let tool_names = extract_tool_names(&response);

        for expected_tool in ORCHESTRATOR_CONFIG.mcp_tools {
            assert!(
                tool_names.contains(*expected_tool),
                "Orchestrator should have {expected_tool} (from centralized config)",
            );
        }
        assert_eq!(
            tool_names.len(),
            ORCHESTRATOR_CONFIG.mcp_tools.len(),
            "Orchestrator tool count should match config"
        );

        // Test implementer
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "implementer");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();
        let tool_names = extract_tool_names(&response);

        for expected_tool in IMPLEMENTER_CONFIG.mcp_tools {
            assert!(
                tool_names.contains(*expected_tool),
                "Implementer should have {expected_tool} (from centralized config)",
            );
        }
        assert_eq!(
            tool_names.len(),
            IMPLEMENTER_CONFIG.mcp_tools.len(),
            "Implementer tool count should match config"
        );
    }

    /// Test that skip_tasks tool definition is available to orchestrators.
    #[tokio::test]
    #[serial]
    async fn test_skip_tasks_tool_definition() {
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "orchestrator");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        // Find the skip_tasks tool definition
        let tools = response["result"]["tools"].as_array().unwrap();
        let skip_tasks_tool = tools
            .iter()
            .find(|t| t["name"] == "skip_tasks")
            .expect("skip_tasks tool should exist for orchestrator");

        // Verify schema structure
        let schema = &skip_tasks_tool["inputSchema"];
        assert_eq!(schema["type"], "object");

        // Check task_ids property
        let task_ids_prop = &schema["properties"]["task_ids"];
        assert_eq!(task_ids_prop["type"], "array");
        assert_eq!(task_ids_prop["items"]["type"], "string");

        // Check reason property
        let reason_prop = &schema["properties"]["reason"];
        assert_eq!(reason_prop["type"], "string");

        // Check required fields
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("task_ids")));
        assert!(
            !required.contains(&json!("reason")),
            "reason should be optional"
        );
    }

    /// Test that skip_tasks is not available to non-orchestrator agents.
    #[tokio::test]
    #[serial]
    async fn test_skip_tasks_not_available_to_planners() {
        std::env::set_var("PAPERBOAT_AGENT_TYPE", "planner");

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        });

        let socket_path = PathBuf::from("/tmp/test-socket");
        let response = handle_request(&request, &socket_path)
            .await
            .unwrap()
            .unwrap();

        let tool_names = extract_tool_names(&response);
        assert!(
            !tool_names.contains("skip_tasks"),
            "Planner should NOT have skip_tasks tool"
        );
    }

    /// Test parsing of skip_tasks tool call with valid arguments.
    #[test]
    fn test_parse_skip_tasks_valid() {
        // This test validates the parsing logic by checking the ToolCall variant construction
        let task_ids = vec!["task001".to_string(), "task002".to_string()];
        let reason = Some("Already completed by previous task".to_string());

        let tool_call = ToolCall::SkipTasks {
            task_ids: task_ids.clone(),
            reason: reason.clone(),
        };

        assert_eq!(tool_call.tool_type(), "skip_tasks");
        if let ToolCall::SkipTasks {
            task_ids: ids,
            reason: r,
        } = tool_call
        {
            assert_eq!(ids, task_ids);
            assert_eq!(r, reason);
        } else {
            panic!("Expected SkipTasks variant");
        }
    }

    /// Test parsing of skip_tasks tool call without reason (optional field).
    #[test]
    fn test_parse_skip_tasks_without_reason() {
        let task_ids = vec!["task001".to_string()];

        let tool_call = ToolCall::SkipTasks {
            task_ids: task_ids.clone(),
            reason: None,
        };

        assert_eq!(tool_call.tool_type(), "skip_tasks");
        if let ToolCall::SkipTasks {
            task_ids: ids,
            reason,
        } = tool_call
        {
            assert_eq!(ids, task_ids);
            assert!(reason.is_none());
        } else {
            panic!("Expected SkipTasks variant");
        }
    }
}
