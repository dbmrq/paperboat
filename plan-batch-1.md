# Batch 1: Session Routing + Concurrent Execution (Phases 1+2)

## Context

This is part of a larger refactor of the Villalobos multi-agent orchestration system. This batch implements the core concurrency infrastructure.

### Current Architecture
- **App struct** manages orchestration with two ACP clients (orchestrator + workers)
- **Flow**: Goal → Planner → Orchestrator → Implementer agents
- **Problem**: Tasks run sequentially because messages for other sessions are dropped

### Design Principles
- **Simplify** - Fewer tools, clearer responsibilities
- **Implicit batching** - `spawn_agents([...])` waits for all by default
- **No backward compatibility** - Clean break from old patterns

### Tool Surface After This Batch

| Agent Type | Tools |
|------------|-------|
| **Orchestrator** | `spawn_agents`, `decompose`, `complete` |
| **Planner** | `write_plan`, `complete` (unchanged for now) |
| **Implementer** | `complete` |

---

## Phase 1: Session Routing Infrastructure

### Problem

In `src/app/session_drain.rs:30-35`, messages for OTHER sessions are consumed and dropped:

```rust
let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());
if msg_session_id != Some(session_id) {
    return Ok(false);  // Message consumed but ignored!
}
```

### Solution: SessionRouter

Create `src/app/router.rs`:

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
        Self { routes: HashMap::new() }
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
```

### Integration Steps

1. Add `mod router;` to `src/app/mod.rs`
2. Add `session_router: Arc<RwLock<SessionRouter>>` to App struct
3. Spawn background task to route messages from `acp_worker`
4. Update session waiting to use per-session receivers

---

## Phase 2: Concurrent Execution + Tool Simplification

### New Tool: spawn_agents

Replace `implement` with `spawn_agents` in `src/mcp_server/types.rs`:

```rust
pub enum ToolCall {
    SpawnAgents {
        agents: Vec<AgentSpec>,
        wait: WaitMode,
    },
    Decompose { task: String },
    Complete { success: bool, message: Option<String> },
    WritePlan { plan: String },  // Keep for now, Phase 3 replaces
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub role: String,  // "implementer" for now
    pub task: String,
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

### Update MCP Handler

In `src/mcp_server/handlers.rs`, update `tools/list` for orchestrator:

```rust
// Replace "implement" tool definition with spawn_agents
```

### Orchestrator Changes

In `src/app/orchestrator.rs`, handle `SpawnAgents`:

1. Parse the agents array
2. For each agent, spawn via `tokio::spawn`
3. Collect completion receivers
4. If `wait == All`, wait for all completions before returning

### Create agent_spawner.rs

Rename/refactor `src/app/implementer.rs` → `src/app/agent_spawner.rs`:
- Generalize to handle any agent role
- Return completion via channel instead of blocking

### Update Orchestrator Prompt

Modify `prompts/orchestrator.txt`:

```
You are an orchestrator agent. Your job is to coordinate task completion.

## Tools Available (via MCP)

1. `spawn_agents(agents, wait)` - Spawn one or more agents
   - agents: Array of {role: "implementer", task: "description"}
   - wait: "all" (default), "any", or "none"

2. `decompose(task)` - Break down a complex task recursively

3. `complete(success, message)` - Signal you are done

## Guidelines

- Use spawn_agents for implementation work
- Multiple agents can run concurrently: spawn_agents([{...}, {...}])
- Use decompose for tasks that need planning first
- Call complete() when all work is delegated
```

---

## Files to Create/Modify

### Create
| File | Purpose |
|------|---------|
| `src/app/router.rs` | SessionRouter implementation |
| `src/app/agent_spawner.rs` | Generic agent spawning (from implementer.rs) |

### Modify
| File | Changes |
|------|---------|
| `src/app/mod.rs` | Add router module, SessionRouter to App |
| `src/app/orchestrator.rs` | Handle SpawnAgents, concurrent waiting |
| `src/app/session.rs` | Use per-session receiver from router |
| `src/mcp_server/types.rs` | Replace Implement with SpawnAgents |
| `src/mcp_server/handlers.rs` | Update tools/list, handle spawn_agents |
| `prompts/orchestrator.txt` | Document spawn_agents |

### Remove
| File | Reason |
|------|--------|
| `src/app/implementer.rs` | Replaced by agent_spawner.rs |

---

## Testing

### Commands
```bash
cargo check   # Must compile
cargo test    # All tests pass
```

### Verification After Implementation

1. **Compile check**: `cargo check`
2. **Run tests**: `cargo test`
3. **Manual test**: `cargo run -- "Create two independent functions"`
   - Should see concurrent activity in logs
   - Both should complete

### Unit Tests to Add

In `src/app/router.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_route_to_registered_session() {
        let mut router = SessionRouter::new();
        let mut rx = router.register("session-1".to_string());

        let msg = json!({"params": {"sessionId": "session-1"}});
        assert!(router.route(&msg).await);
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_unregistered_session_not_routed() {
        let router = SessionRouter::new();
        let msg = json!({"params": {"sessionId": "unknown"}});
        assert!(!router.route(&msg).await);
    }
}
```

---

## Success Criteria

- [ ] `cargo check` passes
- [ ] `cargo test` passes
- [ ] `spawn_agents` with single agent works
- [ ] `spawn_agents` with multiple agents spawns all concurrently
- [ ] `decompose` still works (unchanged)
- [ ] `complete` still works (unchanged)

---

## Common Pitfalls

1. **Don't forget to update `src/lib.rs`** if adding new public modules
2. **Arc<RwLock<>> deadlocks**: Don't hold locks across await points
3. **Channel capacity**: Use reasonable buffer sizes (100 is fine)
4. **Error propagation**: Use `?` with anyhow for proper error context

## Rollback

If something breaks:
```bash
git checkout .
```

