//! Response text builders for tool calls.
//!
//! Contains functions to build user-friendly response messages from tool call results.
//!
//! ## Context-Aware Responses
//!
//! Response builders support an optional `TaskStateInfo` parameter that enables
//! dynamic "What's Next" guidance based on actual task state. When provided,
//! responses include:
//! - Count of remaining tasks
//! - Tasks that can run in parallel (no dependencies)
//! - Blocked tasks and what they're waiting for
//! - Concrete next action suggestions
//!
//! See `docs/INTENT_BASED_MCP_DESIGN.md` for the design rationale.

use crate::mcp_server::types::{TaskStateInfo, ToolCall, ToolResponse, WaitMode};

impl TaskStateInfo {
    /// Check if there are any remaining tasks to work on.
    #[allow(dead_code)]
    pub const fn has_remaining_work(&self) -> bool {
        self.pending_count > 0 || !self.blocked_tasks.is_empty()
    }

    /// Format the "What's Next" section based on task state.
    ///
    /// Returns a formatted string with actionable guidance, or None if
    /// there's no meaningful guidance to provide (e.g., all tasks complete).
    pub fn format_whats_next(&self) -> Option<String> {
        let mut lines = Vec::new();

        // Show count of remaining tasks
        if self.pending_count > 0 {
            let task_word = if self.pending_count == 1 {
                "task"
            } else {
                "tasks"
            };
            lines.push(format!(
                "- **{} {} remaining**",
                self.pending_count, task_word
            ));
        }

        // Show parallel execution opportunities
        if self.parallel_tasks.len() > 1 {
            lines.push(format!(
                "- {} and {} have no dependencies—spawn them together for parallel execution",
                self.parallel_tasks[0],
                if self.parallel_tasks.len() == 2 {
                    self.parallel_tasks[1].clone()
                } else {
                    format!("{} others", self.parallel_tasks.len() - 1)
                }
            ));
        } else if self.parallel_tasks.len() == 1 {
            lines.push(format!(
                "- {} is ready to execute (no dependencies)",
                self.parallel_tasks[0]
            ));
        }

        // Show blocked tasks
        for (task_id, blockers) in &self.blocked_tasks {
            let blocker_list = blockers.join(", ");
            lines.push(format!(
                "- {} depends on {}; wait for {} first",
                task_id,
                blocker_list,
                if blockers.len() == 1 { "it" } else { "them" }
            ));
        }

        // Provide completion hint if no work remains
        if self.pending_count == 0 && self.blocked_tasks.is_empty() {
            lines.push("- All tasks resolved! Call complete(success=true) to finish.".to_string());
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }
}

/// Build a helpful response message based on the tool call and app response.
///
/// This is the backward-compatible version without task state. Use
/// `build_response_text_with_state` for context-aware responses.
#[allow(dead_code)]
pub fn build_response_text(tool_call: &ToolCall, response: &ToolResponse) -> String {
    build_response_text_with_state(tool_call, response, None)
}

/// Build a helpful response message with optional task state for context-aware guidance.
///
/// When `task_state` is provided, success responses include dynamic "What's Next"
/// sections showing remaining tasks, parallel execution opportunities, and blocked tasks.
///
/// # Arguments
///
/// * `tool_call` - The tool call that was executed
/// * `response` - The response from executing the tool call
/// * `task_state` - Optional snapshot of current task state for context-aware guidance
///
/// # Example
///
/// ```ignore
/// let state = TaskStateInfo {
///     pending_count: 2,
///     parallel_tasks: vec!["task004".into()],
///     blocked_tasks: vec![("task005".into(), vec!["task004".into()])],
/// };
/// let text = build_response_text_with_state(&tool_call, &response, Some(&state));
/// ```
pub fn build_response_text_with_state(
    tool_call: &ToolCall,
    response: &ToolResponse,
    task_state: Option<&TaskStateInfo>,
) -> String {
    match tool_call {
        ToolCall::Decompose { task_id, task } => {
            build_decompose_response(task_id.as_ref(), task.as_ref(), response, task_state)
        }
        ToolCall::SpawnAgents { agents, wait } => {
            build_spawn_agents_response(agents.len(), agents, *wait, response, task_state)
        }
        ToolCall::Complete {
            success, message, ..
        } => build_complete_response(*success, message.as_deref()),
        ToolCall::CreateTask { name, .. } => build_create_task_response(name, response),
        ToolCall::SetGoal { summary, .. } => build_set_goal_response(summary, response),
        ToolCall::SkipTasks { task_ids, reason } => {
            build_skip_tasks_response(task_ids, reason.as_deref(), response, task_state)
        }
        ToolCall::ListTasks { status_filter } => {
            build_list_tasks_response(status_filter.as_deref(), response)
        }
    }
}

fn build_list_tasks_response(status_filter: Option<&str>, response: &ToolResponse) -> String {
    let filter = status_filter.unwrap_or("all");
    if response.success {
        // The response.summary contains the formatted task list
        response.summary.clone()
    } else {
        let error_msg = response.error.as_deref().unwrap_or(&response.summary);
        format!(
            "❌ Failed to list tasks (filter={filter}): {error_msg}\n\n\
             ## Why This Happened\n\
             The task list could not be retrieved. This may occur if:\n\
             - No goal has been set yet (tasks require a goal context)\n\
             - The task database is in an inconsistent state\n\n\
             ## How to Fix\n\
             - Call set_goal() first to establish the goal context\n\
             - Try again with a different status_filter (e.g., 'pending', 'completed', or omit for 'all')\n\
             - If the problem persists, the orchestrator may need to be restarted"
        )
    }
}

fn build_decompose_response(
    task_id: Option<&String>,
    task: Option<&String>,
    response: &ToolResponse,
    task_state: Option<&TaskStateInfo>,
) -> String {
    let task_desc = task
        .map(String::as_str)
        .or_else(|| task_id.map(String::as_str))
        .unwrap_or("(unknown task)");
    if response.success {
        // Build dynamic "What's Next" section if task state is available
        let next_steps = task_state
            .and_then(TaskStateInfo::format_whats_next)
            .map_or_else(
                || {
                    "## Next Steps\n\
                 The subtasks have been planned and executed. \
                 Continue with any remaining tasks or call complete() when done."
                        .to_string()
                },
                |guidance| format!("## What's Next\n{guidance}"),
            );

        format!(
            "✅ Decomposition complete for: \"{}\"\n\n\
             ## Summary\n\
             {}\n\n\
             {}",
            task_desc, response.summary, next_steps
        )
    } else {
        let error = response.error.as_deref().unwrap_or("Unknown error");
        format!(
            "❌ Decomposition failed for: \"{task_desc}\"\n\n\
             ## Why This Happened\n\
             {error}\n\n\
             The decomposition could not create or execute subtasks. Common causes:\n\
             - The task_id does not exist or was already completed\n\
             - The task description is unclear or too vague to decompose\n\
             - Sub-agent execution encountered errors\n\n\
             ## How to Fix\n\
             - Use list_tasks() to verify the task exists and check its current status\n\
             - Provide a clearer task description with specific, actionable requirements\n\
             - Try decomposing into fewer, simpler subtasks\n\
             - If the task cannot be completed, call complete(success=false) with an explanation"
        )
    }
}

fn build_spawn_agents_response(
    agent_count: usize,
    agents: &[crate::mcp_server::types::AgentSpec],
    wait: WaitMode,
    response: &ToolResponse,
    task_state: Option<&TaskStateInfo>,
) -> String {
    let roles: Vec<String> = agents
        .iter()
        .map(|a| a.role.clone().unwrap_or_else(|| "implementer".to_string()))
        .collect();

    if response.success {
        let files_section = response
            .files_modified
            .as_ref()
            .filter(|f| !f.is_empty())
            .map(|files| {
                format!(
                    "\n\n## Files Modified\n{}",
                    files
                        .iter()
                        .map(|f| format!("- {f}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
            .unwrap_or_default();

        // Build dynamic "What's Next" section if task state is available
        let next_steps = task_state
            .and_then(TaskStateInfo::format_whats_next)
            .map_or_else(
                || {
                    "## Next Steps\n\
                 If you have more independent tasks, call spawn_agents() for each batch. \
                 When all work is done, call complete(success=true)."
                        .to_string()
                },
                |guidance| format!("## What's Next\n{guidance}"),
            );

        format!(
            "✅ Spawned {} agent(s) [{:?}] (wait={:?}) completed successfully.\n\n\
             ## Summary\n\
             {}{}\n\n\
             {}",
            agent_count, roles, wait, response.summary, files_section, next_steps
        )
    } else {
        let error = response.error.as_deref().unwrap_or("Unknown error");
        format!(
            "❌ Spawned {agent_count} agent(s) [{roles:?}] failed.\n\n\
             ## Why This Happened\n\
             {error}\n\n\
             Agent spawning can fail due to:\n\
             - Invalid task_id reference (task does not exist or is already completed)\n\
             - Agent execution errors (compilation failures, test failures, etc.)\n\
             - Resource constraints or timeout\n\n\
             ## How to Fix\n\
             - Use list_tasks() to verify the task_id exists and is in 'pending' or 'in_progress' state\n\
             - Review the error message for specific issues in agent execution\n\
             - Try breaking the work into smaller pieces with decompose()\n\
             - If the task requirements are unclear, create clearer subtasks with create_task()\n\
             - If unrecoverable, call complete(success=false) with details about what failed"
        )
    }
}

fn build_complete_response(success: bool, message: Option<&str>) -> String {
    if success {
        format!(
            "✅ All tasks completed successfully!\n\n\
             ## Summary\n\
             {}",
            message.unwrap_or("Work finished")
        )
    } else {
        format!(
            "⚠️ Tasks completed with issues.\n\n\
             ## Details\n\
             {}",
            message.unwrap_or("Some tasks encountered problems")
        )
    }
}

fn build_create_task_response(name: &str, response: &ToolResponse) -> String {
    if response.success {
        format!("✅ Task '{name}' created successfully.")
    } else {
        let error = response.error.as_deref().unwrap_or("Unknown error");
        format!(
            "❌ Failed to create task '{name}': {error}\n\n\
             ## Why This Happened\n\
             The task could not be created. Common causes:\n\
             - A task with this name may already exist\n\
             - No goal has been set (tasks must belong to a goal)\n\
             - Invalid task parameters (empty name, invalid parent_id, etc.)\n\n\
             ## How to Fix\n\
             - Use list_tasks() to see existing tasks and avoid duplicate names\n\
             - If no goal exists, call set_goal() first to establish context\n\
             - Verify the parent_id (if provided) exists using list_tasks()\n\
             - Try a different, more descriptive task name"
        )
    }
}

fn build_set_goal_response(summary: &str, response: &ToolResponse) -> String {
    if response.success {
        format!("✅ Goal set: {summary}\n\nNow create tasks to achieve this goal.",)
    } else {
        let error = response.error.as_deref().unwrap_or("Unknown error");
        format!(
            "❌ Failed to set goal: {error}\n\n\
             ## Why This Happened\n\
             The goal could not be set. Possible reasons:\n\
             - A goal may already be active (only one goal at a time)\n\
             - The goal summary may be empty or invalid\n\
             - Internal state error\n\n\
             ## How to Fix\n\
             - Use list_tasks() to check if a goal already exists\n\
             - Ensure the goal summary is non-empty and descriptive\n\
             - If a goal exists and you need to change it, complete the current work first with complete()"
        )
    }
}

fn build_skip_tasks_response(
    task_ids: &[String],
    reason: Option<&str>,
    response: &ToolResponse,
    task_state: Option<&TaskStateInfo>,
) -> String {
    let task_count = task_ids.len();
    let reason_str = reason.unwrap_or("No reason provided");
    if response.success {
        // Build dynamic "What's Next" section if task state is available
        let next_steps = task_state
            .and_then(TaskStateInfo::format_whats_next)
            .map_or_else(
                || {
                    "## Next Steps\n\
                     Continue with remaining tasks or call complete() when done."
                        .to_string()
                },
                |guidance| format!("## What's Next\n{guidance}"),
            );

        format!(
            "✅ Skipped {task_count} task(s): {task_ids:?}\n\n\
             ## Reason\n\
             {reason_str}\n\n\
             {next_steps}"
        )
    } else {
        let error = response.error.as_deref().unwrap_or("Unknown error");
        format!(
            "❌ Failed to skip tasks: {error}\n\n\
             ## Why This Happened\n\
             One or more tasks could not be skipped. Common causes:\n\
             - Task ID(s) do not exist: {task_ids:?}\n\
             - Task(s) are already completed or already skipped\n\
             - Task(s) have dependent tasks that would be orphaned\n\n\
             ## How to Fix\n\
             - Use list_tasks() to verify the task_ids exist and check their current status\n\
             - Only pending or in_progress tasks can be skipped\n\
             - Ensure you're using the correct task_id format (UUID)\n\
             - Reason provided: {reason_str}"
        )
    }
}
