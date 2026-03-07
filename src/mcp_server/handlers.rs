//! JSON-RPC request handlers for the MCP server.

use super::error::{
    internal_error, invalid_params_error, invalid_request_error, method_not_found_error,
};
use super::socket::send_request_and_wait;
use super::types::{AgentSpec, ToolCall, ToolRequest, WaitMode};
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
        "initialize" => {
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

            Ok(Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            })))
        }
        "tools/list" => {
            // Get agent type from environment to filter available tools
            let agent_type = std::env::var("PAPERBOAT_AGENT_TYPE")
                .unwrap_or_else(|_| "orchestrator".to_string());

            let tools = match agent_type.as_str() {
                "planner" => {
                    // Planner gets set_goal, create_task, and complete
                    json!({
                        "tools": [
                            {
                                "name": "set_goal",
                                "description": "<usecase>Define the goal and success criteria before creating tasks.</usecase>\n<instructions>Call this FIRST to establish what success looks like. This helps the orchestrator verify that the work achieves the user's actual goal, not just completes tasks.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "summary": {
                                            "type": "string",
                                            "description": "A concise summary of the user's goal (1-2 sentences)"
                                        },
                                        "acceptance_criteria": {
                                            "type": "string",
                                            "description": "What must be true for the goal to be considered achieved (success conditions)"
                                        }
                                    },
                                    "required": ["summary"]
                                }
                            },
                            {
                                "name": "create_task",
                                "description": "<usecase>Add a task to the plan.</usecase>\n<instructions>Call once per task. Each task will be executed by a separate agent.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "name": {
                                            "type": "string",
                                            "description": "Short task name (e.g., 'Create user model', 'Add login endpoint')"
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "What the implementer agent should do. Include requirements, decisions and contracts. Avoid implementation details."
                                        },
                                        "dependencies": {
                                            "type": "array",
                                            "items": {
                                                "type": "string"
                                            },
                                            "description": "Tasks that must complete before this one (by ID like 'task001' or by name). Tasks without dependencies can run in parallel."
                                        }
                                    },
                                    "required": ["name", "description"]
                                }
                            },
                            {
                                "name": "complete",
                                "description": "<usecase>Signal that planning is finished.</usecase>\n<instructions>Call after setting the goal and creating all tasks. The orchestrator will then execute the plan.</instructions>",
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
                                "description": "<usecase>Signal that your task is finished.</usecase>\n<instructions>Call this after completing your assigned work. The orchestrator is waiting for this signal to proceed. Use 'notes' to leave context for other agents or the orchestrator. Use 'add_tasks' to create new tasks for work you discovered was needed but is outside your scope.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "success": {
                                            "type": "boolean",
                                            "description": "true if task completed successfully, false if it failed"
                                        },
                                        "message": {
                                            "type": "string",
                                            "description": "Brief summary of what you did"
                                        },
                                        "notes": {
                                            "type": "string",
                                            "description": "Optional context for future agents: insights, decisions made, warnings, or things to watch out for"
                                        },
                                        "add_tasks": {
                                            "type": "array",
                                            "description": "Optional tasks to add to the plan. Use for work you discovered was needed but didn't do (outside scope, or should be done later). These become available for the orchestrator to execute.",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "name": { "type": "string", "description": "Short name for the task" },
                                                    "description": { "type": "string", "description": "What needs to be done" },
                                                    "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Task names or IDs this depends on" }
                                                },
                                                "required": ["name", "description"]
                                            }
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
                    // Build dynamic descriptions with auto-discovered roles
                    let roles_list = crate::agents::SPAWNABLE_ROLES.join(", ");
                    let agents_desc = format!(
                        "List of agents to spawn. Use task_id to reference planned tasks, or provide role+task for ad-hoc agents. Available roles: {roles_list} + 'custom' (requires prompt+tools).",
                    );
                    let role_desc = format!(
                        "Agent type. Built-in roles: {roles_list}. Use 'custom' for agents with custom prompt+tools.",
                    );

                    json!({
                        "tools": [
                            {
                                "name": "decompose",
                                "description": "<usecase>Delegates a complex sub-goal to a child orchestrator that plans and executes it autonomously.</usecase>\n<instructions>Use when a task is complex enough to warrant its own planning and orchestration cycle. This spawns a planner to create subtasks, then a child orchestrator to execute them. Returns only after the entire sub-goal is complete. Useful for modular work that should be handled independently (e.g., 'implement the authentication system'). Use task_id to reference a planned task, or task for an explicit description.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "task_id": {
                                            "type": "string",
                                            "description": "Task ID (e.g., 'task001') from the plan. The task description is looked up automatically."
                                        },
                                        "task": {
                                            "type": "string",
                                            "description": "Explicit task description. Use when not referencing a planned task."
                                        }
                                    }
                                }
                            },
                            {
                                "name": "spawn_agents",
                                "description": "<usecase>Delegates tasks to agents who will complete the actual work.</usecase>\n<instructions>Spawn agents by task_id (preferred) or by explicit task description. Each agent has access to file editing, code search, and other development tools. Agents without dependencies can be spawned together for parallel execution.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "agents": {
                                            "type": "array",
                                            "description": agents_desc,
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "task_id": { "type": "string", "description": "Task ID (e.g., 'task001') from the plan. Preferred way to spawn planned tasks - description and role are resolved automatically." },
                                                    "role": { "type": "string", "default": "implementer", "description": role_desc },
                                                    "task": { "type": "string", "description": "Task description. Required for ad-hoc agents (not using task_id)." },
                                                    "prompt": { "type": "string", "description": "Custom prompt. Required when role='custom'." },
                                                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Optional tool whitelist for custom agents. If omitted, all default tools are enabled. Available: str-replace-editor, save-file, remove-files, launch-process, kill-process, read-process, write-process, list-processes, web-search, web-fetch, view, codebase-retrieval." }
                                                }
                                            }
                                        },
                                        "wait": {
                                            "type": "string",
                                            "enum": ["all", "any", "none"],
                                            "default": "all",
                                            "description": "'all' waits for all agents, 'any' returns when first completes, 'none' returns immediately"
                                        }
                                    },
                                    "required": ["agents"]
                                }
                            },
                            {
                                "name": "complete",
                                "description": "<usecase>Marks your orchestration work as finished and returns control to the user.</usecase>\n<instructions>Call this only after all tasks have been delegated (via decompose or implement) and the work as been verified. Set success=true if all work completed successfully, success=false if there were failures. Include a brief summary message describing what was accomplished.</instructions>",
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
                            },
                            {
                                "name": "create_task",
                                "description": "<usecase>Add a new task to the plan dynamically.</usecase>\n<instructions>Use this when agents suggest new tasks via add_tasks, or when you identify additional work needed to achieve the goal. The new task becomes available for execution.</instructions>",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "name": {
                                            "type": "string",
                                            "description": "Short name for the task"
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "Detailed description of what needs to be done"
                                        },
                                        "dependencies": {
                                            "type": "array",
                                            "items": { "type": "string" },
                                            "description": "Task names or IDs that must complete before this one"
                                        }
                                    },
                                    "required": ["name", "description"]
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
        "decompose" => {
            let task_id = arguments
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            let task = arguments
                .get("task")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);

            // Require at least one of task_id or task
            if task_id.is_none() && task.is_none() {
                tracing::warn!("⚠️  decompose tool missing 'task_id' or 'task' argument");
                return Ok(Some(invalid_params_error(
                    id.as_ref(),
                    "decompose",
                    "requires either 'task_id' or 'task' argument",
                )));
            }

            ToolCall::Decompose { task_id, task }
        }
        "spawn_agents" => {
            if let Some(agents_val) = arguments.get("agents") {
                let agents: Vec<AgentSpec> = match serde_json::from_value(agents_val.clone()) {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::warn!("⚠️  spawn_agents invalid 'agents' format: {}", e);
                        return Ok(Some(invalid_params_error(
                            id.as_ref(),
                            "spawn_agents",
                            "requires 'agents' array of {role, task} objects",
                        )));
                    }
                };

                let wait: WaitMode = arguments
                    .get("wait")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                ToolCall::SpawnAgents { agents, wait }
            } else {
                tracing::warn!("⚠️  spawn_agents tool missing 'agents' argument");
                return Ok(Some(invalid_params_error(
                    id.as_ref(),
                    "spawn_agents",
                    "requires 'agents' array argument",
                )));
            }
        }
        "complete" => {
            if let Some(success) = arguments
                .get("success")
                .and_then(serde_json::Value::as_bool)
            {
                let message = arguments
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                let notes = arguments
                    .get("notes")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string);
                let add_tasks = arguments
                    .get("add_tasks")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                ToolCall::Complete {
                    success,
                    message,
                    notes,
                    add_tasks,
                }
            } else {
                tracing::warn!("⚠️  complete tool missing 'success' argument");
                return Ok(Some(invalid_params_error(
                    id.as_ref(),
                    "complete",
                    "requires 'success' boolean argument",
                )));
            }
        }
        "create_task" => {
            let name_arg = if let Some(n) = arguments.get("name").and_then(|v| v.as_str()) {
                n.to_string()
            } else {
                tracing::warn!("⚠️  create_task tool missing 'name' argument");
                return Ok(Some(invalid_params_error(
                    id.as_ref(),
                    "create_task",
                    "requires 'name' string argument",
                )));
            };

            let description = if let Some(d) = arguments.get("description").and_then(|v| v.as_str())
            {
                d.to_string()
            } else {
                tracing::warn!("⚠️  create_task tool missing 'description' argument");
                return Ok(Some(invalid_params_error(
                    id.as_ref(),
                    "create_task",
                    "requires 'description' string argument",
                )));
            };

            let dependencies = arguments
                .get("dependencies")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            ToolCall::CreateTask {
                name: name_arg,
                description,
                dependencies,
            }
        }
        "set_goal" => {
            let summary = if let Some(s) = arguments.get("summary").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                tracing::warn!("⚠️  set_goal tool missing 'summary' argument");
                return Ok(Some(invalid_params_error(
                    id.as_ref(),
                    "set_goal",
                    "requires 'summary' string argument",
                )));
            };

            let acceptance_criteria = arguments
                .get("acceptance_criteria")
                .and_then(|v| v.as_str())
                .map(String::from);

            ToolCall::SetGoal {
                summary,
                acceptance_criteria,
            }
        }
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
    let response = match send_request_and_wait(socket_path, &tool_request).await {
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
fn build_response_text(tool_call: &ToolCall, response: &super::types::ToolResponse) -> String {
    match tool_call {
        ToolCall::Decompose { task_id, task } => {
            let task_desc = task
                .as_deref()
                .or(task_id.as_deref())
                .unwrap_or("(unknown task)");
            if response.success {
                format!(
                    "✅ Decomposition complete for: \"{}\"\n\n\
                     ## Summary\n\
                     {}\n\n\
                     ## Next Steps\n\
                     The subtasks have been planned and executed. \
                     Continue with any remaining tasks or call complete() when done.",
                    task_desc, response.summary
                )
            } else {
                format!(
                    "❌ Decomposition failed for: \"{}\"\n\n\
                     ## Error\n\
                     {}\n\n\
                     ## Next Steps\n\
                     Review the error and decide whether to retry or call complete(success=false).",
                    task_desc,
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
        ToolCall::SpawnAgents { agents, wait } => {
            let agent_count = agents.len();
            let roles: Vec<String> = agents
                .iter()
                .map(|a| a.role.clone().unwrap_or_else(|| "implementer".to_string()))
                .collect();

            if response.success {
                let files_section = if let Some(files) = &response.files_modified {
                    if files.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\n\n## Files Modified\n{}",
                            files
                                .iter()
                                .map(|f| format!("- {f}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    }
                } else {
                    String::new()
                };

                format!(
                    "✅ Spawned {} agent(s) [{:?}] (wait={:?}) completed successfully.\n\n\
                     ## Summary\n\
                     {}{}\n\n\
                     ## Next Steps\n\
                     If you have more independent tasks, call spawn_agents() for each batch. \
                     When all work is done, call complete(success=true).",
                    agent_count, roles, wait, response.summary, files_section
                )
            } else {
                format!(
                    "❌ Spawned {} agent(s) [{:?}] failed.\n\n\
                     ## Error\n\
                     {}\n\n\
                     ## Next Steps\n\
                     Review the error and decide whether to retry, decompose the task, \
                     or call complete(success=false).",
                    agent_count,
                    roles,
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
        ToolCall::Complete {
            success, message, ..
        } => {
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
                    message
                        .as_deref()
                        .unwrap_or("Some tasks encountered problems")
                )
            }
        }
        ToolCall::CreateTask { name, .. } => {
            if response.success {
                format!("✅ Task '{name}' created successfully.")
            } else {
                format!(
                    "❌ Failed to create task '{}': {}",
                    name,
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
        ToolCall::SetGoal { summary, .. } => {
            if response.success {
                format!(
                    "✅ Goal set: {summary}\n\nNow create tasks to achieve this goal.",
                )
            } else {
                format!(
                    "❌ Failed to set goal: {}",
                    response.error.as_deref().unwrap_or("Unknown error")
                )
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
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

        // Planner should NOT have: spawn_agents, decompose
        assert!(
            !tool_names.contains("spawn_agents"),
            "Planner should NOT have spawn_agents tool"
        );
        assert!(
            !tool_names.contains("decompose"),
            "Planner should NOT have decompose tool"
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

        // Orchestrator should have: decompose, spawn_agents, complete, create_task
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

        // Verify exact count
        assert_eq!(
            tool_names.len(),
            4,
            "Orchestrator should have exactly 4 tools, got: {tool_names:?}",
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
    /// This ensures handlers.rs stays in sync with agents/config.rs.
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
}
