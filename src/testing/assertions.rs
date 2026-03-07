//! Test run results and assertion helpers.
//!
//! This module provides the `TestRunResult` type for capturing test outcomes
//! and assertion helper functions for common test patterns.

use crate::mcp_server::{ToolCall, ToolResponse};
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
    /// All prompts sent to agents (`session_id`, prompt).
    pub prompts_sent: Vec<(String, String)>,
    /// Session IDs of all sessions created during the run.
    pub sessions_created: Vec<String>,
}

impl TestRunResult {
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

    /// Get all `spawn_agents()` tool calls as task strings (first agent's task from each call).
    pub fn spawn_agents_calls(&self) -> Vec<String> {
        self.tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::SpawnAgents { agents, .. } => {
                    // Get task or task_id as fallback
                    agents.first().and_then(|a| {
                        a.task
                            .clone()
                            .or_else(|| a.task_id.clone())
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Alias for backward compatibility - returns tasks from spawn_agents calls.
    pub fn implement_calls(&self) -> Vec<String> {
        self.spawn_agents_calls()
    }

    /// Get all `decompose()` tool calls as task strings.
    pub fn decompose_calls(&self) -> Vec<String> {
        self.tool_calls
            .iter()
            .filter_map(|c| match &c.call {
                ToolCall::Decompose { task_id, task } => {
                    // Return task if present, otherwise task_id
                    task.clone().or_else(|| task_id.clone())
                }
                _ => None,
            })
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
pub fn assert_planner_spawned(result: &TestRunResult) {
    assert!(
        result.planner_was_spawned(),
        "Expected planner agent to be spawned, but no planner session was found. \
         Sessions created: {:?}",
        result.sessions_created
    );
}

/// Assert that an orchestrator was spawned during the test.
pub fn assert_orchestrator_spawned(result: &TestRunResult) {
    assert!(
        result.orchestrator_was_spawned(),
        "Expected orchestrator agent to be spawned, but no orchestrator session was found. \
         Sessions created: {:?}",
        result.sessions_created
    );
}

/// Assert that an implementer was spawned during the test.
pub fn assert_implementer_spawned(result: &TestRunResult) {
    assert!(
        result.implementer_was_spawned(),
        "Expected implementer agent to be spawned, but no implementer session was found. \
         Sessions created: {:?}",
        result.sessions_created
    );
}

/// Assert that at least one `implement()` call was made.
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
pub fn assert_success(result: &TestRunResult) {
    assert!(
        result.task_result.success,
        "Expected successful completion, but task failed. \
         Message: {:?}",
        result.task_result.message
    );
}

/// Assert that the test failed.
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
            prompts_sent: vec![],
            sessions_created: vec![
                "planner-001".to_string(),
                "orchestrator-001".to_string(),
                "impl-001".to_string(),
            ],
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
            prompts_sent: vec![],
            sessions_created: vec!["orchestrator-001".to_string()],
        };

        assert!(!result.planner_was_spawned());
        assert!(result.orchestrator_was_spawned());
        assert!(!result.implementer_was_spawned());
    }
}
