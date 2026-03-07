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
}

impl TaskManager {
    /// Creates a new TaskManager with the given event sender.
    pub fn new(event_tx: broadcast::Sender<LogEvent>) -> Self {
        Self {
            tasks: HashMap::new(),
            event_tx,
            next_task_num: 1,
            goal: None,
            notes: Vec::new(),
        }
    }

    /// Set the goal summary.
    pub fn set_goal(&mut self, summary: String, acceptance_criteria: Option<String>) {
        self.goal = Some(GoalSummary {
            summary,
            acceptance_criteria,
        });
        tracing::info!("📎 Goal set: {}", self.goal.as_ref().unwrap().summary);
    }

    /// Get the goal summary.
    pub fn get_goal(&self) -> Option<&GoalSummary> {
        self.goal.as_ref()
    }

    /// Format the goal for inclusion in prompts.
    pub fn format_goal(&self) -> String {
        match &self.goal {
            Some(goal) => {
                let mut result = format!("**Goal**: {}", goal.summary);
                if let Some(criteria) = &goal.acceptance_criteria {
                    result.push_str(&format!("\n\n**Acceptance Criteria**: {}", criteria));
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

    /// Get all notes.
    pub fn get_notes(&self) -> &[AgentNote] {
        &self.notes
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
                    .map(|id| format!(" ({})", id))
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

        // Emit TaskCreated event
        let _ = self.event_tx.send(LogEvent::TaskCreated {
            task_id: id.clone(),
            name: name.to_string(),
            description: description.to_string(),
            dependencies: dep_refs,
            depth: 0,
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
    /// Note: Part of the API for future orchestration, used in tests.
    #[allow(dead_code)]
    pub fn all_tasks(&self) -> Vec<&Task> {
        self.tasks.values().collect()
    }

    /// Get a task by its ID.
    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Check if any tasks have been created.
    pub fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// Format summaries of completed dependencies for a task.
    /// Returns None if the task has no dependencies or no dependencies have summaries.
    pub fn format_dependency_summaries(&self, task_id: &TaskId) -> Option<String> {
        let task = self.tasks.get(task_id)?;

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
            Some(format!("## Context from Completed Dependencies\n{}", summaries.join("\n")))
        }
    }

    /// Format tasks as a structured plan for the orchestrator.
    /// Returns None if no tasks exist, otherwise (task_count, formatted_plan).
    ///
    /// The format includes task IDs so the orchestrator can reference them
    /// when spawning agents: `[task001] **Name**: Description`
    pub fn format_for_orchestrator(&self) -> Option<(usize, String)> {
        if self.tasks.is_empty() {
            return None;
        }

        // Sort tasks: those with no dependencies first, then by ID for stable ordering
        let mut tasks: Vec<_> = self.tasks.values().collect();
        tasks.sort_by(|a, b| {
            let a_deps = a.dependencies.len();
            let b_deps = b.dependencies.len();
            a_deps.cmp(&b_deps).then_with(|| a.id.cmp(&b.id))
        });

        let mut lines = Vec::new();
        for task in tasks.iter() {
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

        Some((tasks.len(), lines.join("\n")))
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
        manager.create("Task B", "Depends on A", vec![id_a.clone()]);

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
        manager.create("Task C", "Depends on A and B", vec![id_a, "Task B".to_string()]);

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
            TaskStatus::Complete {
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
    }
}

