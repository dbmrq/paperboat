//! Mock tool interceptor for testing.
//!
//! This module provides `MockToolInterceptor` which intercepts tool calls
//! and returns scripted responses from test scenarios.

use super::assertions::CapturedToolCall;
use super::{MockScenario, MockToolType};
use crate::mcp_server::{ToolCall, ToolResponse};
use anyhow::Result;
use std::collections::VecDeque;

// ============================================================================
// Mock Tool Interceptor
// ============================================================================

/// Thread-safe interceptor for tool calls that provides scripted responses.
#[derive(Debug)]
pub struct MockToolInterceptor {
    /// Queue of tool responses to return (consumed in order).
    pub(crate) response_queue: VecDeque<(MockToolType, crate::testing::MockToolResponseData)>,
    /// Captured tool calls for assertions.
    captured_calls: Vec<CapturedToolCall>,
    /// Counter for tracking exhaustion errors.
    exhausted_count: usize,
}

impl MockToolInterceptor {
    /// Create a new interceptor from a scenario's `mock_tool_responses`.
    pub fn from_scenario(scenario: &MockScenario) -> Self {
        let response_queue = scenario
            .mock_tool_responses
            .iter()
            .map(|r| (r.tool_type, r.response.clone()))
            .collect();

        Self {
            response_queue,
            captured_calls: Vec::new(),
            exhausted_count: 0,
        }
    }

    /// Create an empty interceptor.
    pub const fn empty() -> Self {
        Self {
            response_queue: VecDeque::new(),
            captured_calls: Vec::new(),
            exhausted_count: 0,
        }
    }

    /// Check if all responses have been consumed.
    pub fn is_exhausted(&self) -> bool {
        self.response_queue.is_empty()
    }

    /// Get the number of times we tried to get a response when exhausted.
    pub const fn exhausted_count(&self) -> usize {
        self.exhausted_count
    }

    /// Get all captured tool calls.
    pub fn captured_calls(&self) -> &[CapturedToolCall] {
        &self.captured_calls
    }

    /// Get a response for a tool call, returning an error if exhausted.
    ///
    /// # Errors
    ///
    /// Returns an error if there are no more mock responses in the queue for the
    /// expected tool type.
    ///
    /// # Panics
    ///
    /// Panics if a matching response is found in the queue but the position becomes
    /// invalid (should never happen in single-threaded use).
    pub fn get_response(&mut self, call: &ToolCall, request_id: &str) -> Result<ToolResponse> {
        let expected_type = match call {
            ToolCall::Decompose { task_id, task } => {
                // First check if there's a specific mock response for decompose
                if let Some(pos) = self
                    .response_queue
                    .iter()
                    .position(|(t, _)| *t == MockToolType::Decompose)
                {
                    let (_, response_data) = self.response_queue.remove(pos).unwrap();
                    let response = response_data.to_tool_response(request_id.to_string());
                    self.captured_calls.push(CapturedToolCall {
                        call: call.clone(),
                        response: response.clone(),
                    });
                    return Ok(response);
                }
                // Otherwise, decompose always succeeds (the App handles the actual decomposition)
                let task_desc = task
                    .as_deref()
                    .or(task_id.as_deref())
                    .unwrap_or("(unknown)");
                let response = ToolResponse::success(
                    request_id.to_string(),
                    format!("Decomposed: {task_desc}"),
                );
                self.captured_calls.push(CapturedToolCall {
                    call: call.clone(),
                    response: response.clone(),
                });
                return Ok(response);
            }
            ToolCall::SpawnAgents { .. } => MockToolType::SpawnAgents,
            ToolCall::Complete {
                success, message, ..
            } => {
                // Complete always succeeds (records the agent's completion status)
                let response = ToolResponse::success(
                    request_id.to_string(),
                    message.clone().unwrap_or_else(|| {
                        if *success {
                            "Task completed successfully".to_string()
                        } else {
                            "Task failed".to_string()
                        }
                    }),
                );
                self.captured_calls.push(CapturedToolCall {
                    call: call.clone(),
                    response: response.clone(),
                });
                return Ok(response);
            }
            ToolCall::CreateTask { name, .. } => {
                // CreateTask always succeeds
                let response = ToolResponse::success(
                    request_id.to_string(),
                    format!("Task '{name}' created successfully"),
                );
                self.captured_calls.push(CapturedToolCall {
                    call: call.clone(),
                    response: response.clone(),
                });
                return Ok(response);
            }
            ToolCall::SetGoal { summary, .. } => {
                // SetGoal always succeeds
                let response =
                    ToolResponse::success(request_id.to_string(), format!("Goal set: {summary}"));
                self.captured_calls.push(CapturedToolCall {
                    call: call.clone(),
                    response: response.clone(),
                });
                return Ok(response);
            }
            ToolCall::SkipTasks { task_ids, reason } => {
                // SkipTasks always succeeds
                let reason_str = reason.as_deref().unwrap_or("No reason provided");
                let response = ToolResponse::success(
                    request_id.to_string(),
                    format!(
                        "Skipped {} task(s): {:?} ({})",
                        task_ids.len(),
                        task_ids,
                        reason_str
                    ),
                );
                self.captured_calls.push(CapturedToolCall {
                    call: call.clone(),
                    response: response.clone(),
                });
                return Ok(response);
            }
            ToolCall::ListTasks { status_filter } => {
                // ListTasks always succeeds with a mock task list
                let filter = status_filter.as_deref().unwrap_or("all");
                let response = ToolResponse::success(
                    request_id.to_string(),
                    format!("## Tasks (mock, filter={filter})\n- No tasks in mock mode"),
                );
                self.captured_calls.push(CapturedToolCall {
                    call: call.clone(),
                    response: response.clone(),
                });
                return Ok(response);
            }
        };

        // Find a matching response in the queue (only for Implement at this point)
        if let Some(pos) = self
            .response_queue
            .iter()
            .position(|(t, _)| *t == expected_type)
        {
            let (_, response_data) = self.response_queue.remove(pos).unwrap();
            let response = response_data.to_tool_response(request_id.to_string());
            self.captured_calls.push(CapturedToolCall {
                call: call.clone(),
                response: response.clone(),
            });
            Ok(response)
        } else {
            self.exhausted_count += 1;
            Err(anyhow::anyhow!(
                "Mock tool responses exhausted: no response available for {:?}. \
                 {} tool calls were made successfully before exhaustion. \
                 Remaining responses: {:?}",
                expected_type,
                self.captured_calls.len(),
                self.response_queue
                    .iter()
                    .map(|(t, _)| format!("{t:?}"))
                    .collect::<Vec<_>>()
            ))
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockToolResponseBuilder;

    #[test]
    fn test_mock_tool_interceptor_from_scenario() {
        let scenario = MockScenario {
            mock_tool_responses: vec![
                MockToolResponseBuilder::new()
                    .tool_type(MockToolType::SpawnAgents)
                    .success("Done")
                    .build(),
                MockToolResponseBuilder::new()
                    .tool_type(MockToolType::Decompose)
                    .success("Decomposed")
                    .build(),
            ],
            ..Default::default()
        };

        let interceptor = MockToolInterceptor::from_scenario(&scenario);
        assert_eq!(interceptor.response_queue.len(), 2);
        assert!(!interceptor.is_exhausted());
    }

    #[test]
    fn test_mock_tool_interceptor_get_response() {
        let scenario = MockScenario {
            mock_tool_responses: vec![MockToolResponseBuilder::new()
                .tool_type(MockToolType::SpawnAgents)
                .success("Task completed")
                .build()],
            ..Default::default()
        };

        let mut interceptor = MockToolInterceptor::from_scenario(&scenario);

        let call = ToolCall::SpawnAgents {
            agents: vec![crate::mcp_server::AgentSpec {
                role: Some("implementer".to_string()),
                task: Some("test task".to_string()),
                task_id: None,
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: crate::mcp_server::WaitMode::All,
        };
        let response = interceptor.get_response(&call, "req-001").unwrap();

        assert!(response.success);
        assert_eq!(response.summary, "Task completed");
        assert!(interceptor.is_exhausted());
        assert_eq!(interceptor.captured_calls().len(), 1);
    }

    #[test]
    fn test_mock_tool_interceptor_exhausted_error() {
        let mut interceptor = MockToolInterceptor::empty();

        let call = ToolCall::SpawnAgents {
            agents: vec![crate::mcp_server::AgentSpec {
                role: Some("implementer".to_string()),
                task: Some("test task".to_string()),
                task_id: None,
                prompt: None,
                tools: None,
                model_complexity: None,
            }],
            wait: crate::mcp_server::WaitMode::All,
        };
        let result = interceptor.get_response(&call, "req-001");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exhausted"));
        assert_eq!(interceptor.exhausted_count(), 1);
    }

    #[test]
    fn test_mock_tool_interceptor_create_task_always_succeeds() {
        let mut interceptor = MockToolInterceptor::empty();

        let call = ToolCall::CreateTask {
            name: "Task 1".to_string(),
            description: "Do something".to_string(),
            dependencies: vec![],
        };
        let response = interceptor.get_response(&call, "req-001").unwrap();

        assert!(response.success);
        assert!(response.summary.contains("Task 'Task 1' created"));
    }

    #[tokio::test]
    async fn test_mock_tool_interceptor_captures_create_task() {
        // Test that create_task tool calls are captured correctly
        let mut interceptor = MockToolInterceptor::empty();

        let call = ToolCall::CreateTask {
            name: "Test Task".to_string(),
            description: "Test task content".to_string(),
            dependencies: vec![],
        };
        let response = interceptor.get_response(&call, "req-ct-001").unwrap();

        assert!(response.success);
        assert!(response.summary.contains("created"));

        // Verify the call was captured
        let captured = interceptor.captured_calls();
        assert_eq!(captured.len(), 1);
        match &captured[0].call {
            ToolCall::CreateTask {
                name, description, ..
            } => {
                assert_eq!(name, "Test Task");
                assert_eq!(description, "Test task content");
            }
            _ => panic!("Expected CreateTask call"),
        }
    }

    #[test]
    fn test_mock_tool_interceptor_skip_tasks_always_succeeds() {
        let mut interceptor = MockToolInterceptor::empty();

        let call = ToolCall::SkipTasks {
            task_ids: vec!["task001".to_string(), "task002".to_string()],
            reason: Some("Not needed".to_string()),
        };
        let response = interceptor.get_response(&call, "req-skip-001").unwrap();

        assert!(response.success);
        assert!(response.summary.contains("Skipped 2 task(s)"));
        assert!(response.summary.contains("task001"));
        assert!(response.summary.contains("task002"));
        assert!(response.summary.contains("Not needed"));
    }

    #[test]
    fn test_mock_tool_interceptor_skip_tasks_captures_call() {
        let mut interceptor = MockToolInterceptor::empty();

        let call = ToolCall::SkipTasks {
            task_ids: vec!["task003".to_string()],
            reason: None,
        };
        let _response = interceptor.get_response(&call, "req-skip-002").unwrap();

        // Verify the call was captured
        let captured = interceptor.captured_calls();
        assert_eq!(captured.len(), 1);
        match &captured[0].call {
            ToolCall::SkipTasks { task_ids, reason } => {
                assert_eq!(task_ids, &vec!["task003".to_string()]);
                assert!(reason.is_none());
            }
            _ => panic!("Expected SkipTasks call"),
        }
    }
}
