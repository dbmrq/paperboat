# Villalobos Multi-Agent Orchestration System - Improvement Plan

## Overview

This plan addresses four major improvements to the Villalobos orchestration system:

1. **Concurrent Task Execution** - Enable parallel agent execution with implicit batching
2. **Structured Task Tracking** - Replace unstructured markdown plans with typed tasks
3. **Logging Coordination** - Clean up overlapping logging mechanisms
4. **Flexible Agent Spawning** - Support diverse agent types including ad-hoc agents

### Design Principles

- **Simplify, simplify, simplify** - Fewer tools, clearer responsibilities
- **Implicit batching** - `spawn_agents([...])` waits for all by default
- **Explicit control when needed** - Optional parameters for advanced cases
- **No backward compatibility burden** - Clean break from old patterns

### Goals and Success Criteria

- Orchestrator can run multiple agents concurrently (batch + wait for all)
- Tasks have structured state with dependencies
- UI can observe real-time task progress via LogEvent
- Clean separation: tracing for debug, events for UI, files for transcripts
- Orchestrator can spawn template-based or ad-hoc agents

### Tool Surface (Final State)

| Agent Type | Tools | Notes |
|------------|-------|-------|
| **Orchestrator** | `spawn_agents`, `decompose`, `complete` | 3 tools |
| **Planner** | `create_task`, `complete` | 2 tools, built-in task tools removed |
| **Implementer** | `complete` | 1 tool + all editing tools |
| **Verifier** | `complete` | 1 tool + read-only + launch-process |
| **Explorer** | `complete` | 1 tool + read-only + web access |
| **Custom** | `complete` | 1 tool + explicit tool whitelist |

---

## Priority Order and Dependencies

```
┌─────────────────────────────────────────────────────────────────┐
│ Phase 1: Session Routing Infrastructure                         │
│   - Foundation for concurrent sessions on single ACP client     │
│   - Route messages by sessionId to per-session channels         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 2: Concurrent Execution + Tool Simplification             │
│   - spawn_agents with implicit batch (wait for all)             │
│   - Remove implement tool, keep decompose                       │
│   - Completion channel for async results                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 3: Structured Task Tracking                               │
│   - Task struct with state and dependencies                     │
│   - create_task tool for planners                               │
│   - TaskManager for progress tracking                           │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 4: Agent Templates + Ad-hoc Agents                        │
│   - Built-in templates (verifier, explorer)                     │
│   - Custom agent support with explicit tool whitelist           │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ Phase 5: Logging Cleanup (Opportunistic)                        │
│   - Remove duplicate print!/event emissions                     │
│   - Ensure LogEvent covers all meaningful state changes         │
└─────────────────────────────────────────────────────────────────┘
```

---

## Phase 1: Session Routing Infrastructure

### 1.1 Problem Statement

Currently, when waiting for a session, messages for OTHER sessions are consumed and dropped:

```rust
// src/app/session_drain.rs:30-35
let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());
if msg_session_id != Some(session_id) {
    return Ok(false);  // Message consumed but ignored!
}
```

This prevents concurrent sessions from working correctly.

### 1.2 Solution: Session Router

Add a routing layer that dispatches messages to per-session channels:

```rust
pub struct SessionRouter {
    /// Map of session_id → sender for that session's messages
    routes: HashMap<String, mpsc::Sender<Value>>,
    /// Sender for session completion notifications to orchestrator
    completion_tx: mpsc::Sender<SessionCompletion>,
}

pub struct SessionCompletion {
    pub session_id: String,
    pub result: Result<SessionOutput, String>,
}

impl SessionRouter {
    /// Register a new session, returns receiver for its messages
    pub fn register(&mut self, session_id: String) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel(100);
        self.routes.insert(session_id, tx);
        rx
    }

    /// Unregister a session when complete
    pub fn unregister(&mut self, session_id: &str) {
        self.routes.remove(session_id);
    }

    /// Route an incoming message to the appropriate session
    pub async fn route(&self, msg: Value) {
        if let Some(session_id) = extract_session_id(&msg) {
            if let Some(tx) = self.routes.get(&session_id) {
                let _ = tx.send(msg).await;
            }
        }
    }
}
```

### 1.3 Integration with ACP Client

Spawn a background task that reads from `acp_worker.recv()` and routes:

```rust
// In App initialization
let (router, completion_rx) = SessionRouter::new();
let router = Arc::new(RwLock::new(router));

// Background routing task
let router_clone = router.clone();
let mut acp_worker_rx = acp_worker.take_notification_rx();
tokio::spawn(async move {
    while let Some(msg) = acp_worker_rx.recv().await {
        router_clone.read().await.route(msg).await;
    }
});
```

### 1.4 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `src/app/router.rs` | Create | SessionRouter implementation |
| `src/app/mod.rs` | Modify | Add router to App, spawn routing task |
| `src/acp.rs` | Modify | Allow taking notification_rx for routing |
| `src/app/session.rs` | Modify | Use per-session receiver instead of acp_worker.recv() |

### 1.5 Testing Strategy

- Unit test: SessionRouter routing correctness
- Unit test: Multiple sessions receiving correct messages
- Integration test: Two sessions running, each gets own messages

---

## Phase 2: Concurrent Execution + Tool Simplification

### 2.1 New Tool: `spawn_agents`

Replace `implement` with a more flexible `spawn_agents`:

```rust
// MCP tool definition
"spawn_agents" => {
    "description": "Spawn one or more agents to work on tasks concurrently. Waits for all to complete by default.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "agents": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "role": {"type": "string", "enum": ["implementer", "verifier", "explorer", "custom"]},
                        "task": {"type": "string"},
                        // For custom agents only:
                        "prompt": {"type": "string"},
                        "tools": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["role", "task"]
                }
            },
            "wait": {
                "type": "string",
                "enum": ["all", "any", "none"],
                "default": "all"
            }
        },
        "required": ["agents"]
    }
}
```

### 2.2 Usage Examples

```javascript
// Single implementer (most common case)
spawn_agents({agents: [{role: "implementer", task: "Add user validation"}]})

// Multiple concurrent implementers (implicit batch)
spawn_agents({agents: [
    {role: "implementer", task: "Add user model"},
    {role: "implementer", task: "Add user repository"},
    {role: "implementer", task: "Add user service"}
]})

// Implementation + verification
spawn_agents({agents: [
    {role: "implementer", task: "Implement feature X"},
    {role: "verifier", task: "Verify feature X works correctly"}
]})

// Ad-hoc custom agent
spawn_agents({agents: [{
    role: "custom",
    task: "Review security of auth module",
    prompt: "You are a security expert. Analyze the following code for vulnerabilities. Focus on authentication bypass, injection attacks, and credential handling.",
    tools: ["view", "codebase-retrieval"]
}]})
```

### 2.3 Concurrent Execution Flow

```
Orchestrator calls spawn_agents([agent1, agent2, agent3])
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Spawn all agents                              │
│  - Register each with SessionRouter                              │
│  - Create session via acp_worker.session_new()                   │
│  - Send prompt via acp_worker.session_prompt()                   │
│  - Track in ActiveSessions map                                   │
└─────────────────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                Wait for completion (wait="all")                  │
│  - Listen on completion_rx                                       │
│  - As each agent calls complete(), remove from ActiveSessions   │
│  - When ActiveSessions empty, return aggregated results          │
└─────────────────────────────────────────────────────────────────┘
                         │
                         ▼
         Return combined result to orchestrator
```

### 2.4 ToolCall Enum Changes

```rust
pub enum ToolCall {
    // Removed: Implement { task: String }

    // New: replaces implement, supports multiple agents
    SpawnAgents {
        agents: Vec<AgentSpec>,
        wait: WaitMode,  // defaults to All
    },

    // Kept: recursive sub-orchestration
    Decompose { task: String },

    // Kept: signal completion
    Complete { success: bool, message: Option<String> },

    // Removed: WritePlan (replaced by create_task)

    // New: for planners only
    CreateTask { name: String, description: String, dependencies: Vec<String> },
}

pub struct AgentSpec {
    pub role: AgentRole,
    pub task: String,
    pub prompt: Option<String>,  // For custom agents
    pub tools: Option<Vec<String>>,  // For custom agents
}

pub enum AgentRole {
    Implementer,
    Verifier,
    Explorer,
    Custom,
}

pub enum WaitMode {
    All,   // Wait for all agents (default)
    Any,   // Return when first completes
    None,  // Return immediately with session IDs
}
```

### 2.5 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `src/mcp_server/types.rs` | Modify | Replace Implement with SpawnAgents, remove WritePlan |
| `src/mcp_server/handlers.rs` | Modify | Handle spawn_agents, update tools/list |
| `src/app/orchestrator.rs` | Modify | Handle SpawnAgents, concurrent waiting |
| `src/app/implementer.rs` | Rename | → `src/app/agent_spawner.rs`, generalize |
| `prompts/orchestrator.txt` | Modify | Document new spawn_agents tool |

### 2.6 Testing Strategy

- Test: spawn single agent, verify completion
- Test: spawn multiple agents, verify all complete
- Test: wait="any" returns on first completion
- Test: error in one agent doesn't crash others

---

## Phase 3: Structured Task Tracking

### 3.1 Task Data Structures

```rust
pub type TaskId = String;  // UUID

pub struct Task {
    pub id: TaskId,
    pub name: String,
    pub description: String,
    pub status: TaskStatus,
    pub dependencies: Vec<TaskId>,
    pub created_at: Instant,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
}

pub enum TaskStatus {
    NotStarted,
    InProgress { agent_session: String },
    Complete { success: bool, summary: String },
    Failed { error: String },
}
```

### 3.2 TaskManager

```rust
pub struct TaskManager {
    tasks: HashMap<TaskId, Task>,
    event_tx: broadcast::Sender<LogEvent>,
}

impl TaskManager {
    /// Create a new task, returns its ID
    pub fn create(&mut self, name: &str, description: &str, deps: Vec<String>) -> TaskId;

    /// Get all tasks ready to execute (dependencies satisfied)
    pub fn get_ready_tasks(&self) -> Vec<&Task>;

    /// Update task status, emits LogEvent::TaskStateChanged
    pub fn update_status(&mut self, id: &TaskId, status: TaskStatus);

    /// Get task by ID
    pub fn get(&self, id: &TaskId) -> Option<&Task>;
}
```

### 3.3 Planner Tool: `create_task`

```rust
"create_task" => {
    "description": "Create a task in the plan. Call once per task.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Short task name"},
            "description": {"type": "string", "description": "Detailed task description"},
            "dependencies": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Names of tasks this depends on (must be created first)"
            }
        },
        "required": ["name", "description"]
    }
}
```

### 3.4 Remove Built-in Task Tools from Planner

When spawning planner agents, add to removed_tools:
- `view_tasklist`
- `reorganize_tasklist`
- `update_tasks`
- `add_tasks`

These are Augment's built-in task tools that would conflict with our `create_task`.

### 3.5 Updated Planner Prompt

```
Your job is to decompose work into smaller tasks.

## Tools Available

1. `create_task(name, description, dependencies)` - Create a task
2. `complete(success, message)` - Signal planning is done

## Guidelines

- Create tasks in dependency order
- Use task names for dependencies (they'll be resolved to IDs)
- Each task should be implementable by a single agent
- Include verification tasks at milestones

## Example

create_task(name="Add User model", description="Create User struct with id, email, name fields", dependencies=[])
create_task(name="Add User repository", description="CRUD operations for User", dependencies=["Add User model"])
create_task(name="Add User service", description="Business logic layer", dependencies=["Add User repository"])
create_task(name="Verify user functionality", description="Run tests, verify integration", dependencies=["Add User service"])
complete(success=true, message="Created 4 tasks for user feature")
```

### 3.6 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `src/tasks/mod.rs` | Create | Task, TaskId, TaskStatus types |
| `src/tasks/manager.rs` | Create | TaskManager implementation |
| `src/app/mod.rs` | Modify | Add TaskManager to App |
| `src/mcp_server/types.rs` | Modify | Add CreateTask variant |
| `src/mcp_server/handlers.rs` | Modify | Handle create_task |
| `src/logging/stream.rs` | Modify | Add TaskCreated, TaskStateChanged events |
| `prompts/planner.txt` | Rewrite | Document create_task workflow |

---

## Phase 4: Agent Templates + Ad-hoc Agents

### 4.1 Agent Template System

```rust
pub struct AgentTemplate {
    pub name: &'static str,
    pub prompt_template: &'static str,
    pub allowed_tools: Vec<&'static str>,
    pub removed_tools: Vec<&'static str>,
    pub model: Option<ModelId>,
}

pub struct AgentRegistry {
    templates: HashMap<String, AgentTemplate>,
}
```

### 4.2 Built-in Templates

**Implementer** (existing, formalized):
```rust
AgentTemplate {
    name: "implementer",
    prompt_template: include_str!("../../prompts/implementer.txt"),
    allowed_tools: vec!["complete"],  // Plus all Augment editing tools
    removed_tools: vec![],
    model: None,  // Use default
}
```

**Verifier** (new):
```rust
AgentTemplate {
    name: "verifier",
    prompt_template: include_str!("../../prompts/verifier.txt"),
    allowed_tools: vec!["complete"],
    removed_tools: vec!["str-replace-editor", "save-file", "remove-files"],  // Read-only
    model: None,
}
```

**Explorer** (new):
```rust
AgentTemplate {
    name: "explorer",
    prompt_template: include_str!("../../prompts/explorer.txt"),
    allowed_tools: vec!["complete"],
    removed_tools: vec![
        "str-replace-editor", "save-file", "remove-files",  // No editing
        "launch-process", "kill-process",  // No execution
    ],
    model: None,
}
```

### 4.3 Custom/Ad-hoc Agent Support

For `role: "custom"`, the orchestrator must provide:
- `prompt`: Full prompt text (required)
- `tools`: Explicit whitelist of Augment tools (required for safety)

```javascript
spawn_agents({agents: [{
    role: "custom",
    task: "Audit authentication security",
    prompt: `You are a security auditor. Your task is to:
1. Review the authentication implementation
2. Identify potential vulnerabilities
3. Report findings with severity levels

Focus on: credential handling, session management, input validation.

Task: {task}`,
    tools: ["view", "codebase-retrieval", "web-search"]
}]})
```

The system will:
1. Replace `{task}` in prompt with the provided task
2. Configure MCP server with only the specified tools + `complete`
3. Remove all other tools

### 4.4 Prompt Files to Create

**`prompts/verifier.txt`:**
```
You are a verification agent. Your job is to verify that work was completed correctly.

## Your Task
{task}

## Verification Checklist
1. Check that the implementation matches requirements
2. Run relevant tests (use launch-process)
3. Review code quality and style
4. Verify no regressions in existing functionality

## Completion
When done, call complete() with your findings:
- success=true if everything passes
- success=false if issues found, include details in message
```

**`prompts/explorer.txt`:**
```
You are an exploration agent. Your job is to research and gather information.

## Your Task
{task}

## Guidelines
- Use codebase-retrieval to find relevant code
- Use view to examine files in detail
- Use web-search/web-fetch for external information if needed
- Synthesize findings into actionable insights

## Completion
When done, call complete() with your findings summary.
Do NOT make any changes - you are read-only.
```

### 4.5 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `src/agents/mod.rs` | Create | AgentRegistry, AgentTemplate |
| `src/agents/templates.rs` | Create | Built-in template definitions |
| `prompts/verifier.txt` | Create | Verifier prompt |
| `prompts/explorer.txt` | Create | Explorer prompt |
| `src/app/agent_spawner.rs` | Modify | Support templates + custom agents |

---

## Phase 5: Logging Cleanup

### 5.1 Goals

**NOT a full unification** - just cleanup of overlapping mechanisms.

Current state:
- `tracing` → console + app.log (debug)
- `AgentWriter` → per-agent log files (transcripts)
- `print!()` → immediate console output (streaming)
- `LogEvent` → broadcast for UI

### 5.2 Changes

1. **Remove duplicate print!() calls** where AgentWriter already handles output
2. **Ensure LogEvent covers all meaningful state changes** for UI consumption
3. **Keep tracing for debug/diagnostics only**
4. **Clear ownership**: each mechanism has one job

### 5.3 Specific Cleanups

**In `src/app/session_drain.rs`:**
```rust
// Remove this:
print!("{text}");
std::io::Write::flush(&mut std::io::stdout()).ok();

// AgentWriter already handles output AND emits LogEvent
let _ = writer.write_message_chunk(text).await;
```

**In `src/app/session.rs`:**
Same pattern - remove print!(), rely on writer + events.

**Add missing LogEvent variants:**
```rust
// For orchestrator progress
LogEvent::OrchestratorProgress {
    phase: OrchestratorPhase,  // SpawningAgents, WaitingForCompletion, etc.
    message: String,
}
```

### 5.4 Files to Modify

| File | Action | Description |
|------|--------|-------------|
| `src/app/session.rs` | Modify | Remove duplicate print!() |
| `src/app/session_drain.rs` | Modify | Remove duplicate print!() |
| `src/logging/stream.rs` | Modify | Add OrchestratorProgress event |

---

## File Changes Summary

### New Files (9 files)

| Path | Purpose |
|------|---------|
| `src/app/router.rs` | SessionRouter for concurrent session message routing |
| `src/app/agent_spawner.rs` | Generic agent spawning (replaces implementer.rs) |
| `src/tasks/mod.rs` | Task, TaskId, TaskStatus types |
| `src/tasks/manager.rs` | TaskManager implementation |
| `src/agents/mod.rs` | AgentRegistry, AgentTemplate |
| `src/agents/templates.rs` | Built-in template definitions |
| `prompts/verifier.txt` | Verifier agent prompt |
| `prompts/explorer.txt` | Explorer agent prompt |
| `tests/scenarios/concurrent_spawn.toml` | Concurrent agent test |

### Modified Files (11 files)

| Path | Changes |
|------|---------|
| `src/acp.rs` | Allow taking notification_rx for routing |
| `src/app/mod.rs` | Add SessionRouter, TaskManager, remove old patterns |
| `src/app/orchestrator.rs` | Handle SpawnAgents, concurrent waiting |
| `src/app/session.rs` | Use per-session receiver, remove print!() |
| `src/app/session_drain.rs` | Remove print!(), simplify |
| `src/mcp_server/types.rs` | Replace Implement→SpawnAgents, add CreateTask |
| `src/mcp_server/handlers.rs` | Handle new tools, update tools/list |
| `src/logging/stream.rs` | Add task events, orchestrator progress |
| `prompts/planner.txt` | Rewrite for create_task workflow |
| `prompts/orchestrator.txt` | Document spawn_agents, decompose |

### Removed Files (1 file)

| Path | Reason |
|------|--------|
| `src/app/implementer.rs` | Replaced by agent_spawner.rs |

### Removed Code

| Item | Reason |
|------|--------|
| `ToolCall::Implement` | Replaced by SpawnAgents |
| `ToolCall::WritePlan` | Replaced by CreateTask |
| `stored_plan: Option<String>` | Replaced by TaskManager |
| Duplicate `print!()` calls | Already handled by AgentWriter |

---

## Estimated Effort

| Phase | Complexity | Estimate |
|-------|------------|----------|
| Phase 1: Session Routing | Medium | 1-2 days |
| Phase 2: Concurrent Execution + Tools | High | 2-3 days |
| Phase 3: Structured Tasks | Medium | 1-2 days |
| Phase 4: Agent Templates | Low | 1 day |
| Phase 5: Logging Cleanup | Low | 0.5 days |

**Total: 5.5-8.5 days** (assuming single developer, tests included)

Note: Effort reduced from original estimate due to:
- No backward compatibility requirement
- Simpler implicit batch model (no complex wait modes needed initially)
- Logging cleanup instead of full unification

---

## Implementation Order

### Suggested Order

1. **Phase 1 first** - SessionRouter is foundation for everything
2. **Phase 2 next** - Core functionality change (spawn_agents)
3. **Phase 3 follows** - Structured tasks build on spawn_agents
4. **Phase 4 then** - Templates are optional enhancement
5. **Phase 5 opportunistic** - Clean as you go

### Minimal Viable Change

If you want to get concurrent execution working quickly:

1. Implement SessionRouter (Phase 1)
2. Add spawn_agents with wait="all" only (Phase 2 partial)
3. Keep existing orchestrator prompts, just change implement→spawn_agents

This gives you concurrency without restructuring tasks or adding new agent types.

### Recommended Implementation Batches

Since Villalobos will implement changes on itself, keep a backup binary:

```bash
cargo build --release
cp target/release/villalobos ~/villalobos-backup
```

Then implement in these batches (each produces a working system):

**Batch 1: Phases 1+2 together** (Core concurrency)
- These are tightly coupled
- Phase 1 alone would work but is unused
- Do them together for coherence
- Breaking change: `implement` → `spawn_agents`

**Batch 2: Phase 3** (Structured tasks)
- Independent enhancement
- Breaking change: `write_plan` → `create_task`
- Can be deferred if Batch 1 is sufficient

**Batch 3: Phases 4+5** (Polish)
- Templates and cleanup
- Nice to have, not essential

After each batch:
1. `cargo check` - must compile
2. `cargo test` - must pass
3. Manual test with a simple goal
4. Commit if working

---

## Testing Strategy

### Running Existing Tests

```bash
# Run all tests
cargo test

# Run with output visible
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run tests in a specific module
cargo test tasks::

# Check compilation without running
cargo check
```

### Test Scenarios Location

Integration tests use TOML scenario files in `tests/scenarios/`. Examples:
- `tests/scenarios/simple_implement.toml` - Single implementation flow
- `tests/scenarios/multi_implement.toml` - Multiple sequential implementations
- `tests/scenarios/planning_only.toml` - Planner creates plan

### Writing New Tests

**Unit tests** should be in the same file as the code:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {
        // ...
    }

    #[tokio::test]
    async fn test_async_something() {
        // ...
    }
}
```

**Integration tests** should update or create scenario files in `tests/scenarios/`.

### Test Requirements Per Phase

**Phase 1 (Session Routing):**
- [ ] Unit test: SessionRouter routes messages correctly by session_id
- [ ] Unit test: Multiple sessions receive their own messages
- [ ] Unit test: Unregistered session messages are dropped gracefully
- [ ] Integration: Existing tests still pass (no regression)

**Phase 2 (Concurrent Execution):**
- [ ] Unit test: spawn_agents with single agent works
- [ ] Unit test: spawn_agents with multiple agents spawns all
- [ ] Unit test: wait="all" blocks until all complete
- [ ] Integration: Create `tests/scenarios/concurrent_spawn.toml`
- [ ] Manual test: Run actual concurrent implementation

**Phase 3 (Structured Tasks):**
- [ ] Unit test: TaskManager create/get/update operations
- [ ] Unit test: Dependency resolution (task blocked if deps not met)
- [ ] Unit test: Status transitions emit LogEvent
- [ ] Integration: Create `tests/scenarios/task_creation.toml`

**Phase 4 (Agent Templates):**
- [ ] Unit test: AgentRegistry lookup by role
- [ ] Unit test: Custom agent with tool whitelist
- [ ] Integration: Verifier agent scenario

**Phase 5 (Logging Cleanup):**
- [ ] Manual verification: No duplicate output
- [ ] All existing tests still pass

---

## Implementation Guidance

### Code Patterns to Follow

**Error Handling:**
```rust
// Use anyhow for errors that bubble up
use anyhow::{Context, Result};

fn do_something() -> Result<Value> {
    let result = operation().context("Failed to do operation")?;
    Ok(result)
}
```

**Async Patterns:**
```rust
// Use tokio for async
use tokio::sync::{mpsc, RwLock};
use std::sync::Arc;

// Shared state pattern
let shared = Arc::new(RwLock::new(State::new()));
let shared_clone = shared.clone();
tokio::spawn(async move {
    let mut state = shared_clone.write().await;
    state.update();
});
```

**Module Organization:**
```rust
// In mod.rs, re-export public types
mod manager;
mod types;

pub use manager::TaskManager;
pub use types::{Task, TaskId, TaskStatus};
```

### Common Pitfalls to Avoid

1. **Don't forget to update `src/lib.rs`** when adding new modules
2. **Don't forget to update `Cargo.toml`** if adding dependencies (unlikely needed)
3. **Watch for borrow checker issues** with `Arc<RwLock<>>` - don't hold locks across await points
4. **MCP tool names must match** in both handlers.rs (tools/list) and the tool call handler
5. **Session IDs are strings** - use `String` not `&str` for owned data in structs

### Verification Steps After Each Phase

**After Phase 1:**
```bash
cargo check  # Must compile
cargo test   # All existing tests pass
```

**After Phase 2:**
```bash
cargo check
cargo test
# Manual test: Run the app with a multi-task goal
cargo run -- "Create two simple functions"
```

**After Phase 3:**
```bash
cargo check
cargo test
cargo test tasks::  # New task tests pass
```

**After Phase 4:**
```bash
cargo check
cargo test
cargo test agents::  # New agent tests pass
```

**After Phase 5:**
```bash
cargo check
cargo test
# Visual inspection: No duplicate console output
```

---

## Detailed Implementation Steps

### Phase 1: Session Routing (Start Here)

**Step 1.1: Create `src/app/router.rs`**

```rust
//! Session routing for concurrent ACP sessions.

use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Routes ACP messages to per-session channels.
pub struct SessionRouter {
    routes: HashMap<String, mpsc::Sender<Value>>,
}

impl SessionRouter {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Register a session, returns receiver for its messages.
    pub fn register(&mut self, session_id: String) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel(100);
        self.routes.insert(session_id, tx);
        rx
    }

    /// Unregister a session.
    pub fn unregister(&mut self, session_id: &str) {
        self.routes.remove(session_id);
    }

    /// Route a message to the appropriate session.
    /// Returns true if routed, false if no route found.
    pub async fn route(&self, msg: &Value) -> bool {
        if let Some(session_id) = Self::extract_session_id(msg) {
            if let Some(tx) = self.routes.get(&session_id) {
                return tx.send(msg.clone()).await.is_ok();
            }
        }
        false
    }

    fn extract_session_id(msg: &Value) -> Option<String> {
        msg.get("params")?
            .get("sessionId")?
            .as_str()
            .map(String::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_route_to_registered_session() {
        let mut router = SessionRouter::new();
        let mut rx = router.register("session-1".to_string());

        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {"sessionUpdate": "agent_message_chunk"}
            }
        });

        assert!(router.route(&msg).await);
        let received = rx.recv().await.unwrap();
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn test_unregistered_session_not_routed() {
        let router = SessionRouter::new();
        let msg = json!({
            "params": {"sessionId": "unknown"}
        });
        assert!(!router.route(&msg).await);
    }
}
```

**Step 1.2: Update `src/app/mod.rs`**

Add the router module and integrate into App:
```rust
mod router;
pub use router::SessionRouter;

// In App struct, add:
pub(crate) session_router: Arc<RwLock<SessionRouter>>,
```

**Step 1.3: Modify ACP client to allow taking notification_rx**

In `src/acp.rs`, add a method to take the notification receiver:
```rust
impl AcpClient {
    /// Take the notification receiver for external routing.
    /// After calling this, recv() will panic.
    pub fn take_notification_rx(&mut self) -> mpsc::Receiver<Value> {
        std::mem::replace(&mut self.notification_rx, /* create dummy */)
        // Note: Need to handle this carefully
    }
}
```

Actually, a better approach is to inject the router at AcpClient creation time, so messages are routed from the start. Review `src/acp.rs:130-170` for the stdout reader task.

**Step 1.4: Update session waiting to use per-session channel**

In `src/app/session.rs`, change from:
```rust
msg_result = self.acp_worker.recv() => { ... }
```

To receiving from the per-session channel provided by the router.

**Step 1.5: Run tests**
```bash
cargo test
```

### Phase 2: Concurrent Execution (After Phase 1 works)

**Step 2.1: Update `src/mcp_server/types.rs`**

Replace `Implement` with `SpawnAgents`:
```rust
pub enum ToolCall {
    SpawnAgents {
        agents: Vec<AgentSpec>,
        wait: WaitMode,
    },
    Decompose { task: String },
    Complete { success: bool, message: Option<String> },
    CreateTask { name: String, description: String, dependencies: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub role: String,  // "implementer", "verifier", "explorer", "custom"
    pub task: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WaitMode {
    #[default]
    All,
    Any,
    None,
}
```

**Step 2.2: Update `src/mcp_server/handlers.rs`**

Update tools/list to return spawn_agents instead of implement.
Update handle_tool_call to parse spawn_agents.

**Step 2.3: Create `src/app/agent_spawner.rs`**

Generalize from implementer.rs to handle any agent role.

**Step 2.4: Update orchestrator to handle SpawnAgents**

In `src/app/orchestrator.rs`, handle the new tool call:
- Spawn all agents concurrently using tokio::spawn
- Wait for all completions via completion channel
- Return aggregated results

**Step 2.5: Update orchestrator prompt**

Modify `prompts/orchestrator.txt` to document spawn_agents.

**Step 2.6: Run tests**
```bash
cargo test
cargo run -- "Create a hello world function"  # Manual test
```

### Subsequent Phases

Follow the detailed steps in each phase section above. The key is:
1. Make the code change
2. Ensure it compiles (`cargo check`)
3. Run existing tests (`cargo test`)
4. Add new tests for new functionality
5. Manual verification if needed

---

## Critical Considerations

### Breaking Changes

This refactor introduces breaking changes. After implementation:
- Old prompts using `implement()` will fail - update to `spawn_agents()`
- Old prompts using `write_plan()` will fail - update to `create_task()`
- Test scenarios in `tests/scenarios/` need updating

### Rust-Specific Notes

**Lifetimes with async:**
When spawning tasks that need references, clone the data:
```rust
// Wrong - borrow doesn't live long enough
let task_ref = &task;
tokio::spawn(async move { use_task(task_ref) });

// Right - clone owned data
let task_owned = task.clone();
tokio::spawn(async move { use_task(&task_owned) });
```

**Select! with multiple channels:**
```rust
tokio::select! {
    biased;  // Use if you want priority ordering

    Some(msg) = channel1.recv() => { ... }
    Some(msg) = channel2.recv() => { ... }
    else => break,  // All channels closed
}
```

**Arc<RwLock<>> deadlock prevention:**
```rust
// Don't hold lock across await
let value = {
    let guard = shared.read().await;
    guard.clone()  // Clone and drop guard
};
// Now safe to await
do_something(value).await;
```

### Files That Must Stay in Sync

1. **`src/mcp_server/types.rs`** ↔ **`src/mcp_server/handlers.rs`**
   - ToolCall enum variants must match handler parsing

2. **`src/mcp_server/handlers.rs` (tools/list)** ↔ **prompts/*.txt**
   - Tool names and parameters must match what agents expect

3. **`src/app/mod.rs`** ↔ **`src/lib.rs`**
   - Module declarations must be consistent

### Rollback Strategy

If something goes wrong mid-implementation:
```bash
# See what changed
git status
git diff

# Revert all changes
git checkout .

# Or revert specific file
git checkout src/app/orchestrator.rs
```

Keep commits small and frequent so rollback is easy.

---

## Appendix: Key Code References

### Current Orchestrator Loop (`src/app/orchestrator.rs:94-186`)
The main `select!` loop that will be modified for concurrent waiting.

### Session Message Handling (`src/app/session_drain.rs:30-35`)
Where messages are currently filtered by session_id (and others dropped).

### ACP Notification Channel (`src/acp.rs:107`)
The `notification_rx` that will be routed through SessionRouter.

### Tool Definitions (`src/mcp_server/handlers.rs:43-166`)
Where tools/list is handled - tool schemas defined per agent type.

### Agent Type Detection (`src/mcp_server/handlers.rs:45-46`)
How agent type is determined: `std::env::var("VILLALOBOS_AGENT_TYPE")`

### LogEvent Types (`src/logging/stream.rs`)
Current event types - add new variants here for task tracking.

### Test Harness (`src/testing/harness.rs`)
Mock infrastructure for integration testing.

---

## Success Criteria

The implementation is complete when:

- [ ] `cargo check` passes with no errors
- [ ] `cargo test` passes (all existing + new tests)
- [ ] `cargo clippy` passes with no warnings (optional but nice)
- [ ] Manual test: `cargo run -- "Create two functions that call each other"`
  - Should spawn multiple implementers concurrently
  - Should complete successfully
- [ ] Console output shows concurrent agent activity (overlapping, not sequential)
- [ ] Log files in `logs/` directory show correct structure

---

## Quick Reference: Tool Mapping

| Old Tool | New Tool | Agent |
|----------|----------|-------|
| `implement(task)` | `spawn_agents([{role:"implementer",task}])` | Orchestrator |
| `decompose(task)` | `decompose(task)` (unchanged) | Orchestrator |
| `complete(...)` | `complete(...)` (unchanged) | All |
| `write_plan(plan)` | `create_task(name,desc,deps)` (call multiple times) | Planner |

---

## Command Cheat Sheet

```bash
# Build and check
cargo build
cargo check
cargo clippy

# Test
cargo test
cargo test -- --nocapture  # See output
cargo test specific_test_name

# Run
cargo run -- "your goal here"
RUST_LOG=debug cargo run -- "your goal"  # Verbose

# Format
cargo fmt

# Watch for changes
cargo watch -x check
```
