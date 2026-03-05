//! JSON-RPC request handlers for the MCP server.

use super::error::{
    internal_error, invalid_params_error, invalid_request_error, method_not_found_error,
};
use super::socket::send_to_socket_with_reconnect;
use super::types::ToolCall;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::net::UnixStream;

/// Handle a JSON-RPC request
pub(crate) async fn handle_request(
    request: &Value,
    socket: &mut UnixStream,
    socket_path: &PathBuf,
) -> Result<Option<Value>> {
    let id = request.get("id").cloned();

    // Extract method, returning error response if missing
    let method = match request["method"].as_str() {
        Some(m) => m,
        None => {
            tracing::warn!("⚠️  Request missing 'method' field: {:?}", request);
            return Ok(Some(invalid_request_error(id, "missing 'method' field")));
        }
    };

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "villalobos-orchestrator",
                    "version": "0.1.0"
                }
            });

            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            })))
        }
        "tools/list" => {
            let tools = json!({
                "tools": [
                    {
                        "name": "decompose",
                        "description": "<usecase>Breaks down complex, multi-step tasks into smaller subtasks by spawning a specialized planner agent.</usecase>\n<instructions>Use when a task involves multiple distinct steps, requires different types of work (e.g., backend + frontend + tests), or would take more than one focused implementation session. The planner will create a detailed plan, then you can implement each subtask. Returns a list of subtasks to implement.</instructions>",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "task": {
                                    "type": "string",
                                    "description": "The complex task to break down into implementable subtasks"
                                }
                            },
                            "required": ["task"]
                        }
                    },
                    {
                        "name": "implement",
                        "description": "<usecase>Implements a single, focused task by spawning a specialized implementer agent with full code editing capabilities.</usecase>\n<instructions>Use for atomic tasks that can be completed in one session: adding a function, fixing a bug, writing tests, creating a file, etc. The implementer has access to all code editing tools. After calling this, the task will be completed by the implementer agent.</instructions>",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "task": {
                                    "type": "string",
                                    "description": "Clear description of the single task to implement"
                                }
                            },
                            "required": ["task"]
                        }
                    },
                    {
                        "name": "complete",
                        "description": "<usecase>Marks your orchestration work as finished and returns control to the user.</usecase>\n<instructions>Call this only after all tasks have been delegated (via decompose or implement). Set success=true if all work completed successfully, success=false if there were failures. Include a brief summary message describing what was accomplished.</instructions>",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "success": {
                                    "type": "boolean",
                                    "description": "Whether all delegated tasks completed successfully"
                                },
                                "message": {
                                    "type": "string",
                                    "description": "Brief summary of what was accomplished or what failed"
                                }
                            },
                            "required": ["success"]
                        }
                    }
                ]
            });

            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tools
            })))
        }
        "tools/call" => handle_tool_call(request, id, socket, socket_path).await,
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

/// Handle a tool call with proper error responses
async fn handle_tool_call(
    request: &Value,
    id: Option<Value>,
    socket: &mut UnixStream,
    socket_path: &PathBuf,
) -> Result<Option<Value>> {
    // Validate params structure
    let params = match request["params"].as_object() {
        Some(p) => p,
        None => {
            tracing::warn!("⚠️  tools/call missing 'params' object");
            return Ok(Some(invalid_request_error(
                id,
                "'params' must be an object for tools/call",
            )));
        }
    };

    // Validate tool name
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            tracing::warn!("⚠️  tools/call missing 'name' parameter");
            return Ok(Some(invalid_params_error(
                id,
                "tools/call",
                "missing required 'name' field",
            )));
        }
    };

    // Validate arguments
    let arguments = match params.get("arguments").and_then(|v| v.as_object()) {
        Some(a) => a,
        None => {
            tracing::warn!(
                "⚠️  tools/call missing 'arguments' parameter for tool '{}'",
                name
            );
            return Ok(Some(invalid_params_error(
                id,
                name,
                "missing required 'arguments' field",
            )));
        }
    };

    // Parse the tool call with specific error messages
    let tool_call = match name {
        "decompose" => match arguments.get("task").and_then(|v| v.as_str()) {
            Some(task) => ToolCall::Decompose {
                task: task.to_string(),
            },
            None => {
                tracing::warn!("⚠️  decompose tool missing 'task' argument");
                return Ok(Some(invalid_params_error(
                    id,
                    "decompose",
                    "requires 'task' string argument",
                )));
            }
        },
        "implement" => match arguments.get("task").and_then(|v| v.as_str()) {
            Some(task) => ToolCall::Implement {
                task: task.to_string(),
            },
            None => {
                tracing::warn!("⚠️  implement tool missing 'task' argument");
                return Ok(Some(invalid_params_error(
                    id,
                    "implement",
                    "requires 'task' string argument",
                )));
            }
        },
        "complete" => match arguments.get("success").and_then(|v| v.as_bool()) {
            Some(success) => {
                let message = arguments
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                ToolCall::Complete { success, message }
            }
            None => {
                tracing::warn!("⚠️  complete tool missing 'success' argument");
                return Ok(Some(invalid_params_error(
                    id,
                    "complete",
                    "requires 'success' boolean argument",
                )));
            }
        },
        _ => {
            tracing::warn!("⚠️  Unknown tool requested: {}", name);
            return Ok(Some(method_not_found_error(
                id,
                name,
                &["decompose", "implement", "complete"],
            )));
        }
    };

    eprintln!("🔧 MCP Tool call: {:?}", tool_call);

    // Send the tool call to the app via Unix socket with error handling
    let message = serde_json::to_string(&tool_call)?;
    eprintln!("📨 Sending to app: {}", message);

    if let Err(e) = send_to_socket_with_reconnect(socket, socket_path, &message).await {
        tracing::error!("Failed to send tool call to socket: {}", e);
        return Ok(Some(internal_error(
            id,
            "socket communication",
            &e.to_string(),
        )));
    }

    // Return helpful response that guides the next step
    let response_text = match &tool_call {
        ToolCall::Decompose { task } => {
            format!("✓ Spawned planner agent to break down task: \"{}\"\n\nThe planner will create a detailed plan with subtasks. Once the plan is ready, use implement() for each subtask.", task)
        }
        ToolCall::Implement { task } => {
            format!("✓ Spawned implementer agent for task: \"{}\"\n\nThe implementer has full code editing access and will complete this task. You can continue with other tasks or call complete() when all work is done.", task)
        }
        ToolCall::Complete { success, message } => {
            if *success {
                format!(
                    "✓ All tasks completed successfully!\n\nSummary: {}",
                    message.as_deref().unwrap_or("Work finished")
                )
            } else {
                format!(
                    "✗ Tasks completed with failures.\n\nDetails: {}",
                    message.as_deref().unwrap_or("Some tasks failed")
                )
            }
        }
    };

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

