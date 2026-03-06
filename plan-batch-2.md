# Batch 2: Structured Task Tracking (Phase 3)

## Current State (Partial Implementation)

Some work was started but NOT completed. Here's what EXISTS and what NEEDS to be done:

### Already Exists (DO NOT RECREATE)
- ✅ `src/tasks/mod.rs` - Task, TaskId, TaskStatus types (complete, but unused)
- ✅ `src/tasks/manager.rs` - STUB only (just has empty `new()` method)
- ✅ `src/logging/stream.rs` - TaskCreated, TaskStateChanged variants (complete)
- ✅ `src/mcp_server/types.rs` - CreateTask variant in ToolCall enum (complete)
- ✅ `src/mcp_server/types.rs` - tool_type() handles CreateTask (complete)
- ✅ `prompts/planner.txt` - Updated for create_task (complete)
- ✅ `src/main.rs` - Has `mod tasks;`
- ✅ `src/testing/interceptor.rs` - CreateTask handling (complete)

### NOT Complete (MUST BE IMPLEMENTED)
1. `src/tasks/manager.rs` - Needs full implementation with:
   - `event_tx: broadcast::Sender<LogEvent>` field
   - `new(event_tx)` constructor
   - `create(name, description, dep_names)` method
   - `find_by_name(name)` method
   - `get_ready_tasks()` method
   - `update_status(id, status)` method
   - `all_tasks()` method
   - Unit tests

2. `src/mcp_server/handlers.rs` - Needs:
   - Add `create_task` tool to planner tools/list response
   - Add `"create_task"` case in `handle_tool_call` to parse and create `ToolCall::CreateTask`

3. `src/app/session.rs` - Needs:
   - Handle `ToolCall::CreateTask` in tool message processing (call task_manager.create())

4. `src/app/mod.rs` - Needs:
   - Add `task_manager: Arc<RwLock<TaskManager>>` field to App struct
   - Initialize TaskManager in constructors
   - Import `use crate::tasks::TaskManager;`

5. `src/app/planner.rs` - Needs:
   - Add removed tools for built-in task management: `view_tasklist`, `reorganize_tasklist`, `update_tasks`, `add_tasks`

---

## Prerequisites
- `cargo check` and `cargo test` pass before starting

## Design Principles
- **Simplify** - `create_task` is simpler than `write_plan` for structured tasks
- **Observable state** - Tasks have status (not_started, in_progress, complete, failed)
- **Dependencies** - Tasks can depend on other tasks

### Tool Surface After This Batch

| Agent Type | Tools |
|------------|-------|
| **Orchestrator** | `spawn_agents`, `decompose`, `complete` (unchanged) |
| **Planner** | `create_task`, `complete`, `write_plan` (write_plan kept for compatibility) |
| **Implementer** | `complete` (unchanged) |

---

## Implementation Tasks

### Task 1: Complete TaskManager Implementation

Update `src/tasks/manager.rs` to add the full implementation:

```rust
//! Task manager for CRUD operations and queries.

use super::{Task, TaskId, TaskStatus};
use crate::logging::LogEvent;
use std::collections::HashMap;
use tokio::sync::broadcast;

pub struct TaskManager {
    tasks: HashMap<TaskId, Task>,
    event_tx: broadcast::Sender<LogEvent>,
}

impl TaskManager {
    pub fn new(event_tx: broadcast::Sender<LogEvent>) -> Self {
        Self {
            tasks: HashMap::new(),
            event_tx,
        }
    }

    /// Create a new task, returns its ID.
    pub fn create(&mut self, name: &str, description: &str, dep_names: Vec<String>) -> TaskId {
        let id = uuid::Uuid::new_v4().to_string();

        // Resolve dependency names to IDs
        let dependencies: Vec<TaskId> = dep_names
            .iter()
            .filter_map(|n| self.find_by_name(n))
            .collect();

        let task = Task {
            id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            status: TaskStatus::NotStarted,
            dependencies,
            created_at: Some(std::time::Instant::now()),
        };

        // Emit TaskCreated event
        let _ = self.event_tx.send(LogEvent::TaskCreated {
            task_id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            dependencies: dep_names,
            depth: 0,
        });

        self.tasks.insert(id.clone(), task);
        id
    }

    /// Find task by name (for dependency resolution).
    pub fn find_by_name(&self, name: &str) -> Option<TaskId> {
        self.tasks.values()
            .find(|t| t.name == name)
            .map(|t| t.id.clone())
    }

    /// Get all tasks ready to execute (dependencies complete).
    pub fn get_ready_tasks(&self) -> Vec<&Task> {
        self.tasks.values()
            .filter(|t| {
                matches!(t.status, TaskStatus::NotStarted) &&
                t.dependencies.iter().all(|dep_id| {
                    self.tasks.get(dep_id)
                        .map(|dep| matches!(dep.status, TaskStatus::Complete { .. }))
                        .unwrap_or(false)
                })
            })
            .collect()
    }

    /// Update task status.
    pub fn update_status(&mut self, id: &TaskId, status: TaskStatus) {
        if let Some(task) = self.tasks.get_mut(id) {
            let old_status = task.status.clone();
            task.status = status.clone();

            // Emit event (ignore send errors)
            let _ = self.event_tx.send(LogEvent::TaskStateChanged {
                task_id: id.clone(),
                name: task.name.clone(),
                old_status: format!("{:?}", old_status),
                new_status: format!("{:?}", status),
                depth: 0,
            });
        }
    }

    /// Get all tasks.
    pub fn all_tasks(&self) -> Vec<&Task> {
        self.tasks.values().collect()
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        // For default, create a channel that will be ignored
        let (tx, _) = broadcast::channel(16);
        Self::new(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_task() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id = manager.create("Task 1", "Description", vec![]);
        assert!(!id.is_empty());
        assert_eq!(manager.all_tasks().len(), 1);
    }

    #[test]
    fn test_dependency_resolution() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        manager.create("Task A", "First task", vec![]);
        manager.create("Task B", "Depends on A", vec!["Task A".to_string()]);

        let task_b = manager.all_tasks().iter()
            .find(|t| t.name == "Task B").unwrap();
        assert_eq!(task_b.dependencies.len(), 1);
    }

    #[test]
    fn test_get_ready_tasks() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id_a = manager.create("Task A", "No deps", vec![]);
        manager.create("Task B", "Depends on A", vec!["Task A".to_string()]);

        // Only A is ready initially
        assert_eq!(manager.get_ready_tasks().len(), 1);

        // Complete A
        manager.update_status(&id_a, TaskStatus::Complete {
            success: true,
            summary: "Done".to_string(),
        });

        // Now B is ready
        assert_eq!(manager.get_ready_tasks().len(), 1);
        assert_eq!(manager.get_ready_tasks()[0].name, "Task B");
    }

    #[test]
    fn test_find_by_name() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id = manager.create("My Task", "Description", vec![]);

        assert_eq!(manager.find_by_name("My Task"), Some(id));
        assert_eq!(manager.find_by_name("Nonexistent"), None);
    }
}
```

### Task 2: Add create_task Tool to MCP Handlers

In `src/mcp_server/handlers.rs`:

1. Find the planner tools/list response and add `create_task` tool schema:
```rust
{
    "name": "create_task",
    "description": "Create a task in the plan. Call once per task.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Short task name"},
            "description": {"type": "string", "description": "Detailed task description with requirements"},
            "dependencies": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Names of tasks this depends on (empty array if none)"
            }
        },
        "required": ["name", "description"]
    }
}
```

2. Add `"create_task"` case in the tool call handling:
```rust
"create_task" => {
    let name = arguments.get("name").and_then(|v| v.as_str())
        .ok_or_else(|| /* error */)?;
    let description = arguments.get("description").and_then(|v| v.as_str())
        .ok_or_else(|| /* error */)?;
    let dependencies = arguments.get("dependencies")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    ToolCall::CreateTask {
        name: name.to_string(),
        description: description.to_string(),
        dependencies,
    }
}
```

### Task 3: Handle CreateTask in Session Tool Processing

In `src/app/session.rs`, find where `ToolCall::WritePlan` is handled and add similar handling for `CreateTask`:

```rust
ToolCall::CreateTask { name, description, dependencies } => {
    // Get task_manager from app (need to pass it or access via self)
    let task_id = {
        let mut tm = self.task_manager.write().await;
        tm.create(&name, &description, dependencies.clone())
    };

    tracing::info!(
        "📋 Session {} created task '{}' (id: {})",
        session_id, name, task_id
    );

    let response = ToolResponse::success(
        request.request_id,
        format!("Task '{}' created with id {}", name, task_id),
    );
    let _ = response_tx.send(response);
}
```

### Task 4: Integrate TaskManager into App

In `src/app/mod.rs`:

1. Add import: `use crate::tasks::TaskManager;`
2. Add field to App struct: `pub(crate) task_manager: Arc<RwLock<TaskManager>>,`
3. Initialize in `with_log_manager_and_timeout`:
   ```rust
   let event_tx = log_manager.event_sender();
   let task_manager = Arc::new(RwLock::new(TaskManager::new(event_tx)));
   ```
4. Update all other constructors (`with_mock_clients`, `with_mock_clients_and_tool_rx`) to include the field

### Task 5: Add Removed Tools for Planner

In `src/app/planner.rs`, when spawning the planner agent, configure removed tools to disable built-in task management:

```rust
const PLANNER_REMOVED_TOOLS: &[&str] = &[
    "view_tasklist",
    "reorganize_tasklist",
    "update_tasks",
    "add_tasks",
];
```

Apply these when setting up the planner's auggie cache (similar to how orchestrator removes editing tools).

### Task 6: Verify and Clean Up

1. Run `cargo check` - must pass
2. Run `cargo test` - must pass (all 166+ tests)
3. Run `cargo test tasks::` - verify new TaskManager tests pass
4. Remove any unused imports causing warnings

---

## Files to Modify

| File | Changes |
|------|---------|
| `src/tasks/manager.rs` | Complete implementation with all methods and tests |
| `src/mcp_server/handlers.rs` | Add create_task tool schema and parsing |
| `src/app/session.rs` | Handle CreateTask tool calls |
| `src/app/mod.rs` | Add task_manager field, initialize in constructors |
| `src/app/planner.rs` | Add removed tools for built-in task management |

---

## Testing

```bash
cargo check
cargo test
cargo test tasks::  # New TaskManager tests
```

---

## Success Criteria

- [ ] `cargo check` passes with no errors
- [ ] `cargo test` passes (all tests)
- [ ] TaskManager has full CRUD implementation
- [ ] `create_task` MCP tool is exposed to planner
- [ ] CreateTask tool calls create tasks in TaskManager
- [ ] TaskCreated and TaskStateChanged events are emitted
- [ ] Built-in task tools are removed from planner

---

## Important Notes

1. **Keep WritePlan** - Don't remove WritePlan yet; it's still used for compatibility
2. **Sequential execution** - Agents currently run sequentially (concurrent mode disabled due to tool routing issues); this is expected behavior
3. **Don't recreate existing files** - The task types already exist; only the manager implementation is incomplete

---

## Rollback

```bash
git checkout .
```

