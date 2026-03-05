# Mock Data System Architecture

## Overview

This document describes a comprehensive mock data system for Villalobos that enables testing and development without requiring live AI agents or external services. The system simulates:

- **ACP (Agent Control Protocol)** interactions with agents
- **MCP Server** tool calls and responses
- **Unix Socket** communication between components
- **Agent behaviors** including planners, orchestrators, and implementers

## Design Goals

1. **Lean**: Minimal code overhead, reuse existing types
2. **Thorough**: Cover all major flows and edge cases
3. **Flexible**: Support multiple testing scenarios (unit, integration, e2e)
4. **Deterministic**: Reproducible tests with predictable mock responses
5. **Configurable**: Easy to define custom scenarios via config files

---

## Current Implementation Status

### ✅ Completed Components

| Component | Location | Description |
|-----------|----------|-------------|
| `MockScenario` | `src/testing/scenario.rs` | Loads test scenarios from TOML files |
| `MockAcpClient` | `src/testing/mock_acp.rs` | Simulates ACP communication with scripted responses |
| `MockToolInterceptor` | `src/testing/harness.rs` | Captures and responds to tool calls |
| `TestHarness` | `src/testing/harness.rs` | Orchestrates test runs with full App integration |
| `AcpClientTrait` | `src/acp.rs` | Trait abstraction for dependency injection |
| `App::with_mock_clients` | `src/app.rs` | Enables mock injection into App |
| Builder Helpers | `src/testing/builders.rs` | Fluent APIs (`MockSessionBuilder`, `MockToolResponseBuilder`) |
| Core Mock Types | `src/testing/types.rs` | `MockSessionUpdate`, `MockAgentSession`, `MockToolCallResponse`, etc. |

### ✅ Implemented Scenarios

| Scenario | File | What It Tests |
|----------|------|---------------|
| Simple Implementation | `tests/scenarios/simple_implement.toml` | Basic single task: Planner → Orchestrator → Implementer |
| Planning Only | `tests/scenarios/planning_only.toml` | Plan creation without implementation |
| Multiple Implementations | `tests/scenarios/multi_implement.toml` | Orchestrator with 3 sequential implement() calls |
| Nested Decomposition | `tests/scenarios/nested_decompose.toml` | Complex task decomposition with sub-orchestrator |
| Error Recovery | `tests/scenarios/error_recovery.toml` | Implementation failure and retry |
| Planner Failure | `tests/scenarios/planner_failure.toml` | Planner fails to create a plan |

### ✅ Integration Tests

All tests are located in `src/testing/mod.rs` under `#[cfg(test)]` and include:
- Unit tests for scenario loading and mock clients
- Integration tests for orchestration flows
- End-to-end tests for complete task lifecycles

---

## 1. Mock Data Structures

### 1.1 Core Mock Types

The mock system extends existing types rather than replacing them:

```rust
// src/testing/mock_types.rs

use crate::types::{Plan, PlanEntry, TaskResult};
use crate::mcp_server::{ToolCall, ToolRequest, ToolResponse};
use serde::{Deserialize, Serialize};

/// A scripted ACP session update message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockSessionUpdate {
    /// Delay before sending this update (milliseconds)
    pub delay_ms: u64,
    /// The session update type (e.g., "plan", "agent_message_chunk", "agent_turn_finished")
    pub session_update: String,
    /// Optional plan entries (for "plan" updates)
    pub entries: Option<Vec<PlanEntry>>,
    /// Optional text content (for "agent_message_chunk" updates)
    pub content: Option<String>,
    /// Optional tool call info (for "tool_call" updates)
    pub tool_title: Option<String>,
    /// Optional tool result (for "tool_result" updates)
    pub tool_result: Option<MockToolResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockToolResult {
    pub title: String,
    pub is_error: bool,
    pub content: String,
}

/// A complete mock agent session (planner, orchestrator, or implementer)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockAgentSession {
    /// Session ID to use
    pub session_id: String,
    /// Sequence of updates this session will produce
    pub updates: Vec<MockSessionUpdate>,
    /// Expected prompt patterns (for validation)
    pub expected_prompt_contains: Option<Vec<String>>,
}

/// Mock response for MCP tool calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockToolCallResponse {
    /// Pattern to match against the tool call (regex on task string)
    pub task_pattern: Option<String>,
    /// The tool call type this responds to
    pub tool_type: MockToolType,
    /// The response to return
    pub response: ToolResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MockToolType {
    Decompose,
    Implement,
    Complete,
}
```

### 1.2 Mock ACP Responses

```rust
/// Scripted ACP JSON-RPC responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockAcpResponse {
    /// The method this responds to (e.g., "session/new", "initialize")
    pub method: String,
    /// The result to return
    pub result: serde_json::Value,
    /// Optional error to return instead of result
    pub error: Option<MockAcpError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockAcpError {
    pub code: i32,
    pub message: String,
}
```

---

## 2. Configuration System

### 2.1 Scenario Configuration Format

Mock scenarios are defined in TOML files for readability:

```toml
# tests/scenarios/simple_implement.toml

[scenario]
name = "simple_implement"
description = "Single task that gets implemented directly"

[[planner_sessions]]
session_id = "planner-001"
[[planner_sessions.updates]]
delay_ms = 100
session_update = "agent_message_chunk"
content = "Creating plan..."

[[planner_sessions.updates]]
delay_ms = 200
session_update = "plan"
[planner_sessions.updates.entries]
entries = [
    { content = "Add error handling to login function", priority = "high", status = "not_started" }
]

[[planner_sessions.updates]]
delay_ms = 50
session_update = "agent_turn_finished"

[[orchestrator_sessions]]
session_id = "orchestrator-001"
# Orchestrator calls implement() tool, handled via mock_tool_responses

[[implementer_sessions]]
session_id = "implementer-001"
[[implementer_sessions.updates]]
delay_ms = 500
session_update = "agent_message_chunk"
content = "Implementing error handling..."

[[implementer_sessions.updates]]
delay_ms = 100
session_update = "tool_call"
tool_title = "str-replace-editor"

[[implementer_sessions.updates]]
delay_ms = 200
session_update = "tool_result"
[implementer_sessions.updates.tool_result]
title = "str-replace-editor"
is_error = false
content = "File updated successfully"

[[implementer_sessions.updates]]
delay_ms = 100
session_update = "agent_turn_finished"

[[mock_tool_responses]]
tool_type = "Implement"
task_pattern = ".*error handling.*"
[mock_tool_responses.response]
request_id = "" # Will be filled dynamically
success = true
summary = "Added try-catch blocks and error logging"
files_modified = ["src/auth/login.rs"]
```

### 2.2 Scenario Loader

```rust
// src/testing/scenario_loader.rs

use std::path::Path;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockScenario {
    pub name: String,
    pub description: String,
    pub planner_sessions: Vec<MockAgentSession>,
    pub orchestrator_sessions: Vec<MockAgentSession>,
    pub implementer_sessions: Vec<MockAgentSession>,
    pub mock_tool_responses: Vec<MockToolCallResponse>,
}

impl MockScenario {
    /// Load a scenario from a TOML file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let scenario: MockScenario = toml::from_str(&content)?;
        Ok(scenario)
    }

    /// Load a scenario from embedded test data
    pub fn from_str(toml_content: &str) -> Result<Self> {
        Ok(toml::from_str(toml_content)?)
    }
}
```

---

## 3. Integration Points

### 3.1 Mock ACP Client

The key integration point is replacing `AcpClient` with a mock version:

```rust
// src/testing/mock_acp.rs

use tokio::sync::mpsc;
use serde_json::Value;

/// Trait for ACP client behavior (real or mock)
pub trait AcpClientTrait {
    async fn initialize(&mut self) -> Result<()>;
    async fn session_new(&mut self, model: &str, mcp_servers: Vec<Value>, cwd: &str)
        -> Result<SessionNewResponse>;
    async fn session_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()>;
    async fn recv(&mut self) -> Result<Value>;
    async fn shutdown(&mut self) -> Result<()>;
}

/// Mock ACP client that returns scripted responses
pub struct MockAcpClient {
    /// Queue of session updates to return from recv()
    update_queue: VecDeque<(Duration, Value)>,
    /// Next session ID to return
    next_session_id: String,
    /// Captured prompts for assertion
    captured_prompts: Vec<(String, String)>, // (session_id, prompt)
    /// Session counter for generating unique IDs
    session_counter: usize,
}

impl MockAcpClient {
    pub fn new() -> Self {
        Self {
            update_queue: VecDeque::new(),
            next_session_id: "mock-session-001".to_string(),
            captured_prompts: Vec::new(),
            session_counter: 0,
        }
    }

    /// Load scripted behavior from a scenario
    pub fn with_scenario(scenario: &MockScenario, agent_type: AgentType) -> Self {
        let sessions = match agent_type {
            AgentType::Planner => &scenario.planner_sessions,
            AgentType::Orchestrator => &scenario.orchestrator_sessions,
            AgentType::Implementer => &scenario.implementer_sessions,
        };
        // Convert sessions to update queue...
        todo!()
    }

    /// Get captured prompts for assertions
    pub fn captured_prompts(&self) -> &[(String, String)] {
        &self.captured_prompts
    }
}
```

### 3.2 Mock MCP Socket

For testing socket communication without real IPC:

```rust
// src/testing/mock_socket.rs

use crate::mcp_server::{ToolRequest, ToolResponse};
use tokio::sync::mpsc;

/// In-memory channel-based mock for Unix socket communication
pub struct MockSocketPair {
    /// Send tool requests (simulates MCP server -> App)
    pub request_tx: mpsc::Sender<ToolRequest>,
    /// Receive tool requests (App side)
    pub request_rx: mpsc::Receiver<ToolRequest>,
    /// Send responses (simulates App -> MCP server)
    pub response_tx: mpsc::Sender<ToolResponse>,
    /// Receive responses (MCP server side)
    pub response_rx: mpsc::Receiver<ToolResponse>,
}

impl MockSocketPair {
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel(100);
        let (response_tx, response_rx) = mpsc::channel(100);
        Self { request_tx, request_rx, response_tx, response_rx }
    }
}
```

### 3.3 Dependency Injection Pattern

The `App` struct should accept trait objects for testability:

```rust
// Proposed refactor for src/app.rs

pub struct App<A: AcpClientTrait = AcpClient> {
    acp_orchestrator: A,
    acp_worker: A,
    // ... rest of fields
}

impl App<AcpClient> {
    /// Production constructor (current behavior)
    pub async fn new(model_config: ModelConfig) -> Result<Self> { ... }
}

impl<A: AcpClientTrait> App<A> {
    /// Test constructor with injected mocks
    pub fn with_mocks(
        orchestrator: A,
        worker: A,
        model_config: ModelConfig,
    ) -> Self { ... }
}
```

---

## 4. Storage Format

### 4.1 Directory Structure

```
tests/
├── scenarios/
│   ├── simple_implement.toml       # Single task implementation
│   ├── nested_decompose.toml       # Task with subtasks
│   ├── parallel_tasks.toml         # Concurrent implementations
│   ├── error_recovery.toml         # Failure and retry scenarios
│   └── timeout.toml                # Timeout handling
├── fixtures/
│   ├── plans/
│   │   ├── simple_plan.json        # Reusable plan data
│   │   └── complex_plan.json
│   ├── acp_responses/
│   │   ├── session_new.json        # Sample ACP responses
│   │   └── initialize.json
│   └── tool_responses/
│       ├── implement_success.json
│       └── decompose_success.json
└── integration/
    ├── mod.rs
    ├── planning_test.rs
    ├── orchestration_test.rs
    └── e2e_test.rs
```

### 4.2 JSON Fixtures

For simpler cases, raw JSON fixtures can be used:

```json
// tests/fixtures/plans/simple_plan.json
{
  "entries": [
    {
      "content": "Add input validation to API endpoints",
      "priority": "high",
      "status": "not_started"
    },
    {
      "content": "Write unit tests for validation logic",
      "priority": "medium",
      "status": "not_started"
    }
  ]
}
```

### 4.3 Snapshot Testing Support

For complex outputs, support snapshot comparisons:

```rust
// Integration with insta or similar
#[test]
fn test_plan_output() {
    let result = run_with_mock_scenario("simple_implement");
    insta::assert_json_snapshot!(result);
}
```

---

## 5. Usage Patterns

### 5.1 Unit Tests

Test individual components with minimal mocking:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockAcpClient, mock_plan};

    #[tokio::test]
    async fn test_wait_for_plan_parses_entries() {
        let plan = mock_plan(vec![
            ("Implement feature X", "high", "not_started"),
            ("Write tests", "medium", "not_started"),
        ]);

        let mut mock_acp = MockAcpClient::new();
        mock_acp.queue_plan_response("session-1", plan.clone());

        let result = wait_for_plan(&mut mock_acp, "session-1").await.unwrap();

        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].content, "Implement feature X");
    }
}
```

### 5.2 Integration Tests

Test component interactions:

```rust
// tests/integration/orchestration_test.rs

use villalobos::testing::{MockScenario, TestHarness};

#[tokio::test]
async fn test_orchestrator_delegates_to_implementer() {
    let scenario = MockScenario::from_file("tests/scenarios/simple_implement.toml").unwrap();
    let harness = TestHarness::with_scenario(scenario);

    let result = harness.run_goal("Add logging to the application").await;

    assert!(result.is_ok());
    let task_result = result.unwrap();
    assert!(task_result.success);

    // Verify expected interactions
    assert!(harness.planner_was_spawned());
    assert!(harness.implementer_was_spawned());
    assert_eq!(harness.tool_calls().len(), 2); // implement + complete
}
```

### 5.3 End-to-End Tests

Full flow tests with mocked external dependencies:

```rust
// tests/e2e/full_flow_test.rs

#[tokio::test]
async fn test_complex_decomposition_flow() {
    let scenario = MockScenario::from_file("tests/scenarios/nested_decompose.toml").unwrap();
    let harness = TestHarness::with_scenario(scenario)
        .with_timeout(Duration::from_secs(30));

    let result = harness.run_goal(
        "Build a complete authentication system with login, registration, and password reset"
    ).await;

    assert!(result.is_ok());

    // Verify decomposition happened
    let decompose_calls: Vec<_> = harness.tool_calls()
        .iter()
        .filter(|c| matches!(c, ToolCall::Decompose { .. }))
        .collect();
    assert!(!decompose_calls.is_empty());

    // Verify all subtasks were implemented
    let implement_calls = harness.implement_calls();
    assert!(implement_calls.len() >= 3);
}
```

### 5.4 Error Scenario Tests

```rust
#[tokio::test]
async fn test_handles_implementer_failure() {
    let scenario = MockScenario::from_file("tests/scenarios/error_recovery.toml").unwrap();
    let harness = TestHarness::with_scenario(scenario);

    let result = harness.run_goal("Fix the broken tests").await;

    // Should complete but report partial failure
    assert!(result.is_ok());
    let task_result = result.unwrap();
    assert!(!task_result.success);
    assert!(task_result.message.as_ref().unwrap().contains("failed"));
}

#[tokio::test]
async fn test_timeout_handling() {
    let scenario = MockScenario::from_file("tests/scenarios/timeout.toml").unwrap();
    let harness = TestHarness::with_scenario(scenario)
        .with_timeout(Duration::from_millis(100));

    let result = harness.run_goal("Long running task").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Timeout"));
}
```

---

## 6. Test Harness Implementation

### 6.1 TestHarness Structure

```rust
// src/testing/harness.rs

pub struct TestHarness {
    scenario: MockScenario,
    timeout: Duration,
    captured_tool_calls: Arc<Mutex<Vec<ToolCall>>>,
    planner_spawned: Arc<AtomicBool>,
    orchestrator_spawned: Arc<AtomicBool>,
    implementer_spawned: Arc<AtomicBool>,
}

impl TestHarness {
    pub fn with_scenario(scenario: MockScenario) -> Self {
        Self {
            scenario,
            timeout: Duration::from_secs(60),
            captured_tool_calls: Arc::new(Mutex::new(Vec::new())),
            planner_spawned: Arc::new(AtomicBool::new(false)),
            orchestrator_spawned: Arc::new(AtomicBool::new(false)),
            implementer_spawned: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn run_goal(&self, goal: &str) -> Result<TaskResult> {
        // Create mock ACP clients from scenario
        let mock_orchestrator = MockAcpClient::with_scenario(&self.scenario, AgentType::Orchestrator);
        let mock_worker = MockAcpClient::with_scenario(&self.scenario, AgentType::Worker);

        // Create app with mocks
        let model_config = ModelConfig::default();
        let mut app = App::with_mocks(mock_orchestrator, mock_worker, model_config);

        // Run with timeout
        tokio::time::timeout(self.timeout, app.run(goal)).await?
    }

    // Assertion helpers
    pub fn planner_was_spawned(&self) -> bool { ... }
    pub fn implementer_was_spawned(&self) -> bool { ... }
    pub fn tool_calls(&self) -> Vec<ToolCall> { ... }
    pub fn implement_calls(&self) -> Vec<String> { ... }
}
```

---

## 7. Builder Helpers

For programmatic test setup without TOML files:

```rust
// src/testing/builders.rs

/// Quick builder for mock plans
pub fn mock_plan(entries: Vec<(&str, &str, &str)>) -> Plan {
    Plan {
        entries: entries.into_iter().map(|(content, priority, status)| {
            PlanEntry {
                content: content.to_string(),
                priority: priority.to_string(),
                status: status.to_string(),
            }
        }).collect()
    }
}

/// Builder for mock tool responses
pub struct MockToolResponseBuilder {
    response: ToolResponse,
}

impl MockToolResponseBuilder {
    pub fn success(task: &str) -> Self {
        Self {
            response: ToolResponse::success(
                String::new(), // request_id filled at runtime
                format!("Completed: {}", task),
            )
        }
    }

    pub fn with_files(mut self, files: Vec<&str>) -> Self {
        self.response.files_modified = Some(files.iter().map(|s| s.to_string()).collect());
        self
    }

    pub fn failure(error: &str) -> Self {
        Self {
            response: ToolResponse::failure(String::new(), error.to_string())
        }
    }

    pub fn build(self) -> ToolResponse {
        self.response
    }
}

/// Session update sequence builder
pub struct MockSessionBuilder {
    updates: Vec<MockSessionUpdate>,
}

impl MockSessionBuilder {
    pub fn new() -> Self {
        Self { updates: Vec::new() }
    }

    pub fn message(mut self, text: &str) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms: 50,
            session_update: "agent_message_chunk".to_string(),
            content: Some(text.to_string()),
            ..Default::default()
        });
        self
    }

    pub fn plan(mut self, entries: Vec<(&str, &str)>) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms: 100,
            session_update: "plan".to_string(),
            entries: Some(entries.iter().map(|(c, p)| PlanEntry {
                content: c.to_string(),
                priority: p.to_string(),
                status: "not_started".to_string(),
            }).collect()),
            ..Default::default()
        });
        self
    }

    pub fn finish(mut self) -> Self {
        self.updates.push(MockSessionUpdate {
            delay_ms: 50,
            session_update: "agent_turn_finished".to_string(),
            ..Default::default()
        });
        self
    }

    pub fn build(self) -> MockAgentSession {
        MockAgentSession {
            session_id: uuid::Uuid::new_v4().to_string(),
            updates: self.updates,
            expected_prompt_contains: None,
        }
    }
}
```

---

## 8. Writing New Scenarios

### Scenario File Structure

Each scenario is a TOML file with the following sections:

```toml
# Metadata section (required)
[scenario]
name = "my_scenario"
description = "What this scenario tests"

# Agent sessions - define scripted behavior for each agent type
[[planner_sessions]]       # Planner agent behaviors
[[orchestrator_sessions]]  # Orchestrator agent behaviors
[[implementer_sessions]]   # Implementer agent behaviors

# Tool response mocks - define responses for MCP tool calls
[[mock_tool_responses]]    # Responses for implement/decompose/complete
```

### Session Update Types

Each session contains an array of `updates` that are delivered sequentially:

| `session_update` Value | Description | Required Fields |
|------------------------|-------------|-----------------|
| `"agent_message_chunk"` | Agent sends text | `content` |
| `"agent_turn_finished"` | Session complete | None |
| `"tool_call"` | Agent calls external tool | `tool_title` |
| `"tool_result"` | External tool returns | `tool_result.{title,is_error,content}` |

### Injecting MCP Tool Calls

Agents call Villalobos tools via `inject_mcp_tool_call`:

```toml
[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "Calling write_plan..."

# This injects a write_plan tool call from the agent
[planner_sessions.updates.inject_mcp_tool_call]
tool = "write_plan"
plan = "## Plan\n\n1. **Task 1**\n   - Details here"

[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "Calling complete..."

# This injects a complete tool call
[planner_sessions.updates.inject_mcp_tool_call]
tool = "complete"
success = true
message = "Plan created successfully"
```

Available tool injections:
- `tool = "write_plan"` + `plan` (planner only)
- `tool = "complete"` + `success` + `message` (all agents)
- `tool = "implement"` + `task` (orchestrator only)
- `tool = "decompose"` + `task` (orchestrator only)

### Mock Tool Responses

Define how the system responds to `implement` and `decompose` calls:

```toml
[[mock_tool_responses]]
tool_type = "Implement"                    # or "Decompose"
task_pattern = ".*error handling.*"        # regex to match task string

[mock_tool_responses.response]
success = true
summary = "Implemented the feature"
files_modified = ["src/file.rs"]

# For failures:
[[mock_tool_responses]]
tool_type = "Implement"
task_pattern = "^Fix.*$"

[mock_tool_responses.response]
success = false
summary = "Implementation failed"
error = "Could not parse config file"
```

### Example: Minimal Scenario

```toml
# tests/scenarios/minimal.toml
[scenario]
name = "minimal"
description = "Absolute minimum successful scenario"

[[planner_sessions]]
session_id = "planner-001"

[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "Creating plan..."

[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "[write_plan]"

[planner_sessions.updates.inject_mcp_tool_call]
tool = "write_plan"
plan = "1. Do the thing"

[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "[complete]"

[planner_sessions.updates.inject_mcp_tool_call]
tool = "complete"
success = true
message = "Done"

# CRITICAL: Must end with agent_turn_finished!
[[planner_sessions.updates]]
session_update = "agent_turn_finished"

[[orchestrator_sessions]]
session_id = "orchestrator-001"

[[orchestrator_sessions.updates]]
session_update = "agent_message_chunk"
content = "Executing..."

[[orchestrator_sessions.updates]]
session_update = "agent_message_chunk"
content = "[implement]"

[orchestrator_sessions.updates.inject_mcp_tool_call]
tool = "implement"
task = "Do the thing"

[[orchestrator_sessions.updates]]
session_update = "agent_message_chunk"
content = "[complete]"

[orchestrator_sessions.updates.inject_mcp_tool_call]
tool = "complete"
success = true
message = "All done"

[[orchestrator_sessions.updates]]
session_update = "agent_turn_finished"

[[implementer_sessions]]
session_id = "implementer-001"

[[implementer_sessions.updates]]
session_update = "agent_message_chunk"
content = "Implementing..."

[[implementer_sessions.updates]]
session_update = "agent_message_chunk"
content = "[complete]"

[implementer_sessions.updates.inject_mcp_tool_call]
tool = "complete"
success = true
message = "Feature implemented"

[[implementer_sessions.updates]]
session_update = "agent_turn_finished"

[[mock_tool_responses]]
tool_type = "Implement"

[mock_tool_responses.response]
success = true
summary = "Did the thing"
files_modified = ["src/main.rs"]
```

---

## 9. Writing Tests with TestHarness

### Basic Test Pattern

```rust
use std::path::Path;
use std::time::Duration;
use villalobos::testing::{TestHarness, assert_success, assert_planner_spawned};

#[tokio::test]
async fn test_my_scenario() {
    // 1. Load the scenario
    let mut harness = TestHarness::with_scenario_file(
        Path::new("tests/scenarios/my_scenario.toml")
    ).expect("Failed to load scenario");

    // 2. Configure timeout (optional, default is 60s)
    let harness = harness.with_timeout(Duration::from_secs(10));
    let mut harness = harness;

    // 3. Run the test
    let result = harness.run_goal("The goal to execute")
        .await
        .expect("Test run failed");

    // 4. Assert the results
    assert_success(&result);
    assert_planner_spawned(&result);
}
```

### Available Assertion Helpers

```rust
use villalobos::testing::{
    assert_success,              // Task completed successfully
    assert_failure,              // Task failed (expected failure)
    assert_planner_spawned,      // Planner session was created
    assert_orchestrator_spawned, // Orchestrator session was created
    assert_implementer_spawned,  // Implementer session was created
    assert_implement_called,     // At least one implement() call made
    assert_decompose_called,     // At least one decompose() call made
};
```

### TestRunResult Methods

The `run_goal()` method returns a `TestRunResult` with:

```rust
result.task_result.success       // bool: did the task succeed?
result.task_result.message       // Option<String>: completion message

result.planner_was_spawned()     // bool
result.orchestrator_was_spawned() // bool
result.implementer_was_spawned() // bool

result.implement_calls()         // Vec<String>: task strings from implement()
result.decompose_calls()         // Vec<String>: task strings from decompose()

result.tool_calls                // Vec<CapturedToolCall>: all tool calls
result.sessions_created          // Vec<String>: all session IDs
result.prompts_sent              // Vec<(String, String)>: (session_id, prompt)
```

### Testing Specific Behaviors

```rust
#[tokio::test]
async fn test_multiple_implementations() {
    let mut harness = TestHarness::with_scenario_file(
        Path::new("tests/scenarios/multi_implement.toml")
    ).expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));
    let result = harness.run_goal("Build multi-component system").await
        .expect("Test should complete");

    // Check specific number of implement calls
    let impl_calls = result.implement_calls();
    assert_eq!(impl_calls.len(), 3, "Expected 3 implement calls");

    // Check specific task patterns
    assert!(impl_calls.iter().any(|t| t.contains("database")));
    assert!(impl_calls.iter().any(|t| t.contains("service")));
    assert!(impl_calls.iter().any(|t| t.contains("API")));
}

#[tokio::test]
async fn test_error_recovery() {
    let mut harness = TestHarness::with_scenario_file(
        Path::new("tests/scenarios/error_recovery.toml")
    ).expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));
    let result = harness.run_goal("Fix the bug").await
        .expect("Test should complete");

    // First implement failed, retry succeeded
    let impl_calls = result.implement_calls();
    assert_eq!(impl_calls.len(), 2, "Expected initial + retry");
    assert_success(&result);
}
```

---

## 10. Common Pitfalls

### ⚠️ Missing `agent_turn_finished`

**Problem**: Session hangs forever waiting for completion signal.

**Symptom**: Test times out, no error message.

**Solution**: ALWAYS end each session with:
```toml
[[planner_sessions.updates]]
session_update = "agent_turn_finished"
```

### ⚠️ Exhausted Mock Responses

**Problem**: More tool calls than mock responses defined.

**Symptom**: Error message about "exhausted mock responses" or "no matching response".

**Solution**: Ensure `[[mock_tool_responses]]` count matches expected `implement()`/`decompose()` calls.

### ⚠️ Wrong Session Order

**Problem**: Sessions are consumed in array order, not matched by content.

**Symptom**: Wrong session used for an agent type.

**Solution**: Define sessions in the order they will be spawned. For nested decomposition:
1. Main planner
2. Sub-planner (from decompose)
3. Main orchestrator
4. Sub-orchestrator (from decompose)
5. Implementers in call order

### ⚠️ Missing Tool Injection

**Problem**: Agent session ends without calling `complete`.

**Symptom**: Task result missing or incorrect.

**Solution**: Every session must call `complete` via `inject_mcp_tool_call`:
```toml
[planner_sessions.updates.inject_mcp_tool_call]
tool = "complete"
success = true
message = "Done"
```

### ⚠️ Task Pattern Mismatch

**Problem**: `task_pattern` regex doesn't match the actual task string.

**Symptom**: "No matching mock response" error.

**Solution**: Use `.*` liberally or exact patterns. Test regex separately.
```toml
# Too specific (fragile):
task_pattern = "Implement login flow with JWT token generation and session management"

# Better (robust):
task_pattern = ".*login.*JWT.*"

# Match anything (fallback):
# omit task_pattern entirely
```

---

## 11. Example Test Scenarios

### Scenario: Simple Task Implementation
```
Goal: "Add a new utility function"
Flow: Planner → Plan (1 task) → Orchestrator → implement() → Complete
```

### Scenario: Task Decomposition
```
Goal: "Build user profile feature"
Flow: Planner → Plan (3 tasks) → Orchestrator → implement() x3 → Complete
```

### Scenario: Nested Decomposition
```
Goal: "Build complete auth system"
Flow: Planner → Plan (2 tasks) → Orchestrator → decompose(task1)
      → Planner → Plan (2 subtasks) → Orchestrator → implement() x2
      → implement(task2) → Complete
```

### Scenario: Parallel Execution
```
Goal: "Add tests for all modules"
Flow: Planner → Plan (4 tasks) → Orchestrator → implement() x4 (concurrent) → Complete
```

### Scenario: Failure Recovery
```
Goal: "Fix critical bugs"
Flow: Planner → Plan → Orchestrator → implement() [FAILS] → Complete(success=false)
```

---

## 12. Testing Best Practices

1. **Isolate tests**: Each test should use its own scenario/fixtures
2. **Use builders**: Prefer programmatic builders over complex TOML for simple tests
3. **Assert behaviors**: Test what happened (tool calls made) not just outcomes
4. **Test edge cases**: Timeouts, empty plans, failure recovery
5. **Keep scenarios small**: One scenario per behavior, compose for complex flows
6. **Document scenarios**: Include description field in all TOML scenarios
7. **Version fixtures**: Keep fixtures in sync with type changes
8. **Always verify agent_turn_finished**: Every session must end with this update
9. **Match mock response count**: Number of `mock_tool_responses` must match tool calls

---

## Appendix: Quick Reference

### Running Tests

```bash
# Run all tests
cargo test

# Run only mock system tests
cargo test --features testing

# Run a specific integration test
cargo test test_simple_implement_flow

# Run E2E tests with verbose output
cargo test e2e -- --nocapture
```

### Key Types to Mock

| Component | Real Type | Mock Type | Purpose |
|-----------|-----------|-----------|---------|
| ACP Client | `AcpClient` | `MockAcpClient` | Simulate agent spawning/messaging |
| Tool Calls | `ToolCall` | `MockToolCallResponse` | Script MCP tool responses |
| Plans | `Plan` | `mock_plan()` helper | Create test plans quickly |
| Sessions | `session/update` | `MockSessionUpdate` | Script agent update sequences |

### Common Test Patterns

```rust
// Integration test with scenario file (recommended)
let mut harness = TestHarness::with_scenario_file(
    Path::new("tests/scenarios/my_test.toml")
).expect("Failed to load scenario");

let mut harness = harness.with_timeout(Duration::from_secs(10));
let result = harness.run_goal("My goal").await
    .expect("Test run failed");

assert_success(&result);
assert_planner_spawned(&result);

// Inline scenario for focused tests
let scenario = MockScenario::builder()
    .with_planner(MockSessionBuilder::new()
        .message("Planning...")
        .plan(vec![("Step 1", "high")])
        .finish()
        .build())
    .with_tool_response(MockToolResponseBuilder::success("Step 1").build())
    .build();
```

