//! Task manager for CRUD operations and queries.

use super::{Task, TaskId, TaskStatus};
use crate::logging::LogEvent;
use std::collections::HashMap;
use tokio::sync::broadcast;

/// The goal summary set by the planner.
#[derive(Debug, Clone, Default)]
pub struct GoalSummary {
    /// A concise summary of the user's goal.
    pub summary: String,
    /// Success criteria / acceptance conditions.
    pub acceptance_criteria: Option<String>,
}

/// A note left by an agent for context sharing.
#[derive(Debug, Clone)]
pub struct AgentNote {
    /// The role of the agent that left the note (e.g., "implementer", "verifier").
    pub agent_role: String,
    /// Optional task ID this note is associated with.
    pub task_id: Option<TaskId>,
    /// The note content.
    pub content: String,
}

/// Snapshot of `TaskManager` state for nested orchestration.
///
/// When decomposing tasks, we need to temporarily clear the task manager
/// for the nested planner, then restore the parent's state after.
#[derive(Clone)]
pub struct TaskManagerSnapshot {
    tasks: HashMap<TaskId, Task>,
    next_task_num: u32,
    goal: Option<GoalSummary>,
    /// Notes are saved in the snapshot but not restored - we keep notes
    /// accumulated during nested orchestration for context.
    #[allow(dead_code)]
    notes: Vec<AgentNote>,
    /// Depth level to restore when returning from nested orchestration.
    depth: u32,
}

/// Manages tasks for a structured plan.
///
/// Provides CRUD operations and queries for tasks, including
/// dependency tracking and status updates.
pub struct TaskManager {
    tasks: HashMap<TaskId, Task>,
    event_tx: broadcast::Sender<LogEvent>,
    /// Counter for generating sequential task IDs (task001, task002, etc.)
    next_task_num: u32,
    /// The goal summary set by the planner.
    goal: Option<GoalSummary>,
    /// Notes left by agents for context sharing.
    notes: Vec<AgentNote>,
    /// Current depth in the task hierarchy (0 = root level).
    /// Used when emitting `TaskCreated` events for nested task visualization.
    depth: u32,
}

impl TaskManager {
    /// Creates a new `TaskManager` with the given event sender.
    pub fn new(event_tx: broadcast::Sender<LogEvent>) -> Self {
        Self {
            tasks: HashMap::new(),
            event_tx,
            next_task_num: 1,
            goal: None,
            notes: Vec::new(),
            depth: 0,
        }
    }

    /// Take a snapshot of the current task state for nested orchestration.
    ///
    /// This saves the current tasks, goal, notes, and depth so they can be restored
    /// after a nested orchestrator completes.
    pub fn snapshot(&self) -> TaskManagerSnapshot {
        TaskManagerSnapshot {
            tasks: self.tasks.clone(),
            next_task_num: self.next_task_num,
            goal: self.goal.clone(),
            notes: self.notes.clone(),
            depth: self.depth,
        }
    }

    /// Clear tasks for nested orchestration.
    ///
    /// This clears all tasks and resets the task counter, allowing a nested
    /// planner to create fresh tasks without ID conflicts.
    /// The goal and notes are preserved for context.
    ///
    /// # Arguments
    /// * `new_depth` - The depth level for the nested scope (for task hierarchy visualization)
    pub fn clear_tasks_for_nested(&mut self, new_depth: u32) {
        self.tasks.clear();
        self.next_task_num = 1;
        self.depth = new_depth;
        tracing::debug!(
            "🔄 Cleared tasks for nested orchestration (depth={})",
            new_depth
        );
    }

    /// Restore from a snapshot after nested orchestration completes.
    ///
    /// This restores the parent's tasks, depth, and state, updating any that changed status
    /// during nested execution.
    pub fn restore_from_snapshot(&mut self, snapshot: TaskManagerSnapshot) {
        self.tasks = snapshot.tasks;
        self.next_task_num = snapshot.next_task_num;
        self.goal = snapshot.goal;
        self.depth = snapshot.depth;
        // Keep notes accumulated during nested orchestration
        // (they provide useful context for later tasks)
        tracing::debug!(
            "🔄 Restored {} tasks from snapshot (depth={})",
            self.tasks.len(),
            self.depth
        );
    }

    /// Set the goal summary.
    pub fn set_goal(&mut self, summary: String, acceptance_criteria: Option<String>) {
        self.goal = Some(GoalSummary {
            summary,
            acceptance_criteria,
        });
        tracing::info!("📎 Goal set: {}", self.goal.as_ref().unwrap().summary);
    }

    /// Format the goal for inclusion in prompts.
    pub fn format_goal(&self) -> String {
        match &self.goal {
            Some(goal) => {
                let mut result = format!("**Goal**: {}", goal.summary);
                if let Some(criteria) = &goal.acceptance_criteria {
                    use std::fmt::Write;
                    let _ = write!(result, "\n\n**Acceptance Criteria**: {criteria}");
                }
                result
            }
            None => "(No goal set)".to_string(),
        }
    }

    /// Add a note from an agent.
    pub fn add_note(&mut self, agent_role: &str, task_id: Option<TaskId>, content: String) {
        tracing::info!("📝 Note from [{}]: {}", agent_role, content);
        self.notes.push(AgentNote {
            agent_role: agent_role.to_string(),
            task_id,
            content,
        });
    }

    /// Format notes for inclusion in prompts.
    /// Returns None if there are no notes.
    pub fn format_notes(&self) -> Option<String> {
        if self.notes.is_empty() {
            return None;
        }

        let lines: Vec<String> = self
            .notes
            .iter()
            .map(|note| {
                let task_ref = note
                    .task_id
                    .as_ref()
                    .map(|id| format!(" ({id})"))
                    .unwrap_or_default();
                format!("- [{}]{}: {}", note.agent_role, task_ref, note.content)
            })
            .collect();

        Some(format!("## Notes from Agents\n{}", lines.join("\n")))
    }

    /// Create a new task, returns its ID.
    pub fn create(&mut self, name: &str, description: &str, dep_refs: Vec<String>) -> TaskId {
        let id = format!("task{:03}", self.next_task_num);
        self.next_task_num += 1;

        // Resolve dependencies - accept either task IDs or task names
        let dependencies: Vec<TaskId> = dep_refs
            .iter()
            .filter_map(|dep| self.resolve_dependency(dep))
            .collect();

        let task = Task {
            id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            status: TaskStatus::NotStarted,
            dependencies,
            created_at: Some(std::time::Instant::now()),
        };

        // Emit TaskCreated event with current depth for nested task visualization
        let _ = self.event_tx.send(LogEvent::TaskCreated {
            task_id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            dependencies: dep_refs,
            depth: self.depth,
        });

        self.tasks.insert(id.clone(), task);
        id
    }

    /// Resolve a dependency reference to a task ID.
    ///
    /// Accepts either:
    /// - A task ID directly (e.g., "task001")
    /// - A task name (e.g., "Setup database")
    fn resolve_dependency(&self, dep: &str) -> Option<TaskId> {
        // First, try as a direct task ID
        if self.tasks.contains_key(dep) {
            return Some(dep.to_string());
        }
        // Fall back to name lookup
        self.find_by_name(dep)
    }

    /// Find task by name (for dependency resolution).
    pub fn find_by_name(&self, name: &str) -> Option<TaskId> {
        self.tasks
            .values()
            .find(|t| t.name == name)
            .map(|t| t.id.clone())
    }

    /// Get all tasks ready to execute (dependencies complete).
    /// Note: Part of the API for future orchestration, used in tests.
    #[allow(dead_code)]
    pub fn get_ready_tasks(&self) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| {
                matches!(t.status, TaskStatus::NotStarted)
                    && t.dependencies.iter().all(|dep_id| {
                        self.tasks
                            .get(dep_id)
                            .is_some_and(|dep| matches!(dep.status, TaskStatus::Complete { .. }))
                    })
            })
            .collect()
    }

    /// Update task status.
    pub fn update_status(&mut self, id: &TaskId, status: &TaskStatus) {
        if let Some(task) = self.tasks.get_mut(id) {
            let old_status = task.status.clone();
            task.status = status.clone();

            // Emit event (ignore send errors)
            // Use as_display_str() for TUI-friendly status strings
            // Use current depth for nested task visualization
            let _ = self.event_tx.send(LogEvent::TaskStateChanged {
                task_id: id.clone(),
                name: task.name.clone(),
                old_status: old_status.as_display_str().to_string(),
                new_status: status.as_display_str().to_string(),
                depth: self.depth,
            });
        }
    }

    /// Get all tasks.
    /// Note: Part of the API for future orchestration, used in tests.
    #[allow(dead_code)]
    pub fn all_tasks(&self) -> Vec<&Task> {
        self.tasks.values().collect()
    }

    /// Get a task by its exact ID.
    ///
    /// Note: For lookups that should also support task names, use `get_by_id_or_name` instead.
    #[allow(dead_code)]
    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Get a task by ID or by name (fallback).
    ///
    /// This method first tries to find a task by exact ID match (e.g., "task001").
    /// If not found, it falls back to searching by name (e.g., "Setup database").
    /// This provides flexibility for agents that may reference tasks by either.
    pub fn get_by_id_or_name(&self, id_or_name: &str) -> Option<&Task> {
        // First try exact ID match
        if let Some(task) = self.tasks.get(id_or_name) {
            return Some(task);
        }
        // Fall back to name lookup
        self.tasks.values().find(|t| t.name == id_or_name)
    }

    /// List all available task IDs for debugging.
    pub fn list_task_ids(&self) -> Vec<String> {
        self.tasks.keys().cloned().collect()
    }

    /// Format summaries of completed dependencies for a task.
    /// Returns None if the task has no dependencies or no dependencies have summaries.
    ///
    /// Uses flexible lookup that accepts both task IDs and task names.
    pub fn format_dependency_summaries(&self, task_id: &str) -> Option<String> {
        let task = self.get_by_id_or_name(task_id)?;

        if task.dependencies.is_empty() {
            return None;
        }

        let summaries: Vec<String> = task
            .dependencies
            .iter()
            .filter_map(|dep_id| {
                let dep = self.tasks.get(dep_id)?;
                if let TaskStatus::Complete { summary, .. } = &dep.status {
                    Some(format!("- **{}**: {}", dep.name, summary))
                } else {
                    None
                }
            })
            .collect();

        if summaries.is_empty() {
            None
        } else {
            Some(format!(
                "## Context from Completed Dependencies\n{}",
                summaries.join("\n")
            ))
        }
    }

    /// Renumber tasks by execution order.
    ///
    /// Sorts tasks by (dependency_count, original_id) to establish execution order,
    /// then assigns new sequential IDs (task001, task002, ...) and updates all
    /// dependency references.
    ///
    /// This ensures task IDs match execution order, making logs easier to follow.
    pub fn renumber_by_execution_order(&mut self) {
        if self.tasks.is_empty() {
            return;
        }

        // Sort tasks by execution order: fewer dependencies first, then by original ID
        let mut sorted_tasks: Vec<_> = self.tasks.values().cloned().collect();
        sorted_tasks.sort_by(|a, b| {
            let a_deps = a.dependencies.len();
            let b_deps = b.dependencies.len();
            a_deps.cmp(&b_deps).then_with(|| a.id.cmp(&b.id))
        });

        // Create mapping from old IDs to new IDs
        let id_mapping: HashMap<TaskId, TaskId> = sorted_tasks
            .iter()
            .enumerate()
            .map(|(i, task)| (task.id.clone(), format!("task{:03}", i + 1)))
            .collect();

        // Rebuild tasks with new IDs and updated dependency references
        let mut new_tasks = HashMap::new();
        for task in sorted_tasks {
            let new_id = id_mapping.get(&task.id).unwrap().clone();
            let new_deps: Vec<TaskId> = task
                .dependencies
                .iter()
                .filter_map(|dep_id| id_mapping.get(dep_id).cloned())
                .collect();

            let new_task = Task {
                id: new_id.clone(),
                name: task.name,
                description: task.description,
                status: task.status,
                dependencies: new_deps,
                created_at: task.created_at,
            };
            new_tasks.insert(new_id, new_task);
        }

        self.tasks = new_tasks;
        self.next_task_num = self.tasks.len() as u32 + 1;

        tracing::debug!(
            "📋 Renumbered {} tasks by execution order",
            self.tasks.len()
        );
    }

    /// Format tasks as a structured plan for the orchestrator.
    /// Returns None if no tasks exist, otherwise (`task_count`, `formatted_plan`).
    ///
    /// The format includes task IDs so the orchestrator can reference them
    /// when spawning agents: `[task001] **Name**: Description`
    ///
    /// Tasks are sorted by task ID which reflects execution order after
    /// `renumber_by_execution_order()` has been called.
    ///
    /// Each task is separated by a blank line for better readability in logs.
    pub fn format_for_orchestrator(&self) -> Option<(usize, String)> {
        if self.tasks.is_empty() {
            return None;
        }

        // Sort tasks by ID (which reflects execution order after renumbering)
        let mut tasks: Vec<_> = self.tasks.values().collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));

        let mut lines = Vec::new();
        for task in &tasks {
            // Format dependencies if any
            let deps_str = if task.dependencies.is_empty() {
                String::new()
            } else {
                format!(" (depends on: {})", task.dependencies.join(", "))
            };
            lines.push(format!(
                "[{}] **{}**: {}{}",
                task.id, task.name, task.description, deps_str
            ));
        }

        // Join with double newlines for clear separation between tasks in logs
        Some((tasks.len(), lines.join("\n\n")))
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
    fn test_create_task_sequential_ids() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id1 = manager.create("Task 1", "Description 1", vec![]);
        let id2 = manager.create("Task 2", "Description 2", vec![]);
        let id3 = manager.create("Task 3", "Description 3", vec![]);

        // IDs should be sequential: task001, task002, task003
        assert_eq!(id1, "task001");
        assert_eq!(id2, "task002");
        assert_eq!(id3, "task003");
        assert_eq!(manager.all_tasks().len(), 3);
    }

    #[test]
    fn test_dependency_resolution_by_name() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        manager.create("Task A", "First task", vec![]);
        manager.create("Task B", "Depends on A", vec!["Task A".to_string()]);

        let all_tasks = manager.all_tasks();
        let task_b = all_tasks.iter().find(|t| t.name == "Task B").unwrap();
        assert_eq!(task_b.dependencies.len(), 1);
        // Dependency should be stored as task ID
        assert_eq!(task_b.dependencies[0], "task001");
    }

    #[test]
    fn test_dependency_resolution_by_id() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id_a = manager.create("Task A", "First task", vec![]);
        // Use the task ID directly instead of the name
        manager.create("Task B", "Depends on A", vec![id_a]);

        let all_tasks = manager.all_tasks();
        let task_b = all_tasks.iter().find(|t| t.name == "Task B").unwrap();
        assert_eq!(task_b.dependencies.len(), 1);
        assert_eq!(task_b.dependencies[0], "task001");
    }

    #[test]
    fn test_dependency_resolution_mixed() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id_a = manager.create("Task A", "First task", vec![]);
        manager.create("Task B", "Second task", vec![]);
        // Mix: use ID for Task A, name for Task B
        manager.create(
            "Task C",
            "Depends on A and B",
            vec![id_a, "Task B".to_string()],
        );

        let all_tasks = manager.all_tasks();
        let task_c = all_tasks.iter().find(|t| t.name == "Task C").unwrap();
        assert_eq!(task_c.dependencies.len(), 2);
        assert!(task_c.dependencies.contains(&"task001".to_string()));
        assert!(task_c.dependencies.contains(&"task002".to_string()));
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
        manager.update_status(
            &id_a,
            &TaskStatus::Complete {
                success: true,
                summary: "Done".to_string(),
            },
        );

        // Now B is ready
        assert_eq!(manager.get_ready_tasks().len(), 1);
        assert_eq!(manager.get_ready_tasks()[0].name, "Task B");
    }

    #[test]
    fn test_find_by_name() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id = manager.create("My Task", "Description", vec![]);

        assert_eq!(id, "task001");
        assert_eq!(manager.find_by_name("My Task"), Some(id));
        assert_eq!(manager.find_by_name("Nonexistent"), None);
    }

    #[test]
    fn test_get_task_by_id() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id = manager.create("My Task", "Description", vec![]);

        let task = manager.get(&id).unwrap();
        assert_eq!(task.name, "My Task");
        assert_eq!(task.description, "Description");

        assert!(manager.get(&"nonexistent".to_string()).is_none());
    }

    #[test]
    fn test_get_by_id_or_name() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id = manager.create("My Task", "Description", vec![]);
        assert_eq!(id, "task001");

        // Lookup by exact ID should work
        let task_by_id = manager.get_by_id_or_name("task001").unwrap();
        assert_eq!(task_by_id.name, "My Task");

        // Lookup by name should work
        let task_by_name = manager.get_by_id_or_name("My Task").unwrap();
        assert_eq!(task_by_name.id, "task001");
        assert_eq!(task_by_name.description, "Description");

        // Non-existent should return None
        assert!(manager.get_by_id_or_name("nonexistent").is_none());
        assert!(manager.get_by_id_or_name("task999").is_none());
    }

    #[test]
    fn test_list_task_ids() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        // Empty manager should return empty list
        assert!(manager.list_task_ids().is_empty());

        // Create some tasks
        manager.create("Task A", "Desc A", vec![]);
        manager.create("Task B", "Desc B", vec![]);
        manager.create("Task C", "Desc C", vec![]);

        let ids = manager.list_task_ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"task001".to_string()));
        assert!(ids.contains(&"task002".to_string()));
        assert!(ids.contains(&"task003".to_string()));
    }

    #[test]
    fn test_format_for_orchestrator() {
        let (tx, _) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        manager.create("Task A", "First task", vec![]);
        manager.create("Task B", "Second task", vec!["Task A".to_string()]);

        let (count, formatted) = manager.format_for_orchestrator().unwrap();
        assert_eq!(count, 2);

        // Task A has no deps, should come first
        assert!(formatted.contains("[task001] **Task A**: First task"));
        // Task B has deps, should include them
        assert!(formatted.contains("[task002] **Task B**: Second task (depends on: task001)"));
        // Tasks should be separated by double newlines for log readability
        assert!(
            formatted.contains("\n\n"),
            "Expected double newline between tasks"
        );
    }

    #[test]
    fn test_update_status_emits_task_state_changed_event() {
        use crate::logging::LogEvent;

        let (tx, mut rx) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        // Create a task
        let id = manager.create("Test Task", "Description", vec![]);

        // Drain the TaskCreated event
        let _ = rx.try_recv();

        // Update status to InProgress
        manager.update_status(
            &id,
            &TaskStatus::InProgress {
                agent_session: Some("impl-001".to_string()),
            },
        );

        // Verify TaskStateChanged event was emitted
        let event = rx.try_recv().expect("Expected TaskStateChanged event");
        match event {
            LogEvent::TaskStateChanged {
                task_id,
                name,
                old_status,
                new_status,
                ..
            } => {
                assert_eq!(task_id, "task001");
                assert_eq!(name, "Test Task");
                assert_eq!(old_status, "pending");
                assert_eq!(new_status, "in_progress");
            }
            _ => panic!("Expected TaskStateChanged event"),
        }

        // Update status to Complete
        manager.update_status(
            &id,
            &TaskStatus::Complete {
                success: true,
                summary: "Done".to_string(),
            },
        );

        let event = rx.try_recv().expect("Expected TaskStateChanged event");
        match event {
            LogEvent::TaskStateChanged {
                old_status,
                new_status,
                ..
            } => {
                assert_eq!(old_status, "in_progress");
                assert_eq!(new_status, "completed");
            }
            _ => panic!("Expected TaskStateChanged event"),
        }
    }

    #[test]
    fn test_update_status_emits_failed_state() {
        use crate::logging::LogEvent;

        let (tx, mut rx) = broadcast::channel(10);
        let mut manager = TaskManager::new(tx);

        let id = manager.create("Failing Task", "Will fail", vec![]);

        // Drain the TaskCreated event
        let _ = rx.try_recv();

        // Go to InProgress first
        manager.update_status(
            &id,
            &TaskStatus::InProgress {
                agent_session: None,
            },
        );
        let _ = rx.try_recv(); // Drain InProgress event

        // Update to Failed
        manager.update_status(
            &id,
            &TaskStatus::Failed {
                error: "Something went wrong".to_string(),
            },
        );

        let event = rx.try_recv().expect("Expected TaskStateChanged event");
        match event {
            LogEvent::TaskStateChanged {
                old_status,
                new_status,
                ..
            } => {
                assert_eq!(old_status, "in_progress");
                assert_eq!(new_status, "failed");
            }
            _ => panic!("Expected TaskStateChanged event"),
        }
    }
}
