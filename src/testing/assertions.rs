//! Test run results and assertion helpers.
//!
//! This module provides the `TestRunResult` type for capturing test outcomes
//! and assertion helper functions for common test patterns.

use crate::mcp_server::{ToolCall, ToolResponse, WaitMode};
use crate::types::TaskResult;

// ============================================================================
// Test Run Result
// ============================================================================

/// Result of a test run, including captured data for assertions.
#[derive(Debug, Clone)]
pub struct TestRunResult {
    /// The final task result from the orchestration.
    pub task_result: TaskResult,
    /// All tool calls that were intercepted during the run.
    pub tool_calls: Vec<CapturedToolCall>,
    /// The same tool calls paired with the actual responses returned by `App`.
    pub app_tool_calls: Vec<CapturedToolCall>,
    /// All prompts sent to agents (`session_id`, prompt).
    pub prompts_sent: Vec<(String, String)>,
    /// Session IDs of all sessions created during the run.
    pub sessions_created: Vec<String>,
    /// Final task states loaded from tasks.json (`task_id`, `task_name`, `status`).
    /// Status is one of: `pending`, `in_progress`, `completed`, `failed`, or `skipped`.
    pub final_tasks: Vec<FinalTaskState>,
}

/// Final state of a task at the end of a test run.
#[derive(Debug, Clone)]
pub struct FinalTaskState {
    /// Task ID (e.g., "task001").
    pub task_id: String,
    /// Task name.
    pub name: String,
    /// Final status (e.g., `completed`, `failed`, `skipped`, `pending`, or `in_progress`).
    pub status: String,
}

impl TestRunResult {
    fn spawn_agents_calls_from(tool_calls: &[CapturedToolCall]) -> Vec<String> {
        tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::SpawnAgents { agents, .. } => agents
                    .first()
                    .and_then(|a| a.task.clone().or_else(|| a.task_id.clone())),
                _ => None,
            })
            .collect()
    }

    fn spawn_agents_batches_from(tool_calls: &[CapturedToolCall]) -> Vec<(WaitMode, Vec<String>)> {
        tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::SpawnAgents { agents, wait } => Some((
                    *wait,
                    agents
                        .iter()
                        .filter_map(|agent| agent.task.clone().or_else(|| agent.task_id.clone()))
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    fn decompose_calls_from(tool_calls: &[CapturedToolCall]) -> Vec<String> {
        tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::Decompose { task_id, task } => task.clone().or_else(|| task_id.clone()),
                _ => None,
            })
            .collect()
    }

    fn skip_tasks_calls_from(
        tool_calls: &[CapturedToolCall],
    ) -> Vec<(Vec<String>, Option<String>)> {
        tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::SkipTasks { task_ids, reason } => {
                    Some((task_ids.clone(), reason.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn complete_calls_from(tool_calls: &[CapturedToolCall]) -> Vec<(bool, Option<String>)> {
        tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::Complete {
                    success, message, ..
                } => Some((*success, message.clone())),
                _ => None,
            })
            .collect()
    }

    /// Check if a planner agent was spawned during the test run.
    pub fn planner_was_spawned(&self) -> bool {
        self.sessions_created
            .iter()
            .any(|id| id.contains("planner"))
    }

    /// Check if an orchestrator agent was spawned during the test run.
    pub fn orchestrator_was_spawned(&self) -> bool {
        self.sessions_created
            .iter()
            .any(|id| id.contains("orchestrator") || id.contains("orch"))
    }

    /// Check if an implementer agent was spawned during the test run.
    pub fn implementer_was_spawned(&self) -> bool {
        self.sessions_created
            .iter()
            .any(|id| id.contains("impl") || id.contains("implementer"))
    }

    /// Get all `spawn_agents` tool calls as task strings (first agent's task from each call).
    pub fn spawn_agents_calls(&self) -> Vec<String> {
        Self::spawn_agents_calls_from(&self.tool_calls)
    }

    /// Get all actual `spawn_agents` tool calls using the app's real responses.
    pub fn app_spawn_agents_calls(&self) -> Vec<String> {
        Self::spawn_agents_calls_from(&self.app_tool_calls)
    }

    /// Get `spawn_agents` batches as (`wait_mode`, `tasks`) tuples.
    pub fn spawn_agents_batches(&self) -> Vec<(WaitMode, Vec<String>)> {
        Self::spawn_agents_batches_from(&self.tool_calls)
    }

    /// Get actual `spawn_agents` batches as (`wait_mode`, `tasks`) tuples.
    pub fn app_spawn_agents_batches(&self) -> Vec<(WaitMode, Vec<String>)> {
        Self::spawn_agents_batches_from(&self.app_tool_calls)
    }

    /// Alias for backward compatibility - returns tasks from `spawn_agents` calls.
    pub fn implement_calls(&self) -> Vec<String> {
        self.spawn_agents_calls()
    }

    /// Alias for actual `spawn_agents` tasks from app responses.
    pub fn app_implement_calls(&self) -> Vec<String> {
        self.app_spawn_agents_calls()
    }

    /// Get all `decompose()` tool calls as task strings.
    pub fn decompose_calls(&self) -> Vec<String> {
        Self::decompose_calls_from(&self.tool_calls)
    }

    /// Get actual `decompose()` tool calls using the app's real responses.
    pub fn app_decompose_calls(&self) -> Vec<String> {
        Self::decompose_calls_from(&self.app_tool_calls)
    }

    /// Get all `skip_tasks` tool calls as (`task_ids`, reason) tuples.
    pub fn skip_tasks_calls(&self) -> Vec<(Vec<String>, Option<String>)> {
        Self::skip_tasks_calls_from(&self.tool_calls)
    }

    /// Get actual `skip_tasks` tool calls using the app's real responses.
    pub fn app_skip_tasks_calls(&self) -> Vec<(Vec<String>, Option<String>)> {
        Self::skip_tasks_calls_from(&self.app_tool_calls)
    }

    /// Get all `complete()` tool calls as (success, message) tuples.
    pub fn complete_calls(&self) -> Vec<(bool, Option<String>)> {
        Self::complete_calls_from(&self.tool_calls)
    }

    /// Get actual `complete()` tool calls using the app's real responses.
    pub fn app_complete_calls(&self) -> Vec<(bool, Option<String>)> {
        Self::complete_calls_from(&self.app_tool_calls)
    }

    /// Return the final status string for a task ID if present.
    pub fn final_task_status(&self, task_id: &str) -> Option<&str> {
        self.final_tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .map(|task| task.status.as_str())
    }

    /// Return all final tasks with a specific status.
    pub fn tasks_with_status(&self, status: &str) -> Vec<&FinalTaskState> {
        self.final_tasks
            .iter()
            .filter(|task| task.status == status)
            .collect()
    }
}

/// A captured tool call with its response.
#[derive(Debug, Clone)]
pub struct CapturedToolCall {
    /// The tool call that was made.
    pub call: ToolCall,
    /// The response that was returned.
    pub response: ToolResponse,
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Assert that a planner was spawned during the test.
///
/// # Panics
///
/// Panics if no planner session was found in the test result, indicating the planner
/// agent was not spawned during the test run.
pub fn assert_planner_spawned(result: &TestRunResult) {
    assert!(
        result.planner_was_spawned(),
        "Expected planner agent to be spawned, but no planner session was found. \
         Sessions created: {:?}",
        result.sessions_created
    );
}

/// Assert that an orchestrator was spawned during the test.
///
/// # Panics
///
/// Panics if no orchestrator session was found in the test result, indicating the
/// orchestrator agent was not spawned during the test run.
pub fn assert_orchestrator_spawned(result: &TestRunResult) {
    assert!(
        result.orchestrator_was_spawned(),
        "Expected orchestrator agent to be spawned, but no orchestrator session was found. \
         Sessions created: {:?}",
        result.sessions_created
    );
}

/// Assert that an implementer was spawned during the test.
///
/// # Panics
///
/// Panics if no implementer session was found in the test result, indicating the
/// implementer agent was not spawned during the test run.
pub fn assert_implementer_spawned(result: &TestRunResult) {
    assert!(
        result.implementer_was_spawned(),
        "Expected implementer agent to be spawned, but no implementer session was found. \
         Sessions created: {:?}",
        result.sessions_created
    );
}

/// Assert that at least one `implement()` call was made.
///
/// # Panics
///
/// Panics if no implement tool calls were made during the test run, indicating the
/// implementation flow was not triggered.
pub fn assert_implement_called(result: &TestRunResult) {
    assert!(
        !result.implement_calls().is_empty(),
        "Expected at least one implement() call, but none were made. \
         Tool calls: {:?}",
        result
            .tool_calls
            .iter()
            .map(|c| format!("{:?}", c.call))
            .collect::<Vec<_>>()
    );
}

/// Assert that at least one `decompose()` call was made.
///
/// # Panics
///
/// Panics if no decompose tool calls were made during the test run, indicating the
/// task decomposition flow was not triggered.
pub fn assert_decompose_called(result: &TestRunResult) {
    assert!(
        !result.decompose_calls().is_empty(),
        "Expected at least one decompose() call, but none were made. \
         Tool calls: {:?}",
        result
            .tool_calls
            .iter()
            .map(|c| format!("{:?}", c.call))
            .collect::<Vec<_>>()
    );
}

/// Assert that the test completed successfully.
///
/// # Panics
///
/// Panics if the task result indicates failure (success field is false).
pub fn assert_success(result: &TestRunResult) {
    assert!(
        result.task_result.success,
        "Expected successful completion, but task failed. \
         Message: {:?}",
        result.task_result.message
    );
}

/// Assert that the test failed.
///
/// # Panics
///
/// Panics if the task result indicates success (success field is true).
pub fn assert_failure(result: &TestRunResult) {
    assert!(
        !result.task_result.success,
        "Expected failure, but task succeeded. \
         Message: {:?}",
        result.task_result.message
    );
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_run_result_helpers() {
        let result = TestRunResult {
            task_result: TaskResult {
                success: true,
                message: Some("Done".to_string()),
            },
            tool_calls: vec![
                CapturedToolCall {
                    call: ToolCall::SpawnAgents {
                        agents: vec![crate::mcp_server::AgentSpec {
                            role: Some("implementer".to_string()),
                            task: Some("task1".to_string()),
                            task_id: None,
                            prompt: None,
                            tools: None,
                            model_complexity: None,
                        }],
                        wait: crate::mcp_server::WaitMode::All,
                    },
                    response: ToolResponse::success("req-1".to_string(), "done".to_string()),
                },
                CapturedToolCall {
                    call: ToolCall::Decompose {
                        task_id: None,
                        task: Some("complex task".to_string()),
                    },
                    response: ToolResponse::success("req-2".to_string(), "decomposed".to_string()),
                },
            ],
            app_tool_calls: vec![],
            prompts_sent: vec![],
            sessions_created: vec![
                "planner-001".to_string(),
                "orchestrator-001".to_string(),
                "impl-001".to_string(),
            ],
            final_tasks: vec![],
        };

        assert!(result.planner_was_spawned());
        assert!(result.orchestrator_was_spawned());
        assert!(result.implementer_was_spawned());

        let impl_calls = result.implement_calls();
        assert_eq!(impl_calls.len(), 1);
        assert_eq!(impl_calls[0], "task1");

        let decompose_calls = result.decompose_calls();
        assert_eq!(decompose_calls.len(), 1);
        assert_eq!(decompose_calls[0], "complex task");
    }

    #[test]
    fn test_test_run_result_no_planner() {
        let result = TestRunResult {
            task_result: TaskResult {
                success: true,
                message: None,
            },
            tool_calls: vec![],
            app_tool_calls: vec![],
            prompts_sent: vec![],
            sessions_created: vec!["orchestrator-001".to_string()],
            final_tasks: vec![],
        };

        assert!(!result.planner_was_spawned());
        assert!(result.orchestrator_was_spawned());
        assert!(!result.implementer_was_spawned());
    }

    #[test]
    fn test_skip_tasks_calls_helper() {
        let result = TestRunResult {
            task_result: TaskResult {
                success: true,
                message: Some("Done".to_string()),
            },
            tool_calls: vec![
                CapturedToolCall {
                    call: ToolCall::SkipTasks {
                        task_ids: vec!["task001".to_string(), "task002".to_string()],
                        reason: Some("Not needed".to_string()),
                    },
                    response: ToolResponse::success("req-1".to_string(), "Skipped 2".to_string()),
                },
                CapturedToolCall {
                    call: ToolCall::SkipTasks {
                        task_ids: vec!["task003".to_string()],
                        reason: None,
                    },
                    response: ToolResponse::success("req-2".to_string(), "Skipped 1".to_string()),
                },
            ],
            app_tool_calls: vec![],
            prompts_sent: vec![],
            sessions_created: vec![],
            final_tasks: vec![],
        };

        let skip_calls = result.skip_tasks_calls();
        assert_eq!(skip_calls.len(), 2);

        assert_eq!(
            skip_calls[0].0,
            vec!["task001".to_string(), "task002".to_string()]
        );
        assert_eq!(skip_calls[0].1, Some("Not needed".to_string()));

        assert_eq!(skip_calls[1].0, vec!["task003".to_string()]);
        assert!(skip_calls[1].1.is_none());
    }

    #[test]
    fn test_complete_calls_helper() {
        let result = TestRunResult {
            task_result: TaskResult {
                success: true,
                message: Some("Done".to_string()),
            },
            tool_calls: vec![
                CapturedToolCall {
                    call: ToolCall::Complete {
                        success: true,
                        message: Some("Plan created".to_string()),
                        notes: None,
                        add_tasks: None,
                    },
                    response: ToolResponse::success("req-1".to_string(), "OK".to_string()),
                },
                CapturedToolCall {
                    call: ToolCall::Complete {
                        success: false,
                        message: Some("Task failed".to_string()),
                        notes: None,
                        add_tasks: None,
                    },
                    response: ToolResponse::success("req-2".to_string(), "OK".to_string()),
                },
            ],
            app_tool_calls: vec![],
            prompts_sent: vec![],
            sessions_created: vec![],
            final_tasks: vec![],
        };

        let complete_calls = result.complete_calls();
        assert_eq!(complete_calls.len(), 2);

        assert!(complete_calls[0].0); // success=true
        assert_eq!(complete_calls[0].1, Some("Plan created".to_string()));

        assert!(!complete_calls[1].0); // success=false
        assert_eq!(complete_calls[1].1, Some("Task failed".to_string()));
    }
}
