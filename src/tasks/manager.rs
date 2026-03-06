//! Task manager for CRUD operations and queries.

use super::{Task, TaskId, TaskStatus};
use crate::logging::LogEvent;
use std::collections::HashMap;
use tokio::sync::broadcast;

/// Manages tasks for a structured plan.
///
/// Provides CRUD operations and queries for tasks, including
/// dependency tracking and status updates.
pub struct TaskManager {
    tasks: HashMap<TaskId, Task>,
    event_tx: broadcast::Sender<LogEvent>,
}

impl TaskManager {
    /// Creates a new TaskManager with the given event sender.
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
    /// Note: Part of the API for future orchestration, used in tests.
    #[allow(dead_code)]
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

        let all_tasks = manager.all_tasks();
        let task_b = all_tasks.iter().find(|t| t.name == "Task B").unwrap();
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

        assert_eq!(manager.find_by_name("My Task"), Some(id));
        assert_eq!(manager.find_by_name("Nonexistent"), None);
    }
}

