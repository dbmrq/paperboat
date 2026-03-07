//! Task tracking for structured plans.
//!
//! This module provides types for tracking tasks in a structured plan.
//! Tasks have dependencies, statuses, and can be managed through the TaskManager.

mod manager;

pub use manager::{AgentNote, GoalSummary, TaskManager};

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
    /// Note: Part of the API for future use, not yet read in production code.
    #[serde(skip)]
    #[allow(dead_code)]
    pub created_at: Option<Instant>,
}

/// Status of a task in its lifecycle.
///
/// Tasks progress through these statuses as work is performed.
/// The status is serialized with a `type` tag for clean JSON representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task has not been started yet.
    NotStarted,
    /// Task is currently being worked on.
    InProgress {
        /// Session ID of the agent working on this task, if any.
        agent_session: Option<String>,
    },
    /// Task completed successfully.
    Complete {
        /// Whether the task was successful.
        success: bool,
        /// Summary of what was accomplished.
        summary: String,
    },
    /// Task failed with an error.
    Failed {
        /// Description of what went wrong.
        error: String,
    },
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::NotStarted
    }
}

