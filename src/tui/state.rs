//! TUI state management module.
//!
//! This module contains the [`TuiState`] struct which holds all UI state,
//! including agent tree navigation, agent output buffers, task lists,
//! and panel focus management.
//!
//! # State Components
//!
//! - [`TuiState`] - Main state container coordinating all UI state
//! - [`AgentTreeState`] - Manages hierarchical agent navigation and metadata (see [`super::agent_tree_state`])
//! - [`TaskListState`] - Manages task list from [`LogEvent`]s (see [`super::task_list_state`])
//! - [`FocusedPanel`] - Tracks which panel has keyboard focus

use crate::logging::LogEvent;

use super::agent_tree_state::{AgentNode, AgentTreeState};
use super::task_list_state::TaskListState;

// ============================================================================
// Focus Management
// ============================================================================

/// Represents which panel currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusedPanel {
    /// Agent tree navigation panel (left)
    #[default]
    AgentTree,
    /// Selected agent's output panel (center)
    AgentOutput,
    /// Task list panel (right)
    TaskList,
    /// Application logs panel (bottom)
    AppLogs,
}

impl FocusedPanel {
    /// Returns the next panel in the focus cycle (Tab key behavior).
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::AgentTree => Self::AgentOutput,
            Self::AgentOutput => Self::TaskList,
            Self::TaskList => Self::AppLogs,
            Self::AppLogs => Self::AgentTree,
        }
    }

    /// Returns the previous panel in the focus cycle (Shift+Tab behavior).
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::AgentTree => Self::AppLogs,
            Self::AgentOutput => Self::AgentTree,
            Self::TaskList => Self::AgentOutput,
            Self::AppLogs => Self::TaskList,
        }
    }
}

// ============================================================================
// Main TUI State
// ============================================================================

/// Main TUI state container.
///
/// This struct holds all state needed to render the TUI, including:
/// - Panel focus management
/// - Agent tree navigation and metadata
/// - Task list state
/// - Auto-follow mode
/// - Help visibility
#[derive(Debug)]
pub struct TuiState {
    /// Currently focused panel
    pub current_focus: FocusedPanel,
    /// Agent tree state (hierarchy + metadata + messages)
    pub agent_tree_state: AgentTreeState,
    /// Task list state
    pub task_list_state: TaskListState,
    /// Currently selected agent's `session_id`
    pub selected_agent_id: Option<String>,
    /// Whether auto-follow mode is enabled (follows new agents)
    pub auto_follow_enabled: bool,
    /// Whether the help panel is visible
    pub help_visible: bool,
    /// Status message to display in status bar
    pub status_message: Option<String>,
    /// App logs scroll offset
    pub app_logs_scroll: u16,
    /// Agent output scroll position (lines from top)
    pub agent_output_scroll: u16,
    /// Last known message count for auto-scroll detection
    pub last_message_count: usize,
}

impl Default for TuiState {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiState {
    /// Creates a new TUI state with default values.
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_focus: FocusedPanel::default(),
            agent_tree_state: AgentTreeState::new(),
            task_list_state: TaskListState::new(),
            selected_agent_id: None,
            auto_follow_enabled: true,
            help_visible: false,
            status_message: None,
            app_logs_scroll: 0,
            agent_output_scroll: 0,
            last_message_count: 0,
        }
    }

    /// Processes an incoming [`LogEvent`] and updates state accordingly.
    pub fn handle_event(&mut self, event: LogEvent) {
        match event {
            LogEvent::AgentStarted {
                agent_type,
                session_id,
                depth,
                task,
            } => {
                self.agent_tree_state.handle_agent_started(
                    agent_type,
                    session_id.clone(),
                    depth,
                    task,
                );

                // Auto-follow: select the new agent if enabled
                if self.auto_follow_enabled {
                    self.selected_agent_id = Some(session_id.clone());
                    self.agent_tree_state.select(&session_id);
                }
            }

            LogEvent::AgentMessage {
                session_id,
                content,
                ..
            } => {
                self.agent_tree_state
                    .handle_agent_message(session_id.as_deref(), &content);
            }

            LogEvent::AgentComplete {
                session_id,
                success,
                ..
            } => {
                self.agent_tree_state
                    .handle_agent_complete(session_id.as_deref(), success);
            }

            LogEvent::TaskCreated {
                task_id,
                name,
                description,
                dependencies,
                depth,
            } => {
                self.task_list_state.handle_task_created(
                    task_id,
                    name,
                    description,
                    dependencies,
                    depth,
                );
            }

            LogEvent::TaskStateChanged {
                task_id,
                new_status,
                ..
            } => {
                self.task_list_state
                    .handle_task_state_changed(&task_id, &new_status);
            }

            // Tool events update the message buffer for the agent (standalone lines)
            LogEvent::ToolCall {
                session_id,
                tool_name,
                ..
            } => {
                let msg = format!("> Calling: {tool_name}");
                self.agent_tree_state
                    .handle_standalone_message(session_id.as_deref(), &msg);
            }

            LogEvent::ToolProgress {
                session_id,
                progress_text,
                ..
            } => {
                // Tool progress is streaming content, concatenate it
                self.agent_tree_state
                    .handle_agent_message(session_id.as_deref(), &progress_text);
            }

            LogEvent::ToolResult {
                session_id,
                tool_name,
                is_error,
                ..
            } => {
                let icon = if is_error { "✗" } else { "✓" };
                let msg = format!("{icon} {tool_name} completed");
                self.agent_tree_state
                    .handle_standalone_message(session_id.as_deref(), &msg);
            }

            LogEvent::SubtaskCreated {
                parent_depth,
                new_depth,
                path,
                task_description,
            } => {
                // Notify the agent tree state about the subtask creation
                self.agent_tree_state
                    .handle_subtask_created(parent_depth, new_depth, &path);

                // SubtaskCreated creates a new scope but the actual agent
                // will be created via AgentStarted. We can log it.
                let msg = format!("+ Subtask: {task_description}");
                // Add to the most recent agent's messages
                // Clone the id to avoid borrow checker issues
                if let Some(id) = self
                    .agent_tree_state
                    .most_recent_session_id()
                    .map(str::to_string)
                {
                    self.agent_tree_state
                        .handle_standalone_message(Some(&id), &msg);
                }
            }
        }
    }

    /// Cycles focus to the next panel (Tab key handler).
    pub const fn cycle_focus(&mut self) {
        self.current_focus = self.current_focus.next();
    }

    /// Cycles focus to the previous panel (Shift+Tab key handler).
    pub const fn cycle_focus_reverse(&mut self) {
        self.current_focus = self.current_focus.prev();
    }

    /// Toggles auto-follow mode (f key handler).
    pub fn toggle_auto_follow(&mut self) {
        self.auto_follow_enabled = !self.auto_follow_enabled;
        self.status_message = Some(if self.auto_follow_enabled {
            "Auto-follow: ON".to_string()
        } else {
            "Auto-follow: OFF".to_string()
        });
    }

    /// Toggles help visibility (? key handler).
    pub const fn toggle_help(&mut self) {
        self.help_visible = !self.help_visible;
    }

    /// Sets the status message.
    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    /// Clears the status message.
    #[allow(dead_code)] // May be used in future feature additions
    pub fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    /// Returns the messages for the currently selected agent.
    #[must_use]
    pub fn selected_agent_messages(&self) -> Option<&Vec<String>> {
        self.selected_agent_id
            .as_ref()
            .and_then(|id| self.agent_tree_state.get_messages(id))
    }

    /// Returns the currently selected agent's metadata.
    #[must_use]
    pub fn selected_agent(&self) -> Option<&AgentNode> {
        self.selected_agent_id
            .as_ref()
            .and_then(|id| self.agent_tree_state.get_agent(id))
    }

    /// Manually selects an agent by `session_id` (disables auto-follow).
    #[cfg(test)]
    pub fn select_agent(&mut self, session_id: &str) {
        self.selected_agent_id = Some(session_id.to_string());
        self.agent_tree_state.select(session_id);
        // Manual selection disables auto-follow
        self.auto_follow_enabled = false;
        self.status_message = Some("Auto-follow disabled (manual selection)".to_string());
    }

    // ========================================================================
    // Status Bar Helper Methods
    // ========================================================================

    /// Returns the total number of agents in the tree.
    #[must_use]
    pub fn get_agent_count(&self) -> usize {
        self.agent_tree_state.agent_count()
    }

    /// Returns task progress as (completed, total) tuple.
    ///
    /// A task is considered "completed" if its status is "completed".
    #[must_use]
    pub fn get_task_progress(&self) -> (usize, usize) {
        let tasks = self.task_list_state.tasks();
        let total = tasks.len();
        let completed = tasks.iter().filter(|t| t.status == "completed").count();
        (completed, total)
    }

    /// Returns true if any agents are still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.agent_tree_state.has_running_agents()
    }

    /// Returns agent statistics: (succeeded, failed, in_progress, total, active).
    ///
    /// - `succeeded`: Number of agents that completed successfully
    /// - `failed`: Number of agents that failed
    /// - `in_progress`: Number of agents currently running
    /// - `total`: Total number of agents (succeeded + failed + in_progress)
    /// - `active`: Same as in_progress (number of currently running agents)
    #[must_use]
    pub fn get_agent_stats(&self) -> (usize, usize, usize, usize, usize) {
        let (succeeded, failed, in_progress) = self.agent_tree_state.count_agents_by_status();
        let total = succeeded + failed + in_progress;
        let active = in_progress;
        (succeeded, failed, in_progress, total, active)
    }

    // ========================================================================
    // Task Detail Helper Methods
    // ========================================================================

    /// Returns true if the task detail view should be shown in the middle panel.
    ///
    /// The task detail view is shown when the Tasks panel is focused AND
    /// a task is selected in the task list.
    #[must_use]
    pub fn should_show_task_detail(&self) -> bool {
        self.current_focus == FocusedPanel::TaskList
            && self.task_list_state.get_selected_task().is_some()
    }

    /// Returns the currently selected task, if the task detail view should be shown.
    ///
    /// This is a convenience method that combines `should_show_task_detail` check
    /// with returning the selected task.
    #[must_use]
    pub fn selected_task(&self) -> Option<&super::task_list_state::TaskDisplay> {
        if self.current_focus == FocusedPanel::TaskList {
            self.task_list_state.get_selected_task()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focused_panel_cycle() {
        let panel = FocusedPanel::AgentTree;
        assert_eq!(panel.next(), FocusedPanel::AgentOutput);
        assert_eq!(panel.next().next(), FocusedPanel::TaskList);
        assert_eq!(panel.next().next().next(), FocusedPanel::AppLogs);
        assert_eq!(panel.next().next().next().next(), FocusedPanel::AgentTree);
    }

    #[test]
    fn test_focused_panel_cycle_reverse() {
        let panel = FocusedPanel::AgentTree;
        assert_eq!(panel.prev(), FocusedPanel::AppLogs);
        assert_eq!(panel.prev().prev(), FocusedPanel::TaskList);
        assert_eq!(panel.prev().prev().prev(), FocusedPanel::AgentOutput);
        assert_eq!(panel.prev().prev().prev().prev(), FocusedPanel::AgentTree);
    }

    #[test]
    fn test_tui_state_new() {
        let state = TuiState::new();
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
        assert!(state.auto_follow_enabled);
        assert!(!state.help_visible);
        assert!(state.selected_agent_id.is_none());
    }

    #[test]
    fn test_tui_state_cycle_focus() {
        let mut state = TuiState::new();
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);

        state.cycle_focus();
        assert_eq!(state.current_focus, FocusedPanel::AgentOutput);

        state.cycle_focus();
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_tui_state_toggle_auto_follow() {
        let mut state = TuiState::new();
        assert!(state.auto_follow_enabled);

        state.toggle_auto_follow();
        assert!(!state.auto_follow_enabled);
        assert!(state.status_message.is_some());

        state.toggle_auto_follow();
        assert!(state.auto_follow_enabled);
    }

    #[test]
    fn test_tui_state_handle_agent_started_event() {
        use crate::logging::AgentType;
        let mut state = TuiState::new();

        let event = LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Test task".to_string(),
        };

        state.handle_event(event);

        // Auto-follow should have selected the agent
        assert_eq!(state.selected_agent_id, Some("sess-1".to_string()));
        assert_eq!(state.agent_tree_state.agent_count(), 1);
    }

    #[test]
    fn test_tui_state_handle_task_events() {
        let mut state = TuiState::new();

        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "Do something".to_string(),
            dependencies: vec![],
            depth: 0,
        });

        assert_eq!(state.task_list_state.len(), 1);

        state.handle_event(LogEvent::TaskStateChanged {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            old_status: "pending".to_string(),
            new_status: "completed".to_string(),
            depth: 0,
        });

        let task = state.task_list_state.get_task("task-1").unwrap();
        assert_eq!(task.status, "completed");
    }

    #[test]
    fn test_tui_state_select_agent_disables_auto_follow() {
        let mut state = TuiState::new();
        assert!(state.auto_follow_enabled);

        state.select_agent("some-session");

        assert!(!state.auto_follow_enabled);
        assert_eq!(state.selected_agent_id, Some("some-session".to_string()));
    }

    #[test]
    fn test_tui_state_get_agent_stats() {
        use crate::logging::AgentType;
        let mut state = TuiState::new();

        // Initially empty
        let (succeeded, failed, in_progress, total, active) = state.get_agent_stats();
        assert_eq!(succeeded, 0);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 0);
        assert_eq!(total, 0);
        assert_eq!(active, 0);

        // Add running agents
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Task 1".to_string(),
        });
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Planner,
            session_id: "sess-2".to_string(),
            depth: 0,
            task: "Task 2".to_string(),
        });

        let (succeeded, failed, in_progress, total, active) = state.get_agent_stats();
        assert_eq!(succeeded, 0);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 2);
        assert_eq!(total, 2);
        assert_eq!(active, 2);

        // Complete one successfully
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("sess-1".to_string()),
            depth: 0,
            success: true,
        });

        let (succeeded, failed, in_progress, total, active) = state.get_agent_stats();
        assert_eq!(succeeded, 1);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 1);
        assert_eq!(total, 2);
        assert_eq!(active, 1);

        // Fail one
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Planner,
            session_id: Some("sess-2".to_string()),
            depth: 0,
            success: false,
        });

        let (succeeded, failed, in_progress, total, active) = state.get_agent_stats();
        assert_eq!(succeeded, 1);
        assert_eq!(failed, 1);
        assert_eq!(in_progress, 0);
        assert_eq!(total, 2);
        assert_eq!(active, 0);
    }
}
