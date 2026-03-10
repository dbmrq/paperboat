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
