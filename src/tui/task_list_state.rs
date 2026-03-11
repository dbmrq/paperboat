//! Task list state management for the TUI.
//!
//! This module provides [`TaskListState`], which manages the task list display
//! state extracted from `LogEvent`s. It tracks task creation, status changes,
//! and provides navigation through the task list.

use std::collections::HashMap;

// ============================================================================
// Task Display
// ============================================================================

/// Represents a task's display state.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some fields stored for future task detail view
pub struct TaskDisplay {
    /// Task identifier
    pub task_id: String,
    /// Task name
    pub name: String,
    /// Task description
    pub description: String,
    /// Task status string
    pub status: String,
    /// Task dependencies
    pub dependencies: Vec<String>,
    /// Depth in hierarchy
    pub depth: u32,
}

// ============================================================================
// Task List State
// ============================================================================

/// Manages the task list from `LogEvent`s.
#[derive(Debug, Default)]
pub struct TaskListState {
    /// All tasks indexed by task ID.
    tasks: HashMap<String, TaskDisplay>,
    /// Ordered list of task IDs for display.
    task_order: Vec<String>,
    /// Currently selected task index
    pub selected_index: Option<usize>,
    /// Scroll offset
    pub scroll_offset: usize,
}

impl TaskListState {
    /// Build a stable internal key for a task occurrence in the hierarchy.
    fn task_key(task_id: &str, depth: u32) -> String {
        format!("{depth}:{task_id}")
    }

    /// Creates a new empty task list state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles a `TaskCreated` event.
    pub fn handle_task_created(
        &mut self,
        task_id: String,
        name: String,
        description: String,
        dependencies: Vec<String>,
        depth: u32,
    ) {
        let task = TaskDisplay {
            task_id: task_id.clone(),
            name,
            description,
            status: "pending".to_string(),
            dependencies,
            depth,
        };

        let key = Self::task_key(&task_id, depth);
        let is_new_task = self.tasks.insert(key.clone(), task).is_none();
        if is_new_task {
            self.task_order.push(key);
        }
    }

    /// Handles a `TaskStateChanged` event at the default depth (0).
    ///
    /// This is a convenience wrapper for tasks without hierarchical depth.
    /// For tasks with explicit depth, use [`handle_task_state_changed_at_depth`].
    #[cfg(test)]
    pub fn handle_task_state_changed(&mut self, task_id: &str, new_status: &str) {
        self.handle_task_state_changed_at_depth(task_id, new_status, 0);
    }

    /// Handles a `TaskStateChanged` event for a task at a specific depth.
    pub fn handle_task_state_changed_at_depth(
        &mut self,
        task_id: &str,
        new_status: &str,
        depth: u32,
    ) {
        let key = Self::task_key(task_id, depth);
        if let Some(task) = self.tasks.get_mut(&key) {
            task.status = new_status.to_string();
        }
    }

    /// Returns all tasks in display order.
    #[must_use]
    pub fn tasks(&self) -> Vec<&TaskDisplay> {
        self.task_order
            .iter()
            .filter_map(|id| self.tasks.get(id))
            .collect()
    }

    /// Returns the number of tasks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Returns true if there are no tasks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Gets a task by ID.
    #[cfg(test)]
    #[must_use]
    pub fn get_task(&self, task_id: &str) -> Option<&TaskDisplay> {
        self.tasks.values().find(|task| task.task_id == task_id)
    }

    /// Returns the currently selected task, if any.
    #[must_use]
    pub fn get_selected_task(&self) -> Option<&TaskDisplay> {
        self.selected_index
            .and_then(|idx| self.task_order.get(idx))
            .and_then(|id| self.tasks.get(id))
    }

    /// Selects the next task.
    pub fn select_next(&mut self) {
        if self.task_order.is_empty() {
            return;
        }

        self.selected_index = Some(match self.selected_index {
            Some(i) => (i + 1).min(self.task_order.len() - 1),
            None => 0,
        });
    }

    /// Selects the previous task.
    #[allow(clippy::missing_const_for_fn)] // Uses non-const Vec::is_empty
    pub fn select_previous(&mut self) {
        if self.task_order.is_empty() {
            return;
        }

        self.selected_index = Some(match self.selected_index {
            Some(i) => i.saturating_sub(1),
            None => 0,
        });
    }

    /// Selects a task by index with bounds checking.
    ///
    /// Only sets the selected index if it is within bounds.
    /// Used by mouse click handler to select tasks.
    pub const fn select_index(&mut self, index: usize) {
        if index < self.task_order.len() {
            self.selected_index = Some(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_list_state_handle_task_created() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "task-1".to_string(),
            "Task 1".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );

        assert_eq!(list.len(), 1);
        assert!(!list.is_empty());

        let task = list.get_task("task-1").unwrap();
        assert_eq!(task.name, "Task 1");
        assert_eq!(task.status, "pending");
    }

    #[test]
    fn test_nested_and_root_tasks_with_same_id_stay_distinct() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "task001".to_string(),
            "Root task".to_string(),
            "Top-level work".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "task001".to_string(),
            "Nested task".to_string(),
            "Child work".to_string(),
            vec![],
            1,
        );

        let tasks = list.tasks();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "Root task");
        assert_eq!(tasks[0].depth, 0);
        assert_eq!(tasks[1].name, "Nested task");
        assert_eq!(tasks[1].depth, 1);
    }

    #[test]
    fn test_duplicate_task_created_same_depth_does_not_duplicate_order() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "task001".to_string(),
            "Original".to_string(),
            "First version".to_string(),
            vec![],
            1,
        );
        list.handle_task_created(
            "task001".to_string(),
            "Updated".to_string(),
            "Second version".to_string(),
            vec![],
            1,
        );

        let tasks = list.tasks();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Updated");
        assert_eq!(tasks[0].depth, 1);
    }

    #[test]
    fn test_task_list_state_handle_state_changed() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "task-1".to_string(),
            "Task 1".to_string(),
            "Desc".to_string(),
            vec![],
            0,
        );

        list.handle_task_state_changed("task-1", "in_progress");

        let task = list.get_task("task-1").unwrap();
        assert_eq!(task.status, "in_progress");
    }

    #[test]
    fn test_task_state_change_uses_depth_to_update_correct_task() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "task001".to_string(),
            "Root task".to_string(),
            "Top-level work".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "task001".to_string(),
            "Nested task".to_string(),
            "Child work".to_string(),
            vec![],
            1,
        );

        list.handle_task_state_changed_at_depth("task001", "in_progress", 1);

        let tasks = list.tasks();
        assert_eq!(tasks[0].status, "pending");
        assert_eq!(tasks[1].status, "in_progress");
    }

    #[test]
    fn test_task_list_state_navigation() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "t1".to_string(),
            "T1".to_string(),
            "D".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "t2".to_string(),
            "T2".to_string(),
            "D".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "t3".to_string(),
            "T3".to_string(),
            "D".to_string(),
            vec![],
            0,
        );

        assert!(list.selected_index.is_none());

        list.select_next();
        assert_eq!(list.selected_index, Some(0));

        list.select_next();
        assert_eq!(list.selected_index, Some(1));

        list.select_previous();
        assert_eq!(list.selected_index, Some(0));
    }

    #[test]
    fn test_task_status_transitions() {
        let mut list = TaskListState::new();

        // Create a task - starts as pending
        list.handle_task_created(
            "task-1".to_string(),
            "Test Task".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );
        assert_eq!(list.get_task("task-1").unwrap().status, "pending");

        // Transition to in_progress
        list.handle_task_state_changed("task-1", "in_progress");
        assert_eq!(list.get_task("task-1").unwrap().status, "in_progress");

        // Transition to completed
        list.handle_task_state_changed("task-1", "completed");
        assert_eq!(list.get_task("task-1").unwrap().status, "completed");

        // Create another task for failure scenario
        list.handle_task_created(
            "task-2".to_string(),
            "Failing Task".to_string(),
            "Will fail".to_string(),
            vec![],
            0,
        );
        list.handle_task_state_changed("task-2", "in_progress");
        list.handle_task_state_changed("task-2", "failed");
        assert_eq!(list.get_task("task-2").unwrap().status, "failed");
    }

    #[test]
    fn test_task_state_change_for_unknown_task_is_ignored() {
        let mut list = TaskListState::new();

        // Try to change state of a task that doesn't exist - should not panic
        list.handle_task_state_changed("nonexistent", "in_progress");
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_select_index_within_bounds() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "t1".to_string(),
            "T1".to_string(),
            "D".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "t2".to_string(),
            "T2".to_string(),
            "D".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "t3".to_string(),
            "T3".to_string(),
            "D".to_string(),
            vec![],
            0,
        );

        // Select first task
        list.select_index(0);
        assert_eq!(list.selected_index, Some(0));

        // Select middle task
        list.select_index(1);
        assert_eq!(list.selected_index, Some(1));

        // Select last task
        list.select_index(2);
        assert_eq!(list.selected_index, Some(2));
    }

    #[test]
    fn test_select_index_out_of_bounds_ignored() {
        let mut list = TaskListState::new();

        list.handle_task_created(
            "t1".to_string(),
            "T1".to_string(),
            "D".to_string(),
            vec![],
            0,
        );
        list.handle_task_created(
            "t2".to_string(),
            "T2".to_string(),
            "D".to_string(),
            vec![],
            0,
        );

        // Set a valid selection first
        list.select_index(0);
        assert_eq!(list.selected_index, Some(0));

        // Try to select out of bounds - should be ignored
        list.select_index(5);
        assert_eq!(list.selected_index, Some(0)); // Still at 0

        // Try to select exactly at len - should be ignored
        list.select_index(2);
        assert_eq!(list.selected_index, Some(0)); // Still at 0
    }

    #[test]
    fn test_select_index_on_empty_list() {
        let mut list = TaskListState::new();

        // Try to select on empty list - should be ignored
        list.select_index(0);
        assert!(list.selected_index.is_none());
    }
}
