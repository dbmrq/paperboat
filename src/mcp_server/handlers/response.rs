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
        ToolCall::ReportHumanAction { description, .. } => {
            build_report_human_action_response(description, response)
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

fn build_report_human_action_response(description: &str, response: &ToolResponse) -> String {
    if response.success {
        let preview = if description.len() > 60 {
            format!("{}...", &description[..57])
        } else {
            description.to_string()
        };
        format!(
            "✅ Human action recorded: \"{preview}\"\n\n\
             This will be displayed prominently at the end of the run."
        )
    } else {
        let error_msg = response.error.as_deref().unwrap_or(&response.summary);
        format!(
            "❌ Failed to record human action: {error_msg}\n\n\
             ## How to Fix\n\
             - Ensure the description is a non-empty string\n\
             - Try calling the tool again with a valid description"
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
        let error_lower = error.to_lowercase();

        // Detect timeout-specific failures for targeted guidance
        let is_timeout = error_lower.contains("timed out") || error_lower.contains("timeout");

        // Detect socket/MCP connection failures - agent may have completed work but failed to signal
        let is_socket_error = error_lower.contains("socket")
            || error_lower.contains("without calling complete")
            || error_lower.contains("mcp socket");

        let how_to_fix = if is_timeout {
            "## How to Fix (Timeout)\n\
             - The agent ran out of time. This often happens with long-running tests or complex tasks.\n\
             - Consider creating smaller, more focused tasks with create_task()\n\
             - For verification tasks, run quick sanity checks instead of full test suites\n\
             - If the task completed meaningful work before timeout, check its notes/results\n\
             - You can retry by spawning another agent for the same task_id"
        } else if is_socket_error {
            "## How to Fix (Socket/Communication Error)\n\
             - The agent likely completed its work but failed to signal completion via MCP socket.\n\
             - Check if the expected files/changes were created - the work may actually be done.\n\
             - Verify the task status with list_tasks() - it may show failure even though work succeeded.\n\
             - Retry the task with spawn_agents() - the retry should succeed quickly if work is already done.\n\
             - If retries keep failing, reduce concurrent agent count to avoid socket resource exhaustion."
        } else {
            "## How to Fix\n\
             - Use list_tasks() to verify the task_id exists and is in 'pending' or 'in_progress' state\n\
             - Review the error message for specific issues in agent execution\n\
             - Try breaking the work into smaller pieces with decompose()\n\
             - If the task requirements are unclear, create clearer subtasks with create_task()\n\
             - If unrecoverable, call complete(success=false) with details about what failed"
        };

        // Add socket-specific cause if detected
        let causes = if is_socket_error {
            "Agent spawning can fail due to:\n\
             - MCP socket connection issues (agent finished work but couldn't signal completion)\n\
             - Invalid task_id reference (task does not exist or is already completed)\n\
             - Resource constraints or timeout"
        } else {
            "Agent spawning can fail due to:\n\
             - Invalid task_id reference (task does not exist or is already completed)\n\
             - Agent execution errors (compilation failures, test failures, etc.)\n\
             - Resource constraints or timeout"
        };

        format!(
            "❌ Spawned {agent_count} agent(s) [{roles:?}] failed.\n\n\
             ## Why This Happened\n\
             {error}\n\n\
             {causes}\n\n\
             {how_to_fix}"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::types::AgentSpec;

    fn success_response(summary: &str) -> ToolResponse {
        ToolResponse::success("req-1".to_string(), summary.to_string())
    }

    fn failure_response(error: &str) -> ToolResponse {
        ToolResponse::failure("req-1".to_string(), error.to_string())
    }

    #[test]
    fn format_whats_next_covers_pending_parallel_blocked_and_complete_states() {
        let cases = [
            (
                "pending with parallel and blocked work",
                TaskStateInfo {
                    pending_count: 3,
                    parallel_tasks: vec!["task004".into(), "task005".into(), "task006".into()],
                    blocked_tasks: vec![(
                        "task007".into(),
                        vec!["task005".into(), "task006".into()],
                    )],
                },
                vec![
                    "3 tasks remaining",
                    "task004 and 2 others have no dependencies",
                    "task007 depends on task005, task006; wait for them first",
                ],
                vec!["All tasks resolved"],
            ),
            (
                "single ready task",
                TaskStateInfo {
                    pending_count: 1,
                    parallel_tasks: vec!["task008".into()],
                    blocked_tasks: vec![],
                },
                vec![
                    "1 task remaining",
                    "task008 is ready to execute (no dependencies)",
                ],
                vec!["All tasks resolved"],
            ),
            (
                "all tasks complete",
                TaskStateInfo::default(),
                vec!["All tasks resolved! Call complete(success=true) to finish."],
                vec!["tasks remaining", "ready to execute"],
            ),
        ];

        for (name, state, expected_substrings, unexpected_substrings) in cases {
            let guidance = state.format_whats_next().expect("guidance should exist");
            for expected in expected_substrings {
                assert!(
                    guidance.contains(expected),
                    "{name}: missing {expected:?} in {guidance:?}"
                );
            }
            for unexpected in unexpected_substrings {
                assert!(
                    !guidance.contains(unexpected),
                    "{name}: unexpectedly found {unexpected:?} in {guidance:?}"
                );
            }
        }
    }

    #[test]
    fn build_decompose_response_guides_next_action_from_task_state() {
        let tool_call = ToolCall::Decompose {
            task_id: Some("task010".to_string()),
            task: None,
        };
        let response = success_response("Created focused subtasks");
        let state = TaskStateInfo {
            pending_count: 2,
            parallel_tasks: vec!["task011".into(), "task012".into()],
            blocked_tasks: vec![],
        };

        let text = build_response_text_with_state(&tool_call, &response, Some(&state));

        assert!(text.contains("Decomposition complete for: \"task010\""));
        assert!(text.contains("Created focused subtasks"));
        assert!(text.contains("## What's Next"));
        assert!(text.contains("2 tasks remaining"));
        assert!(text.contains("task011 and task012 have no dependencies"));
        assert!(!text.contains("Continue with any remaining tasks or call complete() when done."));
    }

    #[test]
    fn build_spawn_agents_success_uses_actionable_state_and_files() {
        let tool_call = ToolCall::SpawnAgents {
            agents: vec![AgentSpec {
                role: Some("reviewer".to_string()),
                task: Some("Review the change".to_string()),
                task_id: Some("task020".to_string()),
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: WaitMode::Any,
        };
        let mut response = success_response("Finished the review pass");
        response.files_modified = Some(vec![
            "src/lib.rs".to_string(),
            "tests/review.rs".to_string(),
        ]);
        let state = TaskStateInfo {
            pending_count: 1,
            parallel_tasks: vec!["task021".into()],
            blocked_tasks: vec![("task022".into(), vec!["task021".into()])],
        };

        let text = build_response_text_with_state(&tool_call, &response, Some(&state));

        assert!(text.contains("Spawned 1 agent(s) [[\"reviewer\"]]"));
        assert!(text.contains("(wait=Any)"));
        assert!(text.contains("Finished the review pass"));
        assert!(text.contains("## Files Modified"));
        assert!(text.contains("- src/lib.rs"));
        assert!(text.contains("- tests/review.rs"));
        assert!(text.contains("1 task remaining"));
        assert!(text.contains("task021 is ready to execute"));
        assert!(text.contains("task022 depends on task021; wait for it first"));
        assert!(!text.contains("If you have more independent tasks"));
    }

    #[test]
    fn build_spawn_agents_failure_provides_targeted_recovery_guidance() {
        let tool_call = ToolCall::SpawnAgents {
            agents: vec![AgentSpec {
                role: None,
                task: Some("Run verification".to_string()),
                task_id: Some("task030".to_string()),
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: WaitMode::All,
        };

        let cases = [
            (
                "timeout",
                failure_response("agent timed out while running tests"),
                vec![
                    "How to Fix (Timeout)",
                    "Consider creating smaller, more focused tasks with create_task()",
                    "check its notes/results",
                ],
            ),
            (
                "socket error",
                failure_response("agent exited without calling complete over mcp socket"),
                vec![
                    "How to Fix (Socket/Communication Error)",
                    "failed to signal completion via MCP socket",
                    "Check if the expected files/changes were created",
                ],
            ),
            (
                "generic error",
                failure_response("task_id task030 was already completed"),
                vec![
                    "## How to Fix",
                    "Use list_tasks() to verify the task_id exists",
                    "If unrecoverable, call complete(success=false)",
                ],
            ),
        ];

        for (name, response, expected_substrings) in cases {
            let text = build_response_text_with_state(&tool_call, &response, None);
            assert!(text.contains("Spawned 1 agent(s)"), "{name}");
            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_skip_tasks_response_uses_completion_hint_when_work_is_done() {
        let tool_call = ToolCall::SkipTasks {
            task_ids: vec!["task040".to_string(), "task041".to_string()],
            reason: Some("covered elsewhere".to_string()),
        };
        let response = success_response("Skipped redundant tasks");
        let state = TaskStateInfo::default();

        let text = build_response_text_with_state(&tool_call, &response, Some(&state));

        assert!(text.contains("Skipped 2 task(s)"));
        assert!(text.contains("covered elsewhere"));
        assert!(text.contains("All tasks resolved! Call complete(success=true) to finish."));
        assert!(!text.contains("Continue with remaining tasks or call complete() when done."));
    }

    #[test]
    fn build_decompose_and_skip_failure_texts_are_actionable() {
        let cases = [
            (
                "decompose failure",
                ToolCall::Decompose {
                    task_id: Some("task050".to_string()),
                    task: None,
                },
                failure_response("Task description was too vague"),
                vec![
                    "Decomposition failed for: \"task050\"",
                    "Use list_tasks() to verify the task exists",
                    "Provide a clearer task description",
                    "call complete(success=false)",
                ],
            ),
            (
                "skip failure",
                ToolCall::SkipTasks {
                    task_ids: vec!["task051".to_string()],
                    reason: None,
                },
                failure_response("task051 is already completed"),
                vec![
                    "Failed to skip tasks: task051 is already completed",
                    "Only pending or in_progress tasks can be skipped",
                    "Reason provided: No reason provided",
                ],
            ),
        ];

        for (name, tool_call, response, expected_substrings) in cases {
            let text = build_response_text_with_state(&tool_call, &response, None);
            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_complete_response_reports_success_and_failure_summaries() {
        let cases = [
            (
                "successful completion",
                ToolCall::Complete {
                    success: true,
                    message: Some("Implemented the requested handler tests".to_string()),
                    notes: None,
                    add_tasks: None,
                },
                success_response("ignored by complete response"),
                vec![
                    "All tasks completed successfully!",
                    "Implemented the requested handler tests",
                ],
                vec!["Tasks completed with issues"],
            ),
            (
                "failed completion",
                ToolCall::Complete {
                    success: false,
                    message: Some("Parser validation is still incomplete".to_string()),
                    notes: None,
                    add_tasks: None,
                },
                failure_response("ignored by complete response"),
                vec![
                    "Tasks completed with issues.",
                    "Parser validation is still incomplete",
                ],
                vec!["All tasks completed successfully!"],
            ),
        ];

        for (name, tool_call, response, expected_substrings, unexpected_substrings) in cases {
            let text = build_response_text_with_state(&tool_call, &response, None);
            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
            for unexpected in unexpected_substrings {
                assert!(
                    !text.contains(unexpected),
                    "{name}: unexpectedly found {unexpected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn format_whats_next_handles_two_parallel_tasks() {
        let state = TaskStateInfo {
            pending_count: 2,
            parallel_tasks: vec!["task001".into(), "task002".into()],
            blocked_tasks: vec![],
        };

        let guidance = state.format_whats_next().expect("should produce guidance");
        assert!(guidance.contains("2 tasks remaining"));
        assert!(guidance.contains("task001 and task002 have no dependencies"));
        assert!(!guidance.contains("others"));
    }

    #[test]
    fn format_whats_next_handles_single_blocker() {
        let state = TaskStateInfo {
            pending_count: 2,
            parallel_tasks: vec!["task001".into()],
            blocked_tasks: vec![("task002".into(), vec!["task001".into()])],
        };

        let guidance = state.format_whats_next().expect("should produce guidance");
        assert!(guidance.contains("task002 depends on task001; wait for it first"));
    }

    #[test]
    fn format_whats_next_handles_multiple_blockers() {
        let state = TaskStateInfo {
            pending_count: 3,
            parallel_tasks: vec![],
            blocked_tasks: vec![(
                "task003".into(),
                vec!["task001".into(), "task002".into()],
            )],
        };

        let guidance = state.format_whats_next().expect("should produce guidance");
        assert!(guidance.contains("task003 depends on task001, task002; wait for them first"));
    }

    #[test]
    fn build_create_task_response_success_and_failure() {
        let cases = [
            (
                "successful creation",
                "implement_feature",
                success_response("Task added to plan"),
                vec!["✅ Task 'implement_feature' created successfully."],
                vec!["Failed", "❌"],
            ),
            (
                "failure due to duplicate",
                "duplicate_task",
                failure_response("A task with this name already exists"),
                vec![
                    "❌ Failed to create task 'duplicate_task'",
                    "A task with this name already exists",
                    "Use list_tasks() to see existing tasks",
                    "call set_goal() first",
                ],
                vec!["created successfully"],
            ),
            (
                "failure due to no goal",
                "orphan_task",
                failure_response("No goal has been set"),
                vec![
                    "Failed to create task 'orphan_task'",
                    "No goal has been set",
                    "If no goal exists, call set_goal() first",
                ],
                vec!["created successfully"],
            ),
        ];

        for (name, task_name, response, expected_substrings, unexpected_substrings) in cases {
            let tool_call = ToolCall::CreateTask {
                name: task_name.to_string(),
                description: "Test description".to_string(),
                dependencies: vec![],
            };
            let text = build_response_text_with_state(&tool_call, &response, None);

            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
            for unexpected in unexpected_substrings {
                assert!(
                    !text.contains(unexpected),
                    "{name}: unexpectedly found {unexpected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_set_goal_response_success_and_failure() {
        let cases = [
            (
                "successful goal set",
                "Improve test coverage",
                success_response("Goal established"),
                vec![
                    "✅ Goal set: Improve test coverage",
                    "Now create tasks to achieve this goal",
                ],
                vec!["Failed", "❌"],
            ),
            (
                "failure due to existing goal",
                "New conflicting goal",
                failure_response("A goal is already active"),
                vec![
                    "❌ Failed to set goal",
                    "A goal is already active",
                    "A goal may already be active",
                    "complete the current work first with complete()",
                ],
                vec!["Goal set:"],
            ),
        ];

        for (name, summary, response, expected_substrings, unexpected_substrings) in cases {
            let tool_call = ToolCall::SetGoal {
                summary: summary.to_string(),
                acceptance_criteria: None,
            };
            let text = build_response_text_with_state(&tool_call, &response, None);

            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
            for unexpected in unexpected_substrings {
                assert!(
                    !text.contains(unexpected),
                    "{name}: unexpectedly found {unexpected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_list_tasks_response_success_and_failure() {
        let cases = [
            (
                "successful list all",
                None,
                success_response("## Tasks\n- task001: pending\n- task002: complete"),
                vec!["## Tasks", "task001: pending", "task002: complete"],
                vec!["Failed", "❌"],
            ),
            (
                "successful list with filter",
                Some("pending"),
                success_response("## Pending Tasks\n- task001"),
                vec!["## Pending Tasks", "task001"],
                vec!["Failed"],
            ),
            (
                "failure no goal",
                None,
                failure_response("No goal has been set yet"),
                vec![
                    "❌ Failed to list tasks (filter=all)",
                    "No goal has been set yet",
                    "Call set_goal() first",
                ],
                vec!["## Tasks"],
            ),
            (
                "failure with filter",
                Some("blocked"),
                failure_response("Invalid filter"),
                vec![
                    "Failed to list tasks (filter=blocked)",
                    "Invalid filter",
                    "Try again with a different status_filter",
                ],
                vec!["## Tasks"],
            ),
        ];

        for (name, filter, response, expected_substrings, unexpected_substrings) in cases {
            let tool_call = ToolCall::ListTasks {
                status_filter: filter.map(String::from),
            };
            let text = build_response_text_with_state(&tool_call, &response, None);

            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
            for unexpected in unexpected_substrings {
                assert!(
                    !text.contains(unexpected),
                    "{name}: unexpectedly found {unexpected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_report_human_action_response_success_and_failure() {
        let cases = [
            (
                "successful short description",
                "User ran database migration",
                success_response("Recorded"),
                vec![
                    "✅ Human action recorded: \"User ran database migration\"",
                    "displayed prominently at the end",
                ],
                vec!["Failed", "❌", "..."],
            ),
            (
                "successful long description truncated",
                "User manually deployed the application to production after verifying all tests passed and getting approval from the team lead",
                success_response("Recorded"),
                vec![
                    "Human action recorded:",
                    "...",
                    "displayed prominently",
                ],
                vec!["Failed", "❌"],
            ),
            (
                "failure",
                "Action that failed",
                failure_response("Description was empty"),
                vec![
                    "❌ Failed to record human action",
                    "Description was empty",
                    "Ensure the description is a non-empty string",
                ],
                vec!["✅"],
            ),
        ];

        for (name, description, response, expected_substrings, unexpected_substrings) in cases {
            let tool_call = ToolCall::ReportHumanAction {
                description: description.to_string(),
                task_id: None,
            };
            let text = build_response_text_with_state(&tool_call, &response, None);

            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
            for unexpected in unexpected_substrings {
                assert!(
                    !text.contains(unexpected),
                    "{name}: unexpectedly found {unexpected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_decompose_response_without_task_state_uses_fallback_guidance() {
        let tool_call = ToolCall::Decompose {
            task_id: Some("task001".to_string()),
            task: None,
        };
        let response = success_response("Created 3 subtasks");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("Decomposition complete for: \"task001\""));
        assert!(text.contains("## Next Steps"));
        assert!(text.contains("Continue with any remaining tasks or call complete() when done"));
        assert!(!text.contains("## What's Next"));
    }

    #[test]
    fn build_decompose_response_uses_task_description_when_task_id_missing() {
        let tool_call = ToolCall::Decompose {
            task_id: None,
            task: Some("Fix the flaky test".to_string()),
        };
        let response = success_response("Analyzed the issue");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("Decomposition complete for: \"Fix the flaky test\""));
    }

    #[test]
    fn build_decompose_response_uses_fallback_when_both_missing() {
        let tool_call = ToolCall::Decompose {
            task_id: None,
            task: None,
        };
        let response = success_response("Decomposed");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("Decomposition complete for: \"(unknown task)\""));
    }

    #[test]
    fn build_spawn_agents_response_without_files_modified() {
        let tool_call = ToolCall::SpawnAgents {
            agents: vec![AgentSpec {
                role: Some("implementer".to_string()),
                task: Some("Write the code".to_string()),
                task_id: None,
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: WaitMode::All,
        };
        let response = success_response("Completed implementation");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("Spawned 1 agent(s)"));
        assert!(text.contains("completed successfully"));
        assert!(!text.contains("## Files Modified"));
    }

    #[test]
    fn build_spawn_agents_response_with_empty_files_modified() {
        let tool_call = ToolCall::SpawnAgents {
            agents: vec![AgentSpec {
                role: None,
                task: Some("Review only".to_string()),
                task_id: None,
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: WaitMode::All,
        };
        let mut response = success_response("Review complete");
        response.files_modified = Some(vec![]);

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(!text.contains("## Files Modified"));
    }

    #[test]
    fn build_spawn_agents_response_default_role_is_implementer() {
        let tool_call = ToolCall::SpawnAgents {
            agents: vec![AgentSpec {
                role: None,
                task: Some("Do work".to_string()),
                task_id: None,
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: WaitMode::All,
        };
        let response = success_response("Done");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("[\"implementer\"]"));
    }

    #[test]
    fn build_skip_tasks_response_without_task_state() {
        let tool_call = ToolCall::SkipTasks {
            task_ids: vec!["task001".to_string()],
            reason: Some("No longer needed".to_string()),
        };
        let response = success_response("Skipped");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("Skipped 1 task(s)"));
        assert!(text.contains("No longer needed"));
        assert!(text.contains("## Next Steps"));
        assert!(text.contains("Continue with remaining tasks or call complete() when done"));
    }

    #[test]
    fn build_skip_tasks_response_failure_shows_task_ids() {
        let tool_call = ToolCall::SkipTasks {
            task_ids: vec!["task001".to_string(), "task002".to_string()],
            reason: None,
        };
        let response = failure_response("task001 not found");

        let text = build_response_text_with_state(&tool_call, &response, None);

        assert!(text.contains("❌ Failed to skip tasks"));
        assert!(text.contains("task001 not found"));
        assert!(text.contains("[\"task001\", \"task002\"]"));
        assert!(text.contains("Reason provided: No reason provided"));
    }

    #[test]
    fn build_complete_response_defaults_when_message_missing() {
        let cases = [
            (
                "success without message",
                ToolCall::Complete {
                    success: true,
                    message: None,
                    notes: None,
                    add_tasks: None,
                },
                success_response("ignored"),
                vec!["All tasks completed successfully!", "Work finished"],
            ),
            (
                "failure without message",
                ToolCall::Complete {
                    success: false,
                    message: None,
                    notes: None,
                    add_tasks: None,
                },
                failure_response("ignored"),
                vec![
                    "Tasks completed with issues",
                    "Some tasks encountered problems",
                ],
            ),
        ];

        for (name, tool_call, response, expected_substrings) in cases {
            let text = build_response_text_with_state(&tool_call, &response, None);
            for expected in expected_substrings {
                assert!(
                    text.contains(expected),
                    "{name}: missing {expected:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn build_response_text_backward_compatible_version() {
        let tool_call = ToolCall::Complete {
            success: true,
            message: Some("Done".to_string()),
            notes: None,
            add_tasks: None,
        };
        let response = success_response("ignored");

        let text = build_response_text(&tool_call, &response);

        assert!(text.contains("All tasks completed successfully!"));
        assert!(text.contains("Done"));
    }

    #[test]
    fn task_state_info_has_remaining_work() {
        let cases = [
            (
                "no work",
                TaskStateInfo::default(),
                false,
            ),
            (
                "pending tasks only",
                TaskStateInfo {
                    pending_count: 3,
                    parallel_tasks: vec![],
                    blocked_tasks: vec![],
                },
                true,
            ),
            (
                "blocked tasks only",
                TaskStateInfo {
                    pending_count: 0,
                    parallel_tasks: vec![],
                    blocked_tasks: vec![("task001".into(), vec!["task000".into()])],
                },
                true,
            ),
            (
                "both pending and blocked",
                TaskStateInfo {
                    pending_count: 2,
                    parallel_tasks: vec!["task001".into()],
                    blocked_tasks: vec![("task002".into(), vec!["task001".into()])],
                },
                true,
            ),
        ];

        for (name, state, expected) in cases {
            assert_eq!(state.has_remaining_work(), expected, "{name}");
        }
    }
}
