//! Custom widgets for the Villalobos TUI.
//!
//! This module contains custom ratatui widgets for displaying:
//!
//! - [`render_agent_tree`] - Interactive agent hierarchy tree with expand/collapse
//! - [`render_agent_output`] - Real-time agent output with scrollback
//! - [`render_task_list`] - Task tree with status indicators
//! - [`render_task_detail`] - Detailed task view when a task is selected
//! - [`render_status_bar`] - Function to render status bar with live statistics
//! - [`render_help_overlay`] - Modal help overlay with keyboard shortcuts
//! - [`render_app_logs`] - Application logs with tui-logger filtering and controls

mod agent_output;
mod agent_tree;
mod app_logs;
mod help;
mod status_bar;
mod task_detail;
mod task_list;

pub use agent_output::{handle_agent_output_key, render_agent_output};
pub use agent_tree::{handle_agent_tree_key, render_agent_tree};
pub use app_logs::{create_app_logs_state, render_app_logs};
pub use help::render_help_overlay;
pub use status_bar::render_status_bar;
pub use task_detail::render_task_detail;
pub use task_list::render_task_list;
