# Test Scenarios

This directory contains TOML scenario files that define scripted behaviors for integration and end-to-end tests.

## Available Scenarios

| Scenario File | Description | Agent Flow |
|---------------|-------------|------------|
| `simple_implement.toml` | Basic single task implementation | Planner → Orchestrator → 1 Implementer |
| `planning_only.toml` | Plan creation without implementation | Planner → Orchestrator (complete immediately) |
| `multi_implement.toml` | Multiple sequential tasks | Planner → Orchestrator → 3 Implementers |
| `nested_decompose.toml` | Complex task decomposition | Planner → Orchestrator → decompose() → Sub-Planner → Sub-Orchestrator → Implementers |
| `error_recovery.toml` | Implementation failure and retry | Planner → Orchestrator → Fail → Retry → Success |
| `planner_failure.toml` | Planner fails to create plan | Planner (complete with failure) |

## What Each Scenario Tests

### `simple_implement.toml`
The most basic successful flow:
1. Planner creates a single-task plan via `write_plan`
2. Planner calls `complete(success=true)`
3. Orchestrator calls `implement(task)`
4. Implementer executes and calls `complete(success=true)`
5. Orchestrator calls `complete(success=true)`

**Use for**: Verifying basic end-to-end flow works.

### `planning_only.toml`
Tests planning without execution:
1. Planner creates a plan via `write_plan`
2. Planner calls `complete(success=true)`
3. Orchestrator immediately calls `complete(success=true)` without implementing

**Use for**: Testing plan-only mode or plan review scenarios.

### `multi_implement.toml`
Tests sequential implementation of multiple tasks:
1. Planner creates a 3-task plan
2. Orchestrator calls `implement()` three times sequentially
3. Each Implementer completes successfully
4. Orchestrator calls `complete(success=true)`

**Use for**: Verifying correct handling of multiple sequential tasks.

### `nested_decompose.toml`
Tests hierarchical task decomposition:
1. Main planner creates plan with a complex task
2. Main orchestrator calls `decompose()` for the complex task
3. Sub-planner creates subtask plan
4. Sub-orchestrator implements subtasks
5. Main orchestrator continues with remaining tasks

**Use for**: Testing decomposition logic and nested orchestration.

### `error_recovery.toml`
Tests failure handling and retry:
1. Planner creates plan
2. Orchestrator calls `implement()`
3. First implementer fails
4. Orchestrator retries with modified task
5. Second implementer succeeds
6. Orchestrator completes successfully

**Use for**: Verifying error recovery and retry logic.

### `planner_failure.toml`
Tests handling of planning failures:
1. Planner attempts to create plan
2. Planner calls `complete(success=false, message="error")`
3. Task fails with planner error

**Use for**: Testing graceful handling of planning failures.

## Running Tests

```bash
# Run all tests including scenario-based tests
cargo test --features testing

# Run a specific scenario test
cargo test test_simple_implement_flow

# Run E2E tests
cargo test e2e

# Run with verbose output to see scenario flow
cargo test test_simple_implement_flow -- --nocapture
```

## Writing a New Scenario

See [MOCK_DATA_SYSTEM_ARCHITECTURE.md](../../docs/MOCK_DATA_SYSTEM_ARCHITECTURE.md) for complete documentation on:
- Scenario file structure
- Session update types
- Injecting MCP tool calls
- Mock tool responses
- Common pitfalls

### Quick Start Template

```toml
[scenario]
name = "my_new_scenario"
description = "What this scenario tests"

# Planner session
[[planner_sessions]]
session_id = "planner-001"

[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "Creating plan..."

[planner_sessions.updates.inject_mcp_tool_call]
tool = "write_plan"
plan = "1. Task to do"

[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "[complete]"

[planner_sessions.updates.inject_mcp_tool_call]
tool = "complete"
success = true
message = "Plan created"

# CRITICAL: Always end with agent_turn_finished
[[planner_sessions.updates]]
session_update = "agent_turn_finished"

# Add orchestrator and implementer sessions similarly...
```

## Important Notes

1. **Order matters**: Sessions are consumed in array order, not matched by ID
2. **Always end sessions**: Every session must end with `agent_turn_finished`
3. **Match mock responses**: Number of `[[mock_tool_responses]]` must equal number of `implement()`/`decompose()` calls
4. **Test your patterns**: `task_pattern` uses regex - test patterns before relying on them

