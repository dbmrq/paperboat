//! Tool schema definitions for MCP tools.
//!
//! Contains the JSON schema definitions for each tool exposed by the MCP server.

use serde_json::{json, Value};

/// Generate the complete tool definition for the `set_goal` tool.
pub fn set_goal_schema() -> Value {
    json!({
        "name": "set_goal",
        "description": "<usecase>Define the goal and success criteria before creating tasks.</usecase>\n<instructions>Call this FIRST to establish what success looks like. This helps the orchestrator verify that the work achieves the user's actual goal, not just completes tasks.</instructions>\n<on_error>If the goal is rejected as too vague, add specific acceptance_criteria that describe measurable outcomes. If already set, the goal cannot be changed - proceed with task creation instead.</on_error>",
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
    })
}

/// Generate the planner's version of the `create_task` tool definition.
pub fn create_task_schema_planner() -> Value {
    json!({
        "name": "create_task",
        "description": "<usecase>Add a task to the plan.</usecase>\n<instructions>Call once per task. Each task will be executed by a separate agent.</instructions>\n<on_error>If a dependency task_id is not found, use list_tasks() to see existing task IDs. If the description is rejected as too vague, include specific requirements, files to modify, and expected outcomes. If planning has already completed, you cannot add more tasks - ask the orchestrator to use create_task() instead.</on_error>",
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
    })
}

/// Generate the planner's version of the complete tool definition.
pub fn complete_schema_planner() -> Value {
    json!({
        "name": "complete",
        "description": "<usecase>Signal that planning is finished.</usecase>\n<instructions>Call after setting the goal and creating all tasks. The orchestrator will then execute the plan.</instructions>\n<on_error>If completion fails due to missing goal, call set_goal() first. If no tasks were created, create at least one task with create_task() before completing. If there are circular dependencies in your tasks, review and fix the dependency graph.</on_error>",
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
    })
}

/// Generate the implementer's version of the complete tool definition.
pub fn complete_schema_implementer() -> Value {
    json!({
        "name": "complete",
        "description": "<usecase>Signal that your task is finished.</usecase>\n<instructions>Call this after completing your assigned work. The orchestrator is waiting for this signal to proceed. Use 'notes' to leave context for other agents or the orchestrator. Use 'add_tasks' to create new tasks for work you discovered was needed but is outside your scope.</instructions>\n<on_error>If you cannot complete the task, set success=false and explain what went wrong in the message. If you discovered required work outside your scope, use add_tasks to suggest follow-up tasks. If the tool call fails, retry with required fields (success is required).</on_error>",
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
    })
}

/// Generate the orchestrator's version of the complete tool definition.
pub fn complete_schema_orchestrator() -> Value {
    json!({
        "name": "complete",
        "description": "<usecase>Marks your orchestration work as finished and returns control to the user.</usecase>\n<instructions>Call this only after all tasks have been delegated (via decompose or implement) and the work as been verified. Set success=true if all work completed successfully, success=false if there were failures. Include a brief summary message describing what was accomplished.</instructions>\n<on_error>If tasks remain pending, use list_tasks() to review status and spawn_agents() or decompose() to execute them. If some tasks failed, decide whether to retry with spawn_agents(), create recovery tasks with create_task(), skip them with skip_tasks(), or complete with success=false.</on_error>",
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
    })
}

/// Generate the decompose tool definition.
pub fn decompose_schema() -> Value {
    json!({
        "name": "decompose",
        "description": "<usecase>Delegates a complex sub-goal to a child orchestrator that plans and executes it autonomously.</usecase>\n<instructions>Use when a task is complex enough to warrant its own planning and orchestration cycle. This spawns a planner to create subtasks, then a child orchestrator to execute them. Returns only after the entire sub-goal is complete. Useful for modular work that should be handled independently (e.g., 'implement the authentication system'). Always use task_id to reference a task from your task list.</instructions>\n<on_error>If task_id is not found, use list_tasks() to see available task IDs, or create the task first with create_task(). If decomposition fails due to planning errors, consider using spawn_agents() with the 'implementer' role for simpler tasks that don't need their own planning cycle.</on_error>",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID (e.g., 'task001') from the task list. REQUIRED for proper tracking. Create the task first with create_task if needed."
                }
            },
            "required": ["task_id"]
        }
    })
}

/// Generate the `spawn_agents` tool definition with dynamic role descriptions.
pub fn spawn_agents_schema(roles_list: &str) -> Value {
    let agents_desc = format!(
        "List of agents to spawn. ALWAYS use task_id to reference tasks. Create tasks first with create_task if needed. Available roles: {roles_list} + 'custom' (requires prompt+tools).",
    );
    let role_desc = format!(
        "Agent type. Built-in roles: {roles_list}. Use 'custom' for agents with custom prompt+tools.",
    );

    json!({
        "name": "spawn_agents",
        "description": "<usecase>Delegates tasks to agents who will complete the actual work.</usecase>\n<instructions>Spawn agents by task_id. IMPORTANT: Always use task_id to ensure tasks are tracked properly in the UI. If you need to spawn an ad-hoc agent, first create a task with create_task, then spawn with that task_id. Each agent has access to file editing, code search, and other development tools. Agents without dependencies can be spawned together for parallel execution. When the system is in 'auto' model mode, you MUST specify model_complexity for each agent based on your assessment of the task difficulty.</instructions>\n<on_error>If task_id is not found, use list_tasks() to see available task IDs. If you need ad-hoc work, use create_task() first to create a task, then spawn with that task_id. If an agent fails, review its output and consider: creating a recovery task with create_task(), retrying with a different model_complexity, or skipping with skip_tasks() if the work is no longer needed.</on_error>",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agents": {
                    "type": "array",
                    "description": agents_desc,
                    "items": {
                        "type": "object",
                        "properties": {
                            "task_id": { "type": "string", "description": "Task ID (e.g., 'task001') from the task list. REQUIRED for all agents to ensure proper tracking. Create tasks first with create_task if needed." },
                            "role": { "type": "string", "default": "implementer", "description": role_desc },
                            "prompt": { "type": "string", "description": "Custom prompt. Required when role='custom'." },
                            "tools": { "type": "array", "items": { "type": "string" }, "description": "Optional tool whitelist for custom agents. If omitted, all default tools are enabled. Available: str-replace-editor, save-file, remove-files, launch-process, kill-process, read-process, write-process, list-processes, web-search, web-fetch, view, codebase-retrieval." },
                            "model_complexity": { "type": "string", "enum": ["simple", "medium", "complex"], "description": "Model complexity hint for auto mode. Required when system is in 'auto' model mode. 'simple': straightforward edits, small changes. 'medium': typical development tasks. 'complex': architectural decisions, nuanced judgment." }
                        },
                        "required": ["task_id"]
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
    })
}

/// Generate the orchestrator's `create_task` tool definition.
pub fn create_task_schema_orchestrator() -> Value {
    json!({
        "name": "create_task",
        "description": "<usecase>Create a task for any purpose - implementation, exploration, verification, testing, recovery, etc.</usecase>\n<instructions>Tasks can represent ANY unit of work. Use this to:\n- Add implementation tasks for new features or fixes\n- Create exploration tasks to gather context\n- Set up verification tasks to check work quality\n- Define testing tasks to validate behavior\n- Add recovery tasks when something needs fixing\n\nAfter creating a task, spawn an agent with its task_id to execute it. This ensures all work is tracked in the UI.</instructions>\n<on_error>If a dependency task_id is not found, use list_tasks() to see existing task IDs. If task creation fails due to missing fields, ensure both 'name' and 'description' are provided. After creating a task, use spawn_agents() with the returned task_id to execute it.</on_error>",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Short name for the task (e.g., 'Explore auth flow', 'Verify database migrations', 'Fix login bug')"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of what the agent should do"
                },
                "dependencies": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task names or IDs that must complete before this one"
                }
            },
            "required": ["name", "description"]
        }
    })
}

/// Generate the `skip_tasks` tool definition.
pub fn skip_tasks_schema() -> Value {
    json!({
        "name": "skip_tasks",
        "description": "<usecase>Skip one or more tasks that are no longer needed.</usecase>\n<instructions>Use this when tasks become unnecessary (e.g., already done by another task, no longer relevant, or blocked permanently). Skipped tasks will not be executed and will be marked as skipped in the plan.</instructions>\n<on_error>If a task_id is not found, use list_tasks() to see available task IDs. If a task cannot be skipped because it's already completed or in progress, no action is needed - the task will proceed as planned. Provide a reason to help future agents understand why the task was skipped.</on_error>",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs to skip (e.g., [\"task001\", \"task002\"])"
                },
                "reason": {
                    "type": "string",
                    "description": "Explanation for why these tasks are being skipped"
                }
            },
            "required": ["task_ids"]
        }
    })
}

/// Generate the `list_tasks` tool definition.
pub fn list_tasks_schema() -> Value {
    json!({
        "name": "list_tasks",
        "description": "<usecase>Get the current state of all tasks.</usecase>\n<instructions>Use this to check task status when you need to review progress, verify which tasks are pending, or see tasks that were suggested by completed agents. Returns all tasks with their IDs, names, descriptions, and status (pending/in_progress/completed/failed/skipped).</instructions>\n<on_error>If no tasks are returned, the plan may not have been created yet - this is normal at the start of orchestration. Use the status_filter parameter to narrow results (e.g., 'pending' to see only tasks awaiting execution).</on_error>",
        "inputSchema": {
            "type": "object",
            "properties": {
                "status_filter": {
                    "type": "string",
                    "enum": ["all", "pending", "in_progress", "completed", "failed", "skipped"],
                    "default": "all",
                    "description": "Filter tasks by status. Defaults to 'all'."
                }
            }
        }
    })
}
