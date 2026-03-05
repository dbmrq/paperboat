//! Core mock types for the testing system.
//!
//! This module contains all the fundamental data structures used
//! to represent mock sessions, tool responses, and ACP responses.

use crate::mcp_server::ToolResponse;
use serde::{Deserialize, Serialize};

// ============================================================================
// Core Mock Types
// ============================================================================

/// A scripted ACP session update message.
///
/// Represents a single update that would be received from an ACP session,
/// such as message chunks, tool calls, or completion signals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockSessionUpdate {
    /// Delay before sending this update (milliseconds).
    /// Used to simulate realistic timing in tests.
    #[serde(default)]
    pub delay_ms: u64,

    /// The session update type (e.g., "agent_message_chunk", "agent_turn_finished").
    pub session_update: String,

    /// Optional text content (for "agent_message_chunk" updates).
    #[serde(default)]
    pub content: Option<String>,

    /// Optional tool call info (for "tool_call" updates).
    #[serde(default)]
    pub tool_title: Option<String>,

    /// Optional tool result (for "tool_result" updates).
    #[serde(default)]
    pub tool_result: Option<MockToolResult>,

    /// Optional MCP tool call to inject (triggers tool call through the mock channel).
    /// This simulates the agent calling one of our MCP tools (write_plan, complete, implement, decompose).
    #[serde(default)]
    pub inject_mcp_tool_call: Option<MockMcpToolCall>,
}

/// An MCP tool call to inject during mock session execution.
/// This represents the agent calling one of our tools via the MCP protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum MockMcpToolCall {
    /// Call the write_plan tool (planner agent).
    WritePlan { plan: String },
    /// Call the complete tool (all agents).
    Complete { success: bool, message: Option<String> },
    /// Call the implement tool (orchestrator only).
    Implement { task: String },
    /// Call the decompose tool (orchestrator only).
    Decompose { task: String },
}

/// Tool result content for mock tool_result updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockToolResult {
    pub title: String,
    pub is_error: bool,
    pub content: String,
}

/// A complete mock agent session (planner, orchestrator, or implementer).
///
/// Defines a sequence of updates that a mock session will produce,
/// along with optional validation patterns for the prompts it receives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockAgentSession {
    /// Session ID to use.
    pub session_id: String,

    /// Sequence of updates this session will produce.
    #[serde(default)]
    pub updates: Vec<MockSessionUpdate>,

    /// Expected prompt patterns (for validation).
    /// If set, the mock will verify that prompts contain these substrings.
    #[serde(default)]
    pub expected_prompt_contains: Option<Vec<String>>,
}

/// Mock response for MCP tool calls.
///
/// Defines how the mock system should respond when an MCP tool is called.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockToolCallResponse {
    /// Pattern to match against the tool call (regex on task string).
    #[serde(default)]
    pub task_pattern: Option<String>,

    /// The tool call type this responds to.
    pub tool_type: MockToolType,

    /// The response to return.
    pub response: MockToolResponseData,
}

/// Tool types that can be mocked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MockToolType {
    Decompose,
    Implement,
    Complete,
}

/// Response data for mock tool calls (mirrors ToolResponse structure).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockToolResponseData {
    /// Whether the operation succeeded.
    pub success: bool,

    /// Human-readable summary of what was done.
    #[serde(default)]
    pub summary: String,

    /// Optional list of files that were modified.
    #[serde(default)]
    pub files_modified: Option<Vec<String>>,

    /// Optional error message if the operation failed.
    #[serde(default)]
    pub error: Option<String>,
}

impl MockToolResponseData {
    /// Convert to a ToolResponse with the given request ID.
    pub fn to_tool_response(&self, request_id: String) -> ToolResponse {
        ToolResponse {
            request_id,
            success: self.success,
            summary: self.summary.clone(),
            files_modified: self.files_modified.clone(),
            error: self.error.clone(),
        }
    }
}

/// Scripted ACP JSON-RPC responses.
///
/// Used to mock responses to ACP methods like "initialize" or "session/new".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockAcpResponse {
    /// The method this responds to (e.g., "session/new", "initialize").
    pub method: String,

    /// The result to return.
    #[serde(default)]
    pub result: serde_json::Value,

    /// Optional error to return instead of result.
    #[serde(default)]
    pub error: Option<MockAcpError>,
}

/// ACP error response structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockAcpError {
    pub code: i32,
    pub message: String,
}

/// Types of agents in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    Planner,
    Orchestrator,
    Implementer,
}

