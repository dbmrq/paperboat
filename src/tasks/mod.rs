//! Task tracking for structured plans.
//!
//! This module provides types for tracking tasks in a structured plan.
//! Tasks have dependencies, statuses, and can be managed through the [`TaskManager`].
//!
//! # Task Lifecycle
//!
//! Tasks progress through the following status transitions:
//!
//! ```text
//! NotStarted ──┬──▶ InProgress ──┬──▶ Complete
//!              │                 └──▶ Failed
//!              │
//!              └──▶ Skipped (via skip_tasks or reconciliation)
//! ```
//!
//! ## Status Descriptions
//!
//! - **`NotStarted`**: Task has been created but no agent has been assigned to work on it.
//! - **`InProgress`**: An agent is actively working on this task. Tracks the agent's session ID.
//! - **`Complete`**: Task finished successfully. Includes a summary of what was accomplished.
//! - **`Failed`**: Task encountered an error. Includes an error description.
//! - **`Skipped`**: Task was explicitly skipped. This can happen in two ways:
//!   - **Orchestrator skip**: The orchestrator calls `skip_tasks` to mark tasks as no longer
//!     needed (e.g., already done by another task, no longer relevant, or blocked).
//!   - **Reconciliation skip**: When a nested orchestrator completes (via `decompose`),
//!     any remaining `NotStarted` child tasks are automatically marked as `Skipped`
//!     before restoring the parent's task state.
//!
//! # Reconciliation at Completion
//!
//! When an orchestrator attempts to complete with `success=true`, the system performs
//! reconciliation to ensure all tasks have a definitive final status:
//!
//! 1. Check for any tasks still in `NotStarted` status
//! 2. If pending tasks exist, reject the completion and prompt the orchestrator to either:
//!    - Spawn agents to execute the remaining tasks, or
//!    - Call `skip_tasks` to explicitly skip tasks that are not needed
//! 3. Only allow completion when all tasks are in a terminal state
//!
//! This ensures the task list provides an accurate audit trail of what was done.

mod manager;

pub use manager::{TaskManager, TaskManagerSnapshot};

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Unique identifier for a task.
pub type TaskId = String;

/// A task in a structured plan.
///
/// Tasks represent units of work that can be tracked, have dependencies
/// on other tasks, and go through various status transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier for this task.
    pub id: TaskId,
    /// Human-readable name for the task.
    pub name: String,
    /// Detailed description of what needs to be done.
    pub description: String,
    /// Current status of the task.
    pub status: TaskStatus,
    /// IDs of tasks that must complete before this task can start.
    pub dependencies: Vec<TaskId>,
    /// When this task was created (not serialized).
    /// Part of the API for future task timing analytics.
    #[serde(skip)]
    #[allow(dead_code)] // Public API for future task timing analytics
    pub created_at: Option<Instant>,
}

/// Status of a task in its lifecycle.
///
/// Tasks progress through these statuses as work is performed. See the module-level
/// documentation for the complete lifecycle diagram and transition rules.
///
/// The status is serialized with a `type` tag for clean JSON representation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task has not been started yet.
    ///
    /// This is the initial state for all tasks. Tasks remain in this state until
    /// an agent is spawned to work on them.
    #[default]
    NotStarted,
    /// Task is currently being worked on.
    ///
    /// Set when an agent begins working on a task. The `agent_session` field tracks
    /// which agent is working on the task for debugging and logging purposes.
    InProgress {
        /// Session ID of the agent working on this task, if any.
        agent_session: Option<String>,
    },
    /// Task completed successfully.
    ///
    /// Set when an agent finishes its work. The `success` field indicates whether
    /// the work was successful, and `summary` provides details about what was done.
    Complete {
        /// Whether the task was successful.
        success: bool,
        /// Summary of what was accomplished.
        summary: String,
    },
    /// Task failed with an error.
    ///
    /// Set when a task encounters an unrecoverable error during execution.
    Failed {
        /// Description of what went wrong.
        error: String,
    },
    /// Task was explicitly skipped and will not be executed.
    ///
    /// This status indicates the task was deliberately not pursued. Common reasons:
    /// - Already completed by another task
    /// - No longer relevant to the goal
    /// - Blocked by external factors
    /// - Nested orchestrator completed without addressing all child tasks
    ///
    /// The `reason` field provides context for why the task was skipped.
    Skipped {
        /// Reason why the task was skipped.
        reason: String,
    },
}

impl TaskStatus {
    /// Returns a simple string representation suitable for display in TUI.
    ///
    /// Returns one of: `pending`, `in_progress`, `completed`, `failed`, or `skipped`.
    pub const fn as_display_str(&self) -> &'static str {
        match self {
            Self::NotStarted => "pending",
            Self::InProgress { .. } => "in_progress",
            Self::Complete { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Skipped { .. } => "skipped",
        }
    }
}
