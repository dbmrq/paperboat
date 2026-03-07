//! Task management error types.
//!
//! Errors related to task creation, status updates, and task lifecycle.

use thiserror::Error;

/// Errors that can occur during task operations.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum TaskError {
    /// Task with the given ID was not found.
    #[error("Task not found: '{task_id}'")]
    NotFound {
        /// The task ID that was not found.
        task_id: String,
        /// Suggestion for similar task IDs, if available.
        suggestion: Option<String>,
    },

    /// Invalid status transition attempted.
    #[error("Invalid status transition for task '{task_id}': cannot go from {from_status} to {to_status}")]
    InvalidStatusTransition {
        /// The task ID.
        task_id: String,
        /// Current status of the task.
        from_status: String,
        /// Attempted new status.
        to_status: String,
    },

    /// Task dependency failed.
    #[error("Dependency failed for task '{task_id}': dependency '{dependency_id}' {reason}")]
    DependencyFailed {
        /// The task that has the failed dependency.
        task_id: String,
        /// The dependency task ID that failed.
        dependency_id: String,
        /// Why the dependency is considered failed.
        reason: String,
    },

    /// Validation failed for task creation or update.
    #[error("Validation failed for task: {message}")]
    ValidationFailed {
        /// What validation check failed.
        message: String,
        /// The field that failed validation, if applicable.
        field: Option<String>,
    },

    /// Circular dependency detected.
    #[error("Circular dependency detected: {}", .cycle.join(" -> "))]
    CircularDependency {
        /// The task IDs forming the cycle.
        cycle: Vec<String>,
    },

    /// Cannot complete with pending tasks.
    #[error("Cannot complete: {count} task(s) still pending: {}", .task_ids.join(", "))]
    PendingTasks {
        /// Number of pending tasks.
        count: usize,
        /// IDs of the pending tasks.
        task_ids: Vec<String>,
    },

    /// Task is already in the requested state.
    #[error("Task '{task_id}' is already {status}")]
    AlreadyInState {
        /// The task ID.
        task_id: String,
        /// The current/requested status.
        status: String,
    },

    /// Duplicate task ID.
    #[error("Task with ID '{task_id}' already exists")]
    DuplicateId {
        /// The duplicate task ID.
        task_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_found_display() {
        let err = TaskError::NotFound {
            task_id: "task999".to_string(),
            suggestion: Some("Did you mean 'task001'?".to_string()),
        };
        let display = format!("{err}");
        assert!(display.contains("task999"));
    }

    #[test]
    fn test_invalid_status_transition_display() {
        let err = TaskError::InvalidStatusTransition {
            task_id: "task001".to_string(),
            from_status: "completed".to_string(),
            to_status: "not_started".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("task001"));
        assert!(display.contains("completed"));
        assert!(display.contains("not_started"));
    }

    #[test]
    fn test_dependency_failed_display() {
        let err = TaskError::DependencyFailed {
            task_id: "task002".to_string(),
            dependency_id: "task001".to_string(),
            reason: "failed with error".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("task002"));
        assert!(display.contains("task001"));
    }

    #[test]
    fn test_validation_failed_display() {
        let err = TaskError::ValidationFailed {
            message: "description cannot be empty".to_string(),
            field: Some("description".to_string()),
        };
        let display = format!("{err}");
        assert!(display.contains("description cannot be empty"));
    }

    #[test]
    fn test_circular_dependency_display() {
        let err = TaskError::CircularDependency {
            cycle: vec![
                "task001".to_string(),
                "task002".to_string(),
                "task001".to_string(),
            ],
        };
        let display = format!("{err}");
        assert!(display.contains("task001 -> task002 -> task001"));
    }

    #[test]
    fn test_pending_tasks_display() {
        let err = TaskError::PendingTasks {
            count: 2,
            task_ids: vec!["task001".to_string(), "task002".to_string()],
        };
        let display = format!("{err}");
        assert!(display.contains("2 task(s)"));
        assert!(display.contains("task001"));
        assert!(display.contains("task002"));
    }
}
