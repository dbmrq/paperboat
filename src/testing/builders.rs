//! Builder helpers for the mock testing system.
//!
//! This module provides convenient builder patterns and helper functions
//! for constructing mock test data programmatically.

use super::types::{
    MockAgentSession, MockMcpToolCall, MockSessionUpdate, MockSuggestedTask, MockToolCallResponse,
    MockToolResponseData, MockToolResult, MockToolType,
};

// ============================================================================
// Builder Helpers
// ============================================================================

/// Builder for creating mock tool call responses.
#[derive(Debug, Default)]
pub struct MockToolResponseBuilder {
    task_pattern: Option<String>,
    tool_type: Option<MockToolType>,
    success: bool,
    summary: String,
    files_modified: Option<Vec<String>>,
    error: Option<String>,
}

impl MockToolResponseBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the task pattern (regex) to match.
    pub fn task_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.task_pattern = Some(pattern.into());
        self
    }

    /// Set the tool type this responds to.
    pub const fn tool_type(mut self, tool_type: MockToolType) -> Self {
        self.tool_type = Some(tool_type);
        self
    }

    /// Set as a successful response.
    pub fn success(mut self, summary: impl Into<String>) -> Self {
        self.success = true;
        self.summary = summary.into();
        self.error = None;
        self
    }

    /// Set as a failure response.
    pub fn failure(mut self, error: impl Into<String>) -> Self {
        self.success = false;
        self.error = Some(error.into());
        self
    }

    /// Set the list of modified files.
    pub fn files_modified(mut self, files: Vec<String>) -> Self {
        self.files_modified = Some(files);
        self
    }

    /// Build the `MockToolCallResponse`.
    pub fn build(self) -> MockToolCallResponse {
        MockToolCallResponse {
            task_pattern: self.task_pattern,
            tool_type: self.tool_type.unwrap_or(MockToolType::SpawnAgents),
            response: MockToolResponseData {
                success: self.success,
                summary: self.summary,
                files_modified: self.files_modified,
                error: self.error,
            },
        }
    }
}

/// Builder for creating mock agent sessions.
#[derive(Debug, Default)]
pub struct MockSessionBuilder {
    session_id: String,
    updates: Vec<MockSessionUpdate>,
    expected_prompt_contains: Option<Vec<String>>,
}

impl MockSessionBuilder {
    /// Create a new builder with a session ID.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            ..Default::default()
        }
    }

    /// Add a message chunk update.
    pub fn with_message_chunk(mut self, content: impl Into<String>, delay_ms: u64) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(),
            content: Some(content.into()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: None,
        });
        self
    }

    /// Add a tool call update.
    pub fn with_tool_call(mut self, tool_title: impl Into<String>, delay_ms: u64) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "tool_call".to_string(),
            content: None,
            tool_title: Some(tool_title.into()),
            tool_result: None,
            inject_mcp_tool_call: None,
        });
        self
    }

    /// Add a tool result update.
    pub fn with_tool_result(
        mut self,
        title: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
        delay_ms: u64,
    ) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "tool_result".to_string(),
            content: None,
            tool_title: None,
            tool_result: Some(MockToolResult {
                title: title.into(),
                is_error,
                content: content.into(),
            }),
            inject_mcp_tool_call: None,
        });
        self
    }

    /// Add an agent turn finished update.
    pub fn with_turn_finished(mut self, delay_ms: u64) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_turn_finished".to_string(),
            content: None,
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: None,
        });
        self
    }

    /// Add a `create_task` MCP tool call injection.
    /// This simulates the planner agent calling `create_task()`.
    pub fn with_create_task(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        delay_ms: u64,
    ) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(), // Dummy update to trigger injection
            content: Some("[calling create_task]".to_string()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: Some(MockMcpToolCall::CreateTask {
                name: name.into(),
                description: description.into(),
                dependencies: vec![],
            }),
        });
        self
    }

    /// Add a complete MCP tool call injection.
    /// This simulates any agent calling `complete()`.
    pub fn with_complete(mut self, success: bool, message: Option<String>, delay_ms: u64) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(), // Dummy update to trigger injection
            content: Some("[calling complete]".to_string()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: Some(MockMcpToolCall::Complete {
                success,
                message,
                notes: None,
                add_tasks: None,
            }),
        });
        self
    }

    /// Add a complete MCP tool call with suggested tasks.
    /// This simulates an agent calling `complete()` with `add_tasks`.
    pub fn with_complete_and_tasks(
        mut self,
        success: bool,
        message: Option<String>,
        add_tasks: Vec<MockSuggestedTask>,
        delay_ms: u64,
    ) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(),
            content: Some("[calling complete with suggested tasks]".to_string()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: Some(MockMcpToolCall::Complete {
                success,
                message,
                notes: None,
                add_tasks: Some(add_tasks),
            }),
        });
        self
    }

    /// Add an implement MCP tool call injection.
    /// This simulates the orchestrator agent calling `implement()`.
    pub fn with_implement(mut self, task: impl Into<String>, delay_ms: u64) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(), // Dummy update to trigger injection
            content: Some("[calling implement]".to_string()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: Some(MockMcpToolCall::SpawnAgents { task: task.into() }),
        });
        self
    }

    /// Add a decompose MCP tool call injection.
    /// This simulates the orchestrator agent calling `decompose()`.
    pub fn with_decompose(mut self, task: impl Into<String>, delay_ms: u64) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(), // Dummy update to trigger injection
            content: Some("[calling decompose]".to_string()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: Some(MockMcpToolCall::Decompose { task: task.into() }),
        });
        self
    }

    /// Add a `skip_tasks` MCP tool call injection.
    /// This simulates the orchestrator agent calling `skip_tasks()`.
    pub fn with_skip_tasks(
        mut self,
        task_ids: Vec<String>,
        reason: Option<String>,
        delay_ms: u64,
    ) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms,
            session_update: "agent_message_chunk".to_string(), // Dummy update to trigger injection
            content: Some("[calling skip_tasks]".to_string()),
            tool_title: None,
            tool_result: None,
            inject_mcp_tool_call: Some(MockMcpToolCall::SkipTasks { task_ids, reason }),
        });
        self
    }

    /// Set expected prompt patterns for validation.
    pub fn expect_prompt_contains(mut self, patterns: Vec<String>) -> Self {
        self.expected_prompt_contains = Some(patterns);
        self
    }

    /// Build the `MockAgentSession`.
    pub fn build(self) -> MockAgentSession {
        MockAgentSession {
            session_id: self.session_id,
            updates: self.updates,
            expected_prompt_contains: self.expected_prompt_contains,
        }
    }
}
