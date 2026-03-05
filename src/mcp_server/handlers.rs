//! JSON-RPC request handlers for the MCP server.

use super::error::{
    internal_error, invalid_params_error, invalid_request_error, method_not_found_error,
};
use super::socket::send_request_and_wait;
use super::types::{ToolCall, ToolRequest};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Handle a JSON-RPC request
pub(crate) async fn handle_request(
    request: &Value,
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
            // Get agent type from environment to filter available tools
            let agent_type = std::env::var("VILLALOBOS_AGENT_TYPE").unwrap_or_else(|_| "orchestrator".to_string());

            let tools = match agent_type.as_str() {
                "planner" => {
                    // Planner gets write_plan (to submit the plan) and complete (to signal done)
                    json!({
                        "tools": [
                            {
                                "name": "write_plan",
                                "description": "<usecase>REQUIRED: Submit your structured plan.</usecase>\n<instructions>You MUST call this tool to submit your plan. The plan should be clear, structured markdown with numbered tasks. Each task should have a title, description, and any relevant context. This is the ONLY way to pass your plan to the orchestrator - do NOT just output text.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "plan": {
                                            "type": "string",
                                            "description": "The structured plan in markdown format"
                                        }
                                    },
                                    "required": ["plan"]
                                }
                            },
                            {
                                "name": "complete",
                                "description": "<usecase>REQUIRED: Signal that you have finished your work.</usecase>\n<instructions>You MUST call this tool AFTER calling write_plan. This signals to the orchestration system that your planning work is done.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "success": {
                                            "type": "boolean",
                                            "description": "Whether the planning was completed successfully"
                                        },
                                        "message": {
                                            "type": "string",
                                            "description": "Brief summary of the plan"
                                        }
                                    },
                                    "required": ["success"]
                                }
                            }
                        ]
                    })
                }
                "implementer" => {
                    // Implementer only gets the complete tool to signal they're done
                    json!({
                        "tools": [
                            {
                                "name": "complete",
                                "description": "<usecase>REQUIRED: Signal that you have finished your work.</usecase>\n<instructions>You MUST call this tool when you have completed your assigned task. This signals to the orchestration system that your work is done. Set success=true and include a brief summary of what you accomplished. Call this IMMEDIATELY when you finish - do not wait for user input.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "success": {
                                            "type": "boolean",
                                            "description": "Whether the task was completed successfully"
                                        },
                                        "message": {
                                            "type": "string",
                                            "description": "Brief summary of what was accomplished"
                                        }
                                    },
                                    "required": ["success"]
                                }
                            }
                        ]
                    })
                }
                _ => {
                    // Orchestrator gets all tools
                    json!({
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
                    })
                }
            };

            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tools
            })))
        }
        "tools/call" => handle_tool_call(request, id, socket_path).await,
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
        "write_plan" => match arguments.get("plan").and_then(|v| v.as_str()) {
            Some(plan) => ToolCall::WritePlan {
                plan: plan.to_string(),
            },
            None => {
                tracing::warn!("⚠️  write_plan tool missing 'plan' argument");
                return Ok(Some(invalid_params_error(
                    id,
                    "write_plan",
                    "requires 'plan' string argument",
                )));
            }
        },
        _ => {
            tracing::warn!("⚠️  Unknown tool requested: {}", name);
            return Ok(Some(method_not_found_error(
                id,
                name,
                &["decompose", "implement", "complete", "write_plan"],
            )));
        }
    };

    eprintln!("🔧 MCP Tool call: {:?}", tool_call);

    // Create request with unique ID for correlation
    let request_id = uuid::Uuid::new_v4().to_string();
    let tool_request = ToolRequest {
        request_id: request_id.clone(),
        tool_call: tool_call.clone(),
    };

    // Send request and wait for response from the app
    let response = match send_request_and_wait(socket_path, &tool_request).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to get response from app: {}", e);
            return Ok(Some(internal_error(
                id,
                "socket communication",
                &e.to_string(),
            )));
        }
    };

    // Build response text based on actual result from the app
    let response_text = build_response_text(&tool_call, &response);

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

/// Build a helpful response message based on the tool call and app response
fn build_response_text(
    tool_call: &ToolCall,
    response: &super::types::ToolResponse,
) -> String {
    match tool_call {
        ToolCall::Decompose { task } => {
            if response.success {
                format!(
                    "✅ Decomposition complete for: \"{}\"\n\n\
                     ## Summary\n\
                     {}\n\n\
                     ## Next Steps\n\
                     The subtasks have been planned and executed. \
                     Continue with any remaining tasks or call complete() when done.",
                    task, response.summary
                )
            } else {
                format!(
                    "❌ Decomposition failed for: \"{}\"\n\n\
                     ## Error\n\
                     {}\n\n\
                     ## Next Steps\n\
                     Review the error and decide whether to retry or call complete(success=false).",
                    task,
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
        ToolCall::Implement { task } => {
            if response.success {
                let files_section = if let Some(files) = &response.files_modified {
                    if files.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\n\n## Files Modified\n{}",
                            files.iter().map(|f| format!("- {}", f)).collect::<Vec<_>>().join("\n")
                        )
                    }
                } else {
                    String::new()
                };

                format!(
                    "✅ Implementation complete: \"{}\"\n\n\
                     ## Summary\n\
                     {}{}\n\n\
                     ## Next Steps\n\
                     If you have more independent tasks, call implement() for each. \
                     You can call multiple implement() tools in the same turn to run them concurrently. \
                     When all work is done, call complete(success=true).",
                    task, response.summary, files_section
                )
            } else {
                format!(
                    "❌ Implementation failed: \"{}\"\n\n\
                     ## Error\n\
                     {}\n\n\
                     ## Next Steps\n\
                     Review the error and decide whether to retry, decompose the task, \
                     or call complete(success=false).",
                    task,
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
        ToolCall::Complete { success, message } => {
            if *success {
                format!(
                    "✅ All tasks completed successfully!\n\n\
                     ## Summary\n\
                     {}",
                    message.as_deref().unwrap_or("Work finished")
                )
            } else {
                format!(
                    "⚠️ Tasks completed with issues.\n\n\
                     ## Details\n\
                     {}",
                    message.as_deref().unwrap_or("Some tasks encountered problems")
                )
            }
        }
        ToolCall::WritePlan { plan: _ } => {
            if response.success {
                "✅ Plan submitted successfully. Now call complete(success=true) to finish.".to_string()
            } else {
                format!(
                    "❌ Failed to submit plan: {}",
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
    }
}

