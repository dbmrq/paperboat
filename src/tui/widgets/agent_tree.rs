//! Agent tree widget module.
//!
//! This module contains the [`render_agent_tree`] function which renders
//! an interactive agent hierarchy tree with expand/collapse support.
//!
//! # Display Format
//!
//! ```text
//! ▼ Orch [00:05:32]
//!   ├ Planner ✓
//!  >├ impl-001 ~
//!   ├ impl-002
//!   └ ▼ Orch [sub]
//!       ├ impl-001
//!       └ impl-002
//! ```

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_tree_widget::Tree;

use super::super::state::{FocusedPanel, TuiState};

/// Status icons for agent states.
#[allow(dead_code)] // Reserved for future use when rendering agents with status icons
mod icons {
    pub const COMPLETED: &str = "✓";
    pub const IN_PROGRESS: &str = "~";
    pub const FAILED: &str = "✗";
}

/// Colors used for the agent tree widget.
#[allow(dead_code)] // Reserved for future use when adding custom agent coloring
mod colors {
    use ratatui::style::Color;

    pub const FOCUSED_BORDER: Color = Color::Cyan;
    pub const UNFOCUSED_BORDER: Color = Color::Gray;
    pub const HIGHLIGHT: Color = Color::Yellow;
    pub const COMPLETED: Color = Color::Green;
    pub const RUNNING: Color = Color::Blue;
    pub const FAILED: Color = Color::Red;
}

/// Renders the agent tree widget in the given area.
///
/// This function renders an interactive tree showing the agent hierarchy.
/// It uses `tui-tree-widget` for the tree rendering and supports:
///
/// - Expand/collapse of nodes with children
/// - Visual indication of selected agent
/// - Status icons for agent states
/// - Focus-dependent border styling
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render to
/// * `area` - The rectangular area to render the widget in
/// * `state` - The TUI state containing agent tree data
/// * `focused` - Whether this panel currently has keyboard focus
///
/// # Example
///
/// ```ignore
/// use ratatui::Frame;
/// use ratatui::layout::Rect;
/// use paperboat::tui::widgets::render_agent_tree;
/// use paperboat::tui::TuiState;
///
/// fn render(frame: &mut Frame, area: Rect, state: &TuiState) {
///     let focused = state.current_focus == FocusedPanel::AgentTree;
///     render_agent_tree(frame, area, state, focused);
/// }
/// ```
pub fn render_agent_tree(frame: &mut Frame, area: Rect, state: &mut TuiState, focused: bool) {
    // Build tree items from the agent tree state
    let items = state.agent_tree_state.build_tree_items();

    // Create the border block with focus-dependent styling
    let border_color = if focused {
        colors::FOCUSED_BORDER
    } else {
        colors::UNFOCUSED_BORDER
    };

    let block = Block::default()
        .title(" Agents ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    // Handle empty tree case
    if items.is_empty() {
        let empty_tree = Tree::new(&[] as &[tui_tree_widget::TreeItem<'_, String>])
            .expect("empty tree should be valid")
            .block(block)
            .highlight_style(Style::default().fg(colors::HIGHLIGHT).bold())
            .highlight_symbol("> ");

        frame.render_stateful_widget(empty_tree, area, &mut state.agent_tree_state.tree_state);
        return;
    }

    // Create the tree widget
    let tree_widget = Tree::new(&items)
        .expect("all item identifiers should be unique")
        .block(block)
        .highlight_style(Style::default().fg(colors::HIGHLIGHT).bold())
        .highlight_symbol("> ")
        .node_closed_symbol("▶ ")
        .node_open_symbol("▼ ")
        .node_no_children_symbol("  ");

    // Render the tree with the mutable tree state for selection/navigation
    frame.render_stateful_widget(tree_widget, area, &mut state.agent_tree_state.tree_state);
}

/// Handles keyboard navigation for the agent tree.
///
/// This function processes keyboard inputs for tree navigation:
/// - Up/Down arrows: Move selection
/// - Left/Right arrows: Collapse/Expand nodes
/// - Enter: Select current agent
///
/// # Arguments
///
/// * `state` - Mutable reference to TUI state
/// * `key` - The key code pressed
///
/// # Returns
///
/// `true` if the key was handled, `false` otherwise.
pub fn handle_agent_tree_key(state: &mut TuiState, key: crossterm::event::KeyCode) -> bool {
    use crossterm::event::KeyCode;

    match key {
        KeyCode::Up => {
            state.agent_tree_state.tree_state.key_up();
            update_selected_agent(state);
            true
        }
        KeyCode::Down => {
            state.agent_tree_state.tree_state.key_down();
            update_selected_agent(state);
            true
        }
        KeyCode::Left => {
            state.agent_tree_state.tree_state.key_left();
            true
        }
        KeyCode::Right => {
            state.agent_tree_state.tree_state.key_right();
            true
        }
        KeyCode::Enter => {
            // Toggle open/close on enter, and update selection
            state.agent_tree_state.tree_state.toggle_selected();
            update_selected_agent(state);
            true
        }
        _ => false,
    }
}

/// Updates the selected agent ID in the TUI state based on tree selection.
fn update_selected_agent(state: &mut TuiState) {
    state.selected_agent_id = state
        .agent_tree_state
        .selected_session_id()
        .map(String::from);
}

/// Checks if the agent tree panel is currently focused.
#[allow(dead_code)]
fn is_agent_tree_focused(state: &TuiState) -> bool {
    state.current_focus == FocusedPanel::AgentTree
}
