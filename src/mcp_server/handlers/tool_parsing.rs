//! Tool call parsing functions.
//!
//! Contains functions to parse tool call arguments from JSON into `ToolCall` variants.

use crate::mcp_server::types::{AgentSpec, ToolCall, WaitMode};
use serde_json::Value;

/// Result of parsing a tool call - either success with `ToolCall` or an error message.
pub type ParseResult = Result<ToolCall, &'static str>;

/// Parse the "decompose" tool call arguments.
pub fn parse_decompose(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    let task_id = arguments
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Legacy: still accept 'task' but log a warning
    let task = arguments
        .get("task")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Require task_id (preferred) or fall back to task (deprecated)
    if task_id.is_none() && task.is_none() {
        tracing::warn!("⚠️  decompose tool missing 'task_id' argument");
        return Err(
            "requires 'task_id' argument. Create the task first with create_task if needed.",
        );
    }

    if task_id.is_none() && task.is_some() {
        tracing::warn!("⚠️  decompose using deprecated 'task' argument. Use 'task_id' instead.");
    }

    Ok(ToolCall::Decompose { task_id, task })
}

/// Parse the `spawn_agents` tool call arguments.
pub fn parse_spawn_agents(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    if let Some(agents_val) = arguments.get("agents") {
        let agents: Vec<AgentSpec> = match serde_json::from_value(agents_val.clone()) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("⚠️  spawn_agents invalid 'agents' format: {}", e);
                return Err("requires 'agents' array of {task_id, role} objects");
            }
        };

        // Warn about agents using deprecated 'task' field instead of 'task_id'
        for (i, agent) in agents.iter().enumerate() {
            if agent.task_id.is_none() && agent.task.is_some() {
                tracing::warn!(
                    "⚠️  spawn_agents agent[{}] using deprecated 'task' field. \
                     Use 'task_id' instead for proper tracking. \
                     Create the task first with create_task if needed.",
                    i
                );
            }
        }

        let wait: WaitMode = arguments
            .get("wait")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(ToolCall::SpawnAgents { agents, wait })
    } else {
        tracing::warn!("⚠️  spawn_agents tool missing 'agents' argument");
        Err("requires 'agents' array argument")
    }
}

/// Parse the "complete" tool call arguments.
pub fn parse_complete(arguments: &serde_json::Map<String, Value>) -> ParseResult {
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
        Ok(ToolCall::Complete {
            success,
            message,
            notes,
            add_tasks,
        })
    } else {
        tracing::warn!("⚠️  complete tool missing 'success' argument");
        Err("requires 'success' boolean argument")
    }
}

/// Parse the `create_task` tool call arguments.
pub fn parse_create_task(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    let name = if let Some(n) = arguments.get("name").and_then(|v| v.as_str()) {
        n.to_string()
    } else {
        tracing::warn!("⚠️  create_task tool missing 'name' argument");
        return Err("requires 'name' string argument");
    };

    let description = if let Some(d) = arguments.get("description").and_then(|v| v.as_str()) {
        d.to_string()
    } else {
        tracing::warn!("⚠️  create_task tool missing 'description' argument");
        return Err("requires 'description' string argument");
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

    Ok(ToolCall::CreateTask {
        name,
        description,
        dependencies,
    })
}

/// Parse the `set_goal` tool call arguments.
pub fn parse_set_goal(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    let summary = if let Some(s) = arguments.get("summary").and_then(|v| v.as_str()) {
        s.to_string()
    } else {
        tracing::warn!("⚠️  set_goal tool missing 'summary' argument");
        return Err("requires 'summary' string argument");
    };

    let acceptance_criteria = arguments
        .get("acceptance_criteria")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(ToolCall::SetGoal {
        summary,
        acceptance_criteria,
    })
}

/// Parse the `skip_tasks` tool call arguments.
pub fn parse_skip_tasks(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    let task_ids = if let Some(arr) = arguments.get("task_ids").and_then(|v| v.as_array()) {
        let ids: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if ids.is_empty() {
            tracing::warn!("⚠️  skip_tasks tool has empty 'task_ids' array");
            return Err("'task_ids' array must contain at least one task ID");
        }
        ids
    } else {
        tracing::warn!("⚠️  skip_tasks tool missing 'task_ids' argument");
        return Err("requires 'task_ids' array argument");
    };

    let reason = arguments
        .get("reason")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(ToolCall::SkipTasks { task_ids, reason })
}

/// Parse the `list_tasks` tool call arguments.
#[allow(clippy::unnecessary_wraps)]
pub fn parse_list_tasks(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    let status_filter = arguments
        .get("status_filter")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(ToolCall::ListTasks { status_filter })
}

/// Parse the `report_human_action` tool call arguments.
pub fn parse_report_human_action(arguments: &serde_json::Map<String, Value>) -> ParseResult {
    let description = if let Some(d) = arguments.get("description").and_then(|v| v.as_str()) {
        d.to_string()
    } else {
        tracing::warn!("⚠️  report_human_action tool missing 'description' argument");
        return Err("requires 'description' string argument");
    };

    let task_id = arguments
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(ToolCall::ReportHumanAction {
        description,
        task_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().cloned().expect("expected JSON object")
    }

    #[test]
    fn parse_decompose_supports_preferred_and_deprecated_inputs() {
        let cases = [
            (
                "preferred task_id",
                json!({ "task_id": "task001" }),
                Ok((Some("task001"), None)),
            ),
            (
                "deprecated task fallback",
                json!({ "task": "Investigate flaky test" }),
                Ok((None, Some("Investigate flaky test"))),
            ),
            (
                "missing both arguments",
                json!({}),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
            (
                "invalid task_id type without fallback",
                json!({ "task_id": 42 }),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_decompose(&object(arguments));
            match (parsed, expected) {
                (Ok(ToolCall::Decompose { task_id, task }), Ok((expected_id, expected_task))) => {
                    assert_eq!(task_id.as_deref(), expected_id, "{name}");
                    assert_eq!(task.as_deref(), expected_task, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_spawn_agents_validates_agents_payload_and_wait_mode() {
        let cases = [
            (
                "task_id based agent with explicit wait",
                json!({
                    "agents": [
                        { "task_id": "task001", "role": "implementer" }
                    ],
                    "wait": "Any"
                }),
                Ok((
                    1usize,
                    WaitMode::Any,
                    Some("task001"),
                    None,
                    Some("implementer"),
                )),
            ),
            (
                "deprecated task fallback is still accepted",
                json!({
                    "agents": [
                        { "task": "Write focused tests" }
                    ]
                }),
                Ok((
                    1usize,
                    WaitMode::All,
                    None,
                    Some("Write focused tests"),
                    None,
                )),
            ),
            (
                "invalid wait falls back to default",
                json!({
                    "agents": [
                        { "task_id": "task002" }
                    ],
                    "wait": "later"
                }),
                Ok((1usize, WaitMode::All, Some("task002"), None, None)),
            ),
            (
                "missing agents",
                json!({ "wait": "none" }),
                Err("requires 'agents' array argument"),
            ),
            (
                "agents must deserialize cleanly",
                json!({
                    "agents": [
                        { "task_id": "task003" },
                        { "task_id": 99, "role": "implementer" }
                    ]
                }),
                Err("requires 'agents' array of {task_id, role} objects"),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_spawn_agents(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::SpawnAgents { agents, wait }),
                    Ok((
                        agent_count,
                        expected_wait,
                        expected_task_id,
                        expected_task,
                        expected_role,
                    )),
                ) => {
                    assert_eq!(agents.len(), agent_count, "{name}");
                    assert_eq!(wait, expected_wait, "{name}");
                    let first = &agents[0];
                    assert_eq!(first.task_id.as_deref(), expected_task_id, "{name}");
                    assert_eq!(first.task.as_deref(), expected_task, "{name}");
                    assert_eq!(first.role.as_deref(), expected_role, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_skip_tasks_requires_non_empty_string_ids() {
        let cases = [
            (
                "valid ids with reason",
                json!({
                    "task_ids": ["task001", "task002"],
                    "reason": "covered by upstream refactor"
                }),
                Ok((
                    vec!["task001", "task002"],
                    Some("covered by upstream refactor"),
                )),
            ),
            (
                "mixed id types keep valid task ids",
                json!({
                    "task_ids": ["task003", 12, true],
                    "reason": "manual follow-up"
                }),
                Ok((vec!["task003"], Some("manual follow-up"))),
            ),
            (
                "empty array is rejected",
                json!({ "task_ids": [] }),
                Err("'task_ids' array must contain at least one task ID"),
            ),
            (
                "non-string ids only are rejected after filtering",
                json!({ "task_ids": [1, 2, 3] }),
                Err("'task_ids' array must contain at least one task ID"),
            ),
            (
                "missing argument is rejected",
                json!({ "reason": "not needed" }),
                Err("requires 'task_ids' array argument"),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_skip_tasks(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::SkipTasks { task_ids, reason }),
                    Ok((expected_ids, expected_reason)),
                ) => {
                    assert_eq!(task_ids, expected_ids, "{name}");
                    assert_eq!(reason.as_deref(), expected_reason, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_complete_requires_boolean_success() {
        let cases = [
            (
                "full payload",
                json!({
                    "success": true,
                    "message": "Implemented the feature",
                    "notes": "Left follow-up details",
                    "add_tasks": [
                        {
                            "name": "task_follow_up",
                            "description": "Handle the leftover edge case",
                            "depends_on": ["task001"]
                        }
                    ]
                }),
                Ok((
                    true,
                    Some("Implemented the feature"),
                    Some("Left follow-up details"),
                    1usize,
                )),
            ),
            (
                "missing success",
                json!({ "message": "done" }),
                Err("requires 'success' boolean argument"),
            ),
            (
                "invalid success type",
                json!({ "success": "yes" }),
                Err("requires 'success' boolean argument"),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_complete(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::Complete {
                        success,
                        message,
                        notes,
                        add_tasks,
                    }),
                    Ok((expected_success, expected_message, expected_notes, expected_task_count)),
                ) => {
                    assert_eq!(success, expected_success, "{name}");
                    assert_eq!(message.as_deref(), expected_message, "{name}");
                    assert_eq!(notes.as_deref(), expected_notes, "{name}");
                    assert_eq!(
                        add_tasks.as_ref().map_or(0, std::vec::Vec::len),
                        expected_task_count,
                        "{name}"
                    );
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_remaining_supported_tools_validate_required_arguments() {
        let create_task_cases = [
            (
                "valid create_task payload",
                json!({
                    "name": "task_focus_tests",
                    "description": "Add focused parser coverage",
                    "dependencies": ["task001", 99, true]
                }),
                Ok((
                    "task_focus_tests",
                    "Add focused parser coverage",
                    vec!["task001"],
                )),
            ),
            (
                "create_task missing name",
                json!({ "description": "missing a name" }),
                Err("requires 'name' string argument"),
            ),
            (
                "create_task missing description",
                json!({ "name": "task_missing_description" }),
                Err("requires 'description' string argument"),
            ),
        ];

        for (name, arguments, expected) in create_task_cases {
            let parsed = parse_create_task(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::CreateTask {
                        name,
                        description,
                        dependencies,
                    }),
                    Ok((expected_name, expected_description, expected_dependencies)),
                ) => {
                    assert_eq!(name, expected_name);
                    assert_eq!(description, expected_description);
                    assert_eq!(dependencies, expected_dependencies);
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }

        let set_goal_cases = [
            (
                "valid set_goal payload",
                json!({
                    "summary": "Improve handler coverage",
                    "acceptance_criteria": "Focused tests cover parser and response edge cases"
                }),
                Ok((
                    "Improve handler coverage",
                    Some("Focused tests cover parser and response edge cases"),
                )),
            ),
            (
                "set_goal missing summary",
                json!({ "acceptance_criteria": "must be descriptive" }),
                Err("requires 'summary' string argument"),
            ),
        ];

        for (name, arguments, expected) in set_goal_cases {
            let parsed = parse_set_goal(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::SetGoal {
                        summary,
                        acceptance_criteria,
                    }),
                    Ok((expected_summary, expected_criteria)),
                ) => {
                    assert_eq!(summary, expected_summary, "{name}");
                    assert_eq!(acceptance_criteria.as_deref(), expected_criteria, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }

        let report_human_action_cases = [
            (
                "valid report_human_action payload",
                json!({
                    "description": "User approved the rollout",
                    "task_id": "task100"
                }),
                Ok((Some("task100"), "User approved the rollout")),
            ),
            (
                "report_human_action missing description",
                json!({ "task_id": "task101" }),
                Err("requires 'description' string argument"),
            ),
        ];

        for (name, arguments, expected) in report_human_action_cases {
            let parsed = parse_report_human_action(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::ReportHumanAction {
                        description,
                        task_id,
                    }),
                    Ok((expected_task_id, expected_description)),
                ) => {
                    assert_eq!(task_id.as_deref(), expected_task_id, "{name}");
                    assert_eq!(description, expected_description, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }

        let parsed = parse_list_tasks(&object(json!({ "status_filter": "pending" })));
        match parsed {
            Ok(ToolCall::ListTasks { status_filter }) => {
                assert_eq!(status_filter.as_deref(), Some("pending"));
            }
            other => panic!("unexpected list_tasks result: {other:?}"),
        }
    }

    #[test]
    fn parse_decompose_validates_missing_arguments_and_invalid_types() {
        let cases = [
            (
                "both task_id and task are missing",
                json!({}),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
            (
                "task_id is null",
                json!({ "task_id": null }),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
            (
                "task_id is number instead of string",
                json!({ "task_id": 123 }),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
            (
                "task_id is boolean instead of string",
                json!({ "task_id": true }),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
            (
                "task_id is array instead of string",
                json!({ "task_id": ["task001"] }),
                Err("requires 'task_id' argument. Create the task first with create_task if needed."),
            ),
            (
                "task is empty string (still accepted as valid)",
                json!({ "task": "" }),
                Ok((None, Some(""))),
            ),
            (
                "both provided prefers task_id",
                json!({ "task_id": "task001", "task": "inline task" }),
                Ok((Some("task001"), Some("inline task"))),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_decompose(&object(arguments));
            match (parsed, expected) {
                (Ok(ToolCall::Decompose { task_id, task }), Ok((expected_id, expected_task))) => {
                    assert_eq!(task_id.as_deref(), expected_id, "{name}");
                    assert_eq!(task.as_deref(), expected_task, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_spawn_agents_validates_mixed_valid_invalid_specs() {
        // Test that parsing fails when any agent has invalid types
        let cases = [
            (
                "first agent valid, second has non-string task_id",
                json!({
                    "agents": [
                        { "task_id": "task001", "role": "implementer" },
                        { "task_id": 42, "role": "verifier" }
                    ]
                }),
                Err("requires 'agents' array of {task_id, role} objects"),
            ),
            (
                "all agents have non-string types",
                json!({
                    "agents": [
                        { "task_id": 1, "role": "implementer" },
                        { "task_id": 2, "role": "verifier" }
                    ]
                }),
                Err("requires 'agents' array of {task_id, role} objects"),
            ),
            (
                "agent with numeric role",
                json!({
                    "agents": [
                        { "task_id": "task001", "role": 42 }
                    ]
                }),
                Err("requires 'agents' array of {task_id, role} objects"),
            ),
            (
                "agents is not an array",
                json!({ "agents": "single-agent" }),
                Err("requires 'agents' array of {task_id, role} objects"),
            ),
            (
                "agents is null",
                json!({ "agents": null }),
                Err("requires 'agents' array of {task_id, role} objects"),
            ),
            (
                "empty agents array is valid",
                json!({ "agents": [] }),
                Ok((0usize, WaitMode::All)),
            ),
            (
                "multiple valid agents with None wait mode",
                json!({
                    "agents": [
                        { "task_id": "task001" },
                        { "task_id": "task002", "role": "verifier" },
                        { "task": "inline task without task_id" }
                    ],
                    "wait": "None"
                }),
                Ok((3usize, WaitMode::None)),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_spawn_agents(&object(arguments));
            match (parsed, expected) {
                (Ok(ToolCall::SpawnAgents { agents, wait }), Ok((count, expected_wait))) => {
                    assert_eq!(agents.len(), count, "{name}");
                    assert_eq!(wait, expected_wait, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_spawn_agents_deprecated_task_vs_task_id_fallback() {
        let cases = [
            (
                "task_id preferred over task",
                json!({
                    "agents": [{ "task_id": "task001", "task": "should use task_id" }]
                }),
                (Some("task001"), Some("should use task_id")),
            ),
            (
                "task only (deprecated)",
                json!({
                    "agents": [{ "task": "Use deprecated task field" }]
                }),
                (None, Some("Use deprecated task field")),
            ),
            (
                "multiple agents with mixed task_id and task",
                json!({
                    "agents": [
                        { "task_id": "task001" },
                        { "task": "Deprecated inline" },
                        { "task_id": "task003", "task": "Both provided" }
                    ]
                }),
                (Some("task001"), None), // Just check first agent
            ),
        ];

        for (name, arguments, (expected_task_id, expected_task)) in cases {
            let parsed = parse_spawn_agents(&object(arguments));
            match parsed {
                Ok(ToolCall::SpawnAgents { agents, .. }) => {
                    assert!(!agents.is_empty(), "{name}: expected at least one agent");
                    let first = &agents[0];
                    assert_eq!(first.task_id.as_deref(), expected_task_id, "{name}");
                    assert_eq!(first.task.as_deref(), expected_task, "{name}");
                }
                Err(e) => panic!("{name}: unexpected error: {e}"),
                other => panic!("{name}: unexpected result: {other:?}"),
            }
        }
    }

    #[test]
    fn parse_complete_validates_all_argument_combinations() {
        let cases = [
            (
                "minimal success",
                json!({ "success": true }),
                Ok((true, None, None, 0)),
            ),
            (
                "success with only message",
                json!({ "success": true, "message": "All done" }),
                Ok((true, Some("All done"), None, 0)),
            ),
            (
                "success with notes only",
                json!({ "success": true, "notes": "Future consideration" }),
                Ok((true, None, Some("Future consideration"), 0)),
            ),
            (
                "failure with all fields",
                json!({
                    "success": false,
                    "message": "Test failures",
                    "notes": "Needs investigation",
                    "add_tasks": []
                }),
                Ok((false, Some("Test failures"), Some("Needs investigation"), 0)),
            ),
            (
                "success is string instead of boolean",
                json!({ "success": "true" }),
                Err("requires 'success' boolean argument"),
            ),
            (
                "success is null",
                json!({ "success": null }),
                Err("requires 'success' boolean argument"),
            ),
            (
                "success is number (0 is not false)",
                json!({ "success": 0 }),
                Err("requires 'success' boolean argument"),
            ),
            (
                "invalid add_tasks format is ignored",
                json!({
                    "success": true,
                    "add_tasks": "not an array"
                }),
                Ok((true, None, None, 0)),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_complete(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::Complete {
                        success,
                        message,
                        notes,
                        add_tasks,
                    }),
                    Ok((expected_success, expected_message, expected_notes, expected_task_count)),
                ) => {
                    assert_eq!(success, expected_success, "{name}");
                    assert_eq!(message.as_deref(), expected_message, "{name}");
                    assert_eq!(notes.as_deref(), expected_notes, "{name}");
                    assert_eq!(
                        add_tasks.as_ref().map_or(0, std::vec::Vec::len),
                        expected_task_count,
                        "{name}"
                    );
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_create_task_validates_all_required_arguments() {
        let cases = [
            (
                "missing name",
                json!({ "description": "Some work" }),
                Err("requires 'name' string argument"),
            ),
            (
                "missing description",
                json!({ "name": "my_task" }),
                Err("requires 'description' string argument"),
            ),
            (
                "name is not a string",
                json!({ "name": 123, "description": "work" }),
                Err("requires 'name' string argument"),
            ),
            (
                "description is not a string",
                json!({ "name": "my_task", "description": 123 }),
                Err("requires 'description' string argument"),
            ),
            (
                "name is null",
                json!({ "name": null, "description": "work" }),
                Err("requires 'name' string argument"),
            ),
            (
                "valid without dependencies",
                json!({ "name": "my_task", "description": "Do the work" }),
                Ok(("my_task", "Do the work", vec![])),
            ),
            (
                "valid with empty dependencies",
                json!({ "name": "my_task", "description": "work", "dependencies": [] }),
                Ok(("my_task", "work", vec![])),
            ),
            (
                "dependencies filters non-strings",
                json!({
                    "name": "my_task",
                    "description": "work",
                    "dependencies": ["dep1", 123, null, "dep2"]
                }),
                Ok(("my_task", "work", vec!["dep1", "dep2"])),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_create_task(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::CreateTask {
                        name,
                        description,
                        dependencies,
                    }),
                    Ok((expected_name, expected_desc, expected_deps)),
                ) => {
                    assert_eq!(name, expected_name);
                    assert_eq!(description, expected_desc);
                    assert_eq!(dependencies, expected_deps);
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_set_goal_validates_all_required_arguments() {
        let cases = [
            (
                "missing summary",
                json!({}),
                Err("requires 'summary' string argument"),
            ),
            (
                "summary is null",
                json!({ "summary": null }),
                Err("requires 'summary' string argument"),
            ),
            (
                "summary is number",
                json!({ "summary": 42 }),
                Err("requires 'summary' string argument"),
            ),
            (
                "valid with acceptance_criteria",
                json!({
                    "summary": "Improve test coverage",
                    "acceptance_criteria": ">80% coverage"
                }),
                Ok(("Improve test coverage", Some(">80% coverage"))),
            ),
            (
                "valid without acceptance_criteria",
                json!({ "summary": "Fix the bug" }),
                Ok(("Fix the bug", None)),
            ),
            (
                "acceptance_criteria is non-string (ignored)",
                json!({ "summary": "Goal", "acceptance_criteria": 123 }),
                Ok(("Goal", None)),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_set_goal(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::SetGoal {
                        summary,
                        acceptance_criteria,
                    }),
                    Ok((expected_summary, expected_criteria)),
                ) => {
                    assert_eq!(summary, expected_summary, "{name}");
                    assert_eq!(acceptance_criteria.as_deref(), expected_criteria, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_skip_tasks_validates_empty_and_missing_arrays() {
        let cases = [
            (
                "missing task_ids",
                json!({}),
                Err("requires 'task_ids' array argument"),
            ),
            (
                "task_ids is null",
                json!({ "task_ids": null }),
                Err("requires 'task_ids' array argument"),
            ),
            (
                "task_ids is not an array",
                json!({ "task_ids": "task001" }),
                Err("requires 'task_ids' array argument"),
            ),
            (
                "empty task_ids array",
                json!({ "task_ids": [] }),
                Err("'task_ids' array must contain at least one task ID"),
            ),
            (
                "all non-string items filtered leaves empty",
                json!({ "task_ids": [1, 2, null, true] }),
                Err("'task_ids' array must contain at least one task ID"),
            ),
            (
                "valid single task_id",
                json!({ "task_ids": ["task001"] }),
                Ok((vec!["task001"], None)),
            ),
            (
                "valid multiple task_ids with reason",
                json!({
                    "task_ids": ["task001", "task002", "task003"],
                    "reason": "All obsolete after refactor"
                }),
                Ok((vec!["task001", "task002", "task003"], Some("All obsolete after refactor"))),
            ),
            (
                "reason is non-string (ignored)",
                json!({ "task_ids": ["task001"], "reason": 123 }),
                Ok((vec!["task001"], None)),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_skip_tasks(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::SkipTasks { task_ids, reason }),
                    Ok((expected_ids, expected_reason)),
                ) => {
                    assert_eq!(task_ids, expected_ids, "{name}");
                    assert_eq!(reason.as_deref(), expected_reason, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_report_human_action_validates_all_required_arguments() {
        let cases = [
            (
                "missing description",
                json!({}),
                Err("requires 'description' string argument"),
            ),
            (
                "description is null",
                json!({ "description": null }),
                Err("requires 'description' string argument"),
            ),
            (
                "description is number",
                json!({ "description": 42 }),
                Err("requires 'description' string argument"),
            ),
            (
                "valid without task_id",
                json!({ "description": "User deployed manually" }),
                Ok(("User deployed manually", None)),
            ),
            (
                "valid with task_id",
                json!({ "description": "User ran migrations", "task_id": "task001" }),
                Ok(("User ran migrations", Some("task001"))),
            ),
            (
                "task_id is non-string (ignored)",
                json!({ "description": "Action", "task_id": 123 }),
                Ok(("Action", None)),
            ),
        ];

        for (name, arguments, expected) in cases {
            let parsed = parse_report_human_action(&object(arguments));
            match (parsed, expected) {
                (
                    Ok(ToolCall::ReportHumanAction {
                        description,
                        task_id,
                    }),
                    Ok((expected_desc, expected_task_id)),
                ) => {
                    assert_eq!(description, expected_desc, "{name}");
                    assert_eq!(task_id.as_deref(), expected_task_id, "{name}");
                }
                (Err(actual), Err(expected_error)) => {
                    assert_eq!(actual, expected_error, "{name}");
                }
                (other, expected) => {
                    panic!("unexpected result for {name}: got {other:?}, expected {expected:?}");
                }
            }
        }
    }

    #[test]
    fn parse_list_tasks_accepts_all_valid_inputs() {
        let cases = [
            ("no filter", json!({}), None),
            ("pending filter", json!({ "status_filter": "pending" }), Some("pending")),
            ("completed filter", json!({ "status_filter": "completed" }), Some("completed")),
            ("all filter", json!({ "status_filter": "all" }), Some("all")),
            ("filter is non-string (ignored)", json!({ "status_filter": 123 }), None),
            ("filter is null (ignored)", json!({ "status_filter": null }), None),
        ];

        for (name, arguments, expected_filter) in cases {
            let parsed = parse_list_tasks(&object(arguments));
            match parsed {
                Ok(ToolCall::ListTasks { status_filter }) => {
                    assert_eq!(status_filter.as_deref(), expected_filter, "{name}");
                }
                other => panic!("unexpected result for {name}: {other:?}"),
            }
        }
    }
}
