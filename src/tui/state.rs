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
//! - [`ModelConfigUpdate`] - Message type for sending model configuration changes to the App (see [`super::model_config_update`])

use tui_logger::TuiWidgetState;

use crate::logging::LogEvent;
use crate::models::{AvailableModel, ModelConfig};

use super::agent_tree_state::{AgentNode, AgentTreeState};
// Re-export for backward compatibility (used by app.rs)
pub use super::model_config_update::ModelConfigUpdate;
use super::task_list_state::TaskListState;
use super::widgets::{create_app_logs_state, SettingsState};

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
/// - Help and settings visibility
/// - Animation frame counter for UI animations
/// - Model configuration for display and editing
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
    /// Whether the settings panel is visible
    pub settings_visible: bool,
    /// Settings panel state (model selection, pending changes)
    pub settings_state: SettingsState,
    /// Status message to display in status bar
    pub status_message: Option<String>,
    /// App logs scroll offset
    pub app_logs_scroll: u16,
    /// Agent output scroll position (lines from top)
    pub agent_output_scroll: u16,
    /// Task detail scroll position (lines from top)
    pub task_detail_scroll: u16,
    /// Last known message count for auto-scroll detection
    pub last_message_count: usize,
    /// Last selected task ID for scroll reset detection
    pub last_selected_task_id: Option<String>,
    /// Animation frame counter for running agent indicators.
    /// Increments each render frame; use `animation_frame % 30` to toggle every ~500ms at 60 FPS.
    pub animation_frame: u32,
    /// Current model configuration (clone for display purposes)
    pub model_config: ModelConfig,
    /// List of available models for selection
    pub available_models: Vec<AvailableModel>,
    /// Pending config update to send to the App (polled by event loop)
    pub pending_config_update: Option<ModelConfigUpdate>,
    /// Logger widget state for tui-logger (target selector, level filtering)
    pub logger_state: TuiWidgetState,
}

// Manual Debug implementation because TuiWidgetState doesn't implement Debug
impl std::fmt::Debug for TuiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiState")
            .field("current_focus", &self.current_focus)
            .field("agent_tree_state", &self.agent_tree_state)
            .field("task_list_state", &self.task_list_state)
            .field("selected_agent_id", &self.selected_agent_id)
            .field("auto_follow_enabled", &self.auto_follow_enabled)
            .field("help_visible", &self.help_visible)
            .field("settings_visible", &self.settings_visible)
            .field("settings_state", &self.settings_state)
            .field("status_message", &self.status_message)
            .field("app_logs_scroll", &self.app_logs_scroll)
            .field("agent_output_scroll", &self.agent_output_scroll)
            .field("task_detail_scroll", &self.task_detail_scroll)
            .field("last_message_count", &self.last_message_count)
            .field("last_selected_task_id", &self.last_selected_task_id)
            .field("animation_frame", &self.animation_frame)
            .field("model_config", &self.model_config)
            .field("available_models", &self.available_models)
            .field("pending_config_update", &self.pending_config_update)
            .field("logger_state", &"<TuiWidgetState>")
            .finish()
    }
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
            settings_visible: false,
            settings_state: SettingsState::new(),
            status_message: None,
            app_logs_scroll: 0,
            agent_output_scroll: 0,
            task_detail_scroll: 0,
            last_message_count: 0,
            last_selected_task_id: None,
            animation_frame: 0,
            model_config: ModelConfig::default(),
            available_models: Vec::new(),
            pending_config_update: None,
            logger_state: create_app_logs_state(),
        }
    }

    /// Creates a new TUI state with the provided model configuration.
    ///
    /// This constructor should be used when the TUI has access to the initial
    /// model configuration from the main application.
    #[must_use]
    pub fn with_model_config(model_config: ModelConfig) -> Self {
        let available_models = model_config.available_models.clone();
        Self {
            current_focus: FocusedPanel::default(),
            agent_tree_state: AgentTreeState::new(),
            task_list_state: TaskListState::new(),
            selected_agent_id: None,
            auto_follow_enabled: true,
            help_visible: false,
            settings_visible: false,
            settings_state: SettingsState::new(),
            status_message: None,
            app_logs_scroll: 0,
            agent_output_scroll: 0,
            task_detail_scroll: 0,
            last_message_count: 0,
            last_selected_task_id: None,
            animation_frame: 0,
            model_config,
            available_models,
            pending_config_update: None,
            logger_state: create_app_logs_state(),
        }
    }

    /// Toggles settings visibility (s key handler).
    pub const fn toggle_settings(&mut self) {
        self.settings_visible = !self.settings_visible;
    }

    /// Applies pending settings changes and prepares an update for the App.
    ///
    /// This method:
    /// 1. Creates a `ModelConfigUpdate` from any pending changes in `SettingsState`
    /// 2. Applies changes to the local `model_config` for immediate UI feedback
    /// 3. Stores the update in `pending_config_update` for the event loop to send
    /// 4. Clears the settings state pending changes
    /// 5. Sets a status message indicating the changes were applied
    ///
    /// Returns `true` if any changes were applied, `false` if there were no pending changes.
    pub fn apply_settings_changes(&mut self) -> bool {
        if !self.settings_state.has_pending_changes() {
            return false;
        }

        // Build the update from pending changes
        let update = ModelConfigUpdate {
            orchestrator_model: self.settings_state.pending_orchestrator,
            planner_model: self.settings_state.pending_planner,
            implementer_model: self.settings_state.pending_implementer,
        };

        // Apply changes to local config for immediate UI feedback
        if let Some(model) = update.orchestrator_model {
            self.model_config.orchestrator_model = model;
        }
        if let Some(model) = update.planner_model {
            self.model_config.planner_model = model;
        }
        if let Some(model) = update.implementer_model {
            self.model_config.implementer_model = model;
        }

        // Store update for the event loop to send to the App
        self.pending_config_update = Some(update);

        // Clear pending changes in settings state
        self.settings_state.clear_pending();

        // Provide user feedback
        self.status_message = Some("Model settings saved".to_string());

        true
    }

    /// Takes the pending config update, if any, for sending to the App.
    ///
    /// This is called by the event loop to get updates to send via the config channel.
    /// Returns `None` if there's no pending update.
    pub const fn take_pending_config_update(&mut self) -> Option<ModelConfigUpdate> {
        self.pending_config_update.take()
    }

    /// Updates the model configuration.
    ///
    /// This is called when the TUI receives an updated configuration,
    /// typically after the user makes changes in a settings panel.
    #[allow(dead_code)] // Public API for external TUI configuration
    pub fn update_model_config(&mut self, config: ModelConfig) {
        self.available_models.clone_from(&config.available_models);
        self.model_config = config;
    }

    /// Returns a reference to the current model configuration.
    #[must_use]
    #[allow(dead_code)] // Public API for external TUI state access
    pub const fn model_config(&self) -> &ModelConfig {
        &self.model_config
    }

    /// Returns a reference to the available models list.
    #[must_use]
    #[allow(dead_code)] // Public API for external TUI state access
    pub fn available_models(&self) -> &[AvailableModel] {
        &self.available_models
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
    pub fn cycle_focus(&mut self) {
        self.on_focus_changed(self.current_focus.next());
    }

    /// Cycles focus to the previous panel (Shift+Tab key handler).
    pub fn cycle_focus_reverse(&mut self) {
        self.on_focus_changed(self.current_focus.prev());
    }

    /// Handles a focus change to a new panel.
    ///
    /// This method centralizes focus change logic including:
    /// - Setting the new focus
    /// - Auto-selecting the first task when focusing the task list panel
    /// - Clearing task selection when focusing agent tree or app logs panels
    /// - Preserving task selection when focusing agent output panel
    pub fn on_focus_changed(&mut self, new_focus: FocusedPanel) {
        self.current_focus = new_focus;

        match new_focus {
            FocusedPanel::TaskList => {
                // Auto-select first task when focusing TaskList if tasks exist and none selected
                if !self.task_list_state.is_empty() && self.task_list_state.selected_index.is_none()
                {
                    self.task_list_state.selected_index = Some(0);
                }
            }
            FocusedPanel::AgentTree | FocusedPanel::AppLogs => {
                // Clear task selection when focusing agent-related panels
                self.task_list_state.selected_index = None;
            }
            FocusedPanel::AgentOutput => {
                // Keep task selection unchanged - allows scrolling agent output
                // while keeping task detail visible
            }
        }
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
    #[allow(dead_code)] // Public API for status bar updates
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
    #[allow(dead_code)] // Public API for external status display
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

    /// Returns agent statistics: (succeeded, failed, in progress, total, active).
    ///
    /// - `succeeded`: Number of agents that completed successfully
    /// - `failed`: Number of agents that failed
    /// - `in progress`: Number of agents currently running
    /// - `total`: Total number of agents (succeeded + failed + in progress)
    /// - `active`: Same as in progress (number of currently running agents)
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
    #[allow(dead_code)] // Public API for layout decision making
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

    // ========================================================================
    // Settings State Tests
    // ========================================================================

    #[test]
    fn test_tui_state_toggle_settings() {
        let mut state = TuiState::new();
        assert!(!state.settings_visible);

        state.toggle_settings();
        assert!(state.settings_visible);

        state.toggle_settings();
        assert!(!state.settings_visible);
    }

    #[test]
    fn test_tui_state_with_model_config() {
        use crate::models::{AvailableModel, ModelConfig, ModelId};

        let mut config = ModelConfig::default();
        config.orchestrator_model = ModelId::Opus4_5;
        config.planner_model = ModelId::Sonnet4_5;
        config.implementer_model = ModelId::Haiku4_5;
        config.available_models = vec![
            AvailableModel {
                id: ModelId::Opus4_5,
                name: "Opus 4.5".to_string(),
                description: "Most capable".to_string(),
            },
            AvailableModel {
                id: ModelId::Sonnet4_5,
                name: "Sonnet 4.5".to_string(),
                description: "Balanced".to_string(),
            },
        ];

        let state = TuiState::with_model_config(config.clone());

        assert_eq!(state.model_config.orchestrator_model, ModelId::Opus4_5);
        assert_eq!(state.model_config.planner_model, ModelId::Sonnet4_5);
        assert_eq!(state.model_config.implementer_model, ModelId::Haiku4_5);
        assert_eq!(state.available_models.len(), 2);
    }

    #[test]
    fn test_tui_state_update_model_config() {
        use crate::models::{AvailableModel, ModelConfig, ModelId};

        let mut state = TuiState::new();

        let mut new_config = ModelConfig::default();
        new_config.orchestrator_model = ModelId::Opus4_5;
        new_config.available_models = vec![AvailableModel {
            id: ModelId::Opus4_5,
            name: "Opus 4.5".to_string(),
            description: "Powerful".to_string(),
        }];

        state.update_model_config(new_config);

        assert_eq!(state.model_config.orchestrator_model, ModelId::Opus4_5);
        assert_eq!(state.available_models.len(), 1);
        assert_eq!(state.available_models[0].id, ModelId::Opus4_5);
    }

    #[test]
    fn test_tui_state_model_config_getter() {
        use crate::models::{ModelConfig, ModelId};

        let mut config = ModelConfig::default();
        config.orchestrator_model = ModelId::Sonnet4_5;
        let state = TuiState::with_model_config(config);

        let config_ref = state.model_config();
        assert_eq!(config_ref.orchestrator_model, ModelId::Sonnet4_5);
    }

    #[test]
    fn test_tui_state_available_models_getter() {
        use crate::models::{AvailableModel, ModelConfig, ModelId};

        let mut config = ModelConfig::default();
        config.available_models = vec![AvailableModel {
            id: ModelId::Haiku4_5,
            name: "Haiku".to_string(),
            description: "Fast".to_string(),
        }];
        let state = TuiState::with_model_config(config);

        let models = state.available_models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, ModelId::Haiku4_5);
    }

    #[test]
    fn test_tui_state_settings_state_initialization() {
        use crate::tui::widgets::SelectedAgentType;

        let state = TuiState::new();

        // Settings state should be properly initialized
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Orchestrator
        );
        assert_eq!(state.settings_state.selected_model_index, 0);
        assert!(!state.settings_state.has_pending_changes());
    }

    #[test]
    fn test_tui_state_settings_and_help_independent() {
        let mut state = TuiState::new();

        // Both should start false
        assert!(!state.help_visible);
        assert!(!state.settings_visible);

        // Toggle settings
        state.toggle_settings();
        assert!(state.settings_visible);
        assert!(!state.help_visible);

        // Toggle help (independent)
        state.toggle_help();
        assert!(state.settings_visible);
        assert!(state.help_visible);

        // Toggle settings off
        state.toggle_settings();
        assert!(!state.settings_visible);
        assert!(state.help_visible);
    }

    // ========================================================================
    // ModelConfigUpdate Tests
    // ========================================================================

    // ========================================================================
    // apply_settings_changes Tests
    // ========================================================================

    #[test]
    fn test_apply_settings_changes_no_pending() {
        let mut state = TuiState::new();

        // No pending changes should return false and not create an update
        let applied = state.apply_settings_changes();
        assert!(!applied);
        assert!(state.pending_config_update.is_none());
        assert!(state.status_message.is_none());
    }

    #[test]
    fn test_apply_settings_changes_with_pending() {
        use crate::models::{AvailableModel, ModelConfig, ModelId};

        let mut config = ModelConfig::default();
        config.orchestrator_model = ModelId::Haiku4_5;
        config.planner_model = ModelId::Haiku4_5;
        config.implementer_model = ModelId::Haiku4_5;
        config.available_models = vec![AvailableModel {
            id: ModelId::Opus4_5,
            name: "Opus".to_string(),
            description: "".to_string(),
        }];

        let mut state = TuiState::with_model_config(config);

        // Set pending changes
        state.settings_state.pending_orchestrator = Some(ModelId::Opus4_5);
        state.settings_state.pending_planner = Some(ModelId::Sonnet4_5);

        // Apply changes
        let applied = state.apply_settings_changes();
        assert!(applied);

        // Check local config was updated
        assert_eq!(state.model_config.orchestrator_model, ModelId::Opus4_5);
        assert_eq!(state.model_config.planner_model, ModelId::Sonnet4_5);
        assert_eq!(state.model_config.implementer_model, ModelId::Haiku4_5); // Unchanged

        // Check pending update was created
        let update = state.pending_config_update.unwrap();
        assert_eq!(update.orchestrator_model, Some(ModelId::Opus4_5));
        assert_eq!(update.planner_model, Some(ModelId::Sonnet4_5));
        assert!(update.implementer_model.is_none());

        // Check settings state was cleared
        assert!(!state.settings_state.has_pending_changes());

        // Check status message
        assert!(state.status_message.is_some());
    }

    #[test]
    fn test_take_pending_config_update() {
        use crate::models::ModelId;

        let mut state = TuiState::new();
        state.settings_state.pending_orchestrator = Some(ModelId::Opus4_5);

        // Apply to create the update
        state.apply_settings_changes();
        assert!(state.pending_config_update.is_some());

        // Take the update
        let update = state.take_pending_config_update();
        assert!(update.is_some());
        assert_eq!(update.unwrap().orchestrator_model, Some(ModelId::Opus4_5));

        // Second take should return None
        let update2 = state.take_pending_config_update();
        assert!(update2.is_none());
    }

    // ========================================================================
    // on_focus_changed Tests
    // ========================================================================

    #[test]
    fn test_on_focus_changed_task_list_auto_selects_first_task() {
        let mut state = TuiState::new();

        // Add some tasks
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "First task".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-2".to_string(),
            name: "Task 2".to_string(),
            description: "Second task".to_string(),
            dependencies: vec![],
            depth: 0,
        });

        // Initially no task selected
        assert!(state.task_list_state.selected_index.is_none());

        // Focus task list - should auto-select first task
        state.on_focus_changed(FocusedPanel::TaskList);
        assert_eq!(state.task_list_state.selected_index, Some(0));
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_on_focus_changed_task_list_empty_does_not_select() {
        let mut state = TuiState::new();

        // No tasks added

        // Focus task list - should not select anything
        state.on_focus_changed(FocusedPanel::TaskList);
        assert!(state.task_list_state.selected_index.is_none());
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_on_focus_changed_task_list_keeps_existing_selection() {
        let mut state = TuiState::new();

        // Add tasks
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "First task".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-2".to_string(),
            name: "Task 2".to_string(),
            description: "Second task".to_string(),
            dependencies: vec![],
            depth: 0,
        });

        // Manually select second task
        state.task_list_state.selected_index = Some(1);

        // Focus task list - should keep existing selection
        state.on_focus_changed(FocusedPanel::TaskList);
        assert_eq!(state.task_list_state.selected_index, Some(1));
    }

    #[test]
    fn test_on_focus_changed_agent_tree_clears_task_selection() {
        let mut state = TuiState::new();

        // Add and select a task
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "Test task".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.task_list_state.selected_index = Some(0);

        // Focus agent tree - should clear task selection
        state.on_focus_changed(FocusedPanel::AgentTree);
        assert!(state.task_list_state.selected_index.is_none());
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
    }

    #[test]
    fn test_on_focus_changed_app_logs_clears_task_selection() {
        let mut state = TuiState::new();

        // Add and select a task
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "Test task".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.task_list_state.selected_index = Some(0);

        // Focus app logs - should clear task selection
        state.on_focus_changed(FocusedPanel::AppLogs);
        assert!(state.task_list_state.selected_index.is_none());
        assert_eq!(state.current_focus, FocusedPanel::AppLogs);
    }

    #[test]
    fn test_on_focus_changed_agent_output_preserves_task_selection() {
        let mut state = TuiState::new();

        // Add and select a task
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "Test task".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.task_list_state.selected_index = Some(0);

        // Focus agent output - should preserve task selection
        state.on_focus_changed(FocusedPanel::AgentOutput);
        assert_eq!(state.task_list_state.selected_index, Some(0));
        assert_eq!(state.current_focus, FocusedPanel::AgentOutput);
    }

    #[test]
    fn test_on_focus_changed_agent_output_no_task_stays_none() {
        let mut state = TuiState::new();

        // No tasks, no selection
        assert!(state.task_list_state.selected_index.is_none());

        // Focus agent output - should stay None
        state.on_focus_changed(FocusedPanel::AgentOutput);
        assert!(state.task_list_state.selected_index.is_none());
    }
}
