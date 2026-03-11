# Test Scenarios

This directory contains TOML scenario files that define scripted behaviors for integration and end-to-end tests.

## Available Scenarios

| Scenario File | Description | Agent Flow |
|---------------|-------------|------------|
| `simple_implement.toml` | Basic single task implementation | Planner → Orchestrator → 1 Implementer |
| `planning_only.toml` | Plan creation without implementation | Planner → Orchestrator (complete immediately) |
| `multi_implement.toml` | Multiple sequential tasks | Planner → Orchestrator → 3 Implementers |
| `nested_decompose.toml` | Complex task decomposition | Planner → Orchestrator → decompose() → Sub-Planner → Sub-Orchestrator → Implementers |
| `concurrent_agents.toml` | Multiple concurrent agents | Planner → Orchestrator → 3 Implementers (parallel) |
| `error_recovery.toml` | Implementation failure and retry | Planner → Orchestrator → Fail → Retry → Success |
| `planner_failure.toml` | Planner fails to create plan | Planner (complete with failure) |
| `completion_rejection.toml` | Orchestrator completion rejected with pending tasks | Planner → Orchestrator (rejected) → skip_tasks → Complete |
| `wait_mode_any.toml` | Test spawn_agents with wait=any mode | Planner → Orchestrator → 2 Implementers (first result returns) |
| `wait_mode_all_mixed.toml` | Test spawn_agents with wait=all under mixed results | Planner → Orchestrator → 3 Implementers (mixed success/failure) |
| `session_drain.toml` | Session drain handles racing notifications | Planner → Orchestrator → 1 Implementer (with late updates) |
| `agent_spawn_failure.toml` | Agent spawn fails at startup | Planner → Orchestrator → Fail → 1 Implementer (partial success) |
| `early_socket_close.toml` | Graceful handling of early socket close | Planner → Orchestrator → Implementer (interrupted) |

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

### `concurrent_agents.toml`
Tests concurrent execution of multiple agents:
1. Planner creates a 3-task plan with independent tasks
2. Orchestrator spawns 3 agents concurrently
3. All implementers run in parallel
4. Results are aggregated
5. Orchestrator completes successfully

**Use for**: Verifying concurrent agent spawning and execution.

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

### `completion_rejection.toml`
Tests that orchestrator completion is rejected when tasks remain pending:
1. Planner creates 2 tasks
2. Orchestrator implements Task 1 only
3. Orchestrator calls `complete(success=true)` - REJECTED (pending tasks)
4. Orchestrator calls `skip_tasks` for Task 2
5. Orchestrator calls `complete(success=true)` - ACCEPTED

**Use for**: Verifying task reconciliation before completion.

### `wait_mode_any.toml`
Tests `spawn_agents` with `wait=any` mode where first result returns immediately:
1. Planner creates 2 tasks
2. Orchestrator spawns 2 agents with `wait=any`
3. First agent fails quickly
4. `wait=any` returns immediately with failure result
5. Second agent continues in background
6. Orchestrator handles result and completes

**Use for**: Verifying wait=any semantics return first completion.

### `wait_mode_all_mixed.toml`
Tests `spawn_agents` with `wait=all` mode under mixed success/failure:
1. Planner creates 3 tasks
2. Orchestrator spawns 3 agents with `wait=all` (default)
3. Agent 1: succeeds, Agent 2: fails, Agent 3: succeeds
4. `wait=all` returns with mixed results
5. Orchestrator handles mixed results appropriately

**Use for**: Verifying wait=all waits for all agents and handles mixed results.

### `session_drain.toml`
Tests behavior when notifications arrive after a session ends:
1. Planner creates tasks and completes quickly
2. Late notifications arrive after `turn_finished`
3. Session drain handles queued updates correctly
4. Orchestrator sees consistent state after drain

**Use for**: Verifying no data loss from racing session updates.

### `agent_spawn_failure.toml`
Tests behavior when agent spawn fails (model creation error, session startup failure):
1. Planner creates 2 tasks
2. Orchestrator spawns agent for Task 1 - FAILS (simulated startup error)
3. Task 1 is marked as failed
4. Orchestrator handles failure, spawns agent for Task 2 - succeeds
5. Orchestrator completes with partial success

**Use for**: Verifying system remains functional after spawn failures.

### `early_socket_close.toml`
Tests cleanup behavior when socket closes while agents are running:
1. Planner creates task
2. Orchestrator spawns agent
3. Implementer starts but socket closes early (simulated via error)
4. System cleans up and reports appropriate status

**Use for**: Verifying graceful shutdown without hanging.

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

The scenario system supports the following features:
- **Session updates**: `agent_message_chunk`, `agent_turn_finished`
- **Injected MCP tool calls**: `complete`, `create_task`, `set_goal`
- **Mock tool responses**: Define responses for `implement()` and `decompose()` calls
- **Task patterns**: Regex matching for task-based responses

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

