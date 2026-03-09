//! Mouse event handling.
//!
//! This module handles mouse clicks and scroll events and routes them
//! to the appropriate panels based on cursor position.

use crossterm::event::MouseEvent as CrosstermMouseEvent;
use ratatui::layout::Rect;

use crate::tui::layout::PanelLayout;
use crate::tui::state::{FocusedPanel, TuiState};
use crate::tui::widgets::calculate_wrapped_line_count;

// ============================================================================
// Mouse Click Handling
// ============================================================================

/// Handles a mouse click event to switch panel focus.
///
/// Maps the click coordinates to panel areas and updates the current focus
/// if the click occurred within a valid panel area.
///
/// # Arguments
///
/// * `state` - Mutable reference to the TUI state
/// * `mouse_event` - The crossterm mouse event containing click coordinates
/// * `layout` - The current panel layout with rectangle areas
///
/// # Panel Areas
///
/// Clicks are mapped to the following panels:
/// - Agent Tree (left side, top section)
/// - Agent Output (center, top section)
/// - Task List (right side, top section)
/// - App Logs (full width, middle section)
///
/// Clicks on the status bar are ignored.
pub fn handle_mouse_click(
    state: &mut TuiState,
    mouse_event: CrosstermMouseEvent,
    layout: &PanelLayout,
) {
    // Don't process mouse clicks if help overlay is visible
    if state.help_visible {
        return;
    }

    let column = mouse_event.column;
    let row = mouse_event.row;

    // Check which panel was clicked and update focus accordingly
    if let Some(panel) = panel_at_position(column, row, layout) {
        // Check if task detail is showing in center panel
        let task_detail_visible = state.current_focus == FocusedPanel::TaskList
            && state.task_list_state.get_selected_task().is_some();

        // Handle panel-specific click behavior
        match panel {
            FocusedPanel::AgentTree => {
                handle_agent_tree_click(state, row, layout);
            }
            FocusedPanel::TaskList => {
                handle_task_list_click(state, row, layout);
            }
            FocusedPanel::AgentOutput if task_detail_visible => {
                // When clicking center panel while task detail is shown,
                // don't change focus - keep task detail visible
                return;
            }
            _ => {}
        }
        state.on_focus_changed(panel);
    }
}

/// Handles a mouse click inside the task list panel.
///
/// Calculates which task row was clicked based on the mouse row position,
/// accounting for border offset and scroll offset. If a valid task row is
/// clicked, selects that task.
///
/// # Arguments
///
/// * `state` - Mutable reference to the TUI state
/// * `row` - The row position of the mouse click
/// * `layout` - The current panel layout with rectangle areas
#[allow(clippy::missing_const_for_fn)]
fn handle_task_list_click(state: &mut TuiState, row: u16, layout: &PanelLayout) {
    // Account for top border (inner area starts 1 row below panel top)
    let inner_y = layout.task_list.y + 1;
    // Inner height excludes top and bottom borders
    let inner_height = layout.task_list.height.saturating_sub(2);

    // Check if click is inside the inner content area (not on borders)
    if row >= inner_y && row < inner_y + inner_height {
        // Calculate which visible row was clicked
        let visible_row = row - inner_y;
        // Calculate actual task index accounting for scroll offset
        let clicked_index = visible_row as usize + state.task_list_state.scroll_offset;
        // select_index has built-in bounds checking
        state.task_list_state.select_index(clicked_index);
    }
}

/// Determines which panel (if any) is at the given screen position.
///
/// Returns `None` if the position is in the status bar or outside all panels.
pub(super) const fn panel_at_position(
    column: u16,
    row: u16,
    layout: &PanelLayout,
) -> Option<FocusedPanel> {
    // Check each panel area - order doesn't matter as they don't overlap
    if rect_contains(layout.agent_tree, column, row) {
        Some(FocusedPanel::AgentTree)
    } else if rect_contains(layout.agent_output, column, row) {
        Some(FocusedPanel::AgentOutput)
    } else if rect_contains(layout.task_list, column, row) {
        Some(FocusedPanel::TaskList)
    } else if rect_contains(layout.app_logs, column, row) {
        Some(FocusedPanel::AppLogs)
    } else {
        // Click was on status bar or outside valid panel areas
        None
    }
}

/// Checks if a point (column, row) is within a rectangle.
#[inline]
pub(super) const fn rect_contains(rect: ratatui::layout::Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// Handles a mouse click within the agent tree panel to select an agent.
///
/// Maps the click row position to a visible tree item and selects it.
/// This accounts for:
/// - Border offset (inner area starts at `y + 1`)
/// - Scroll offset from the tree state
/// - Collapsed/expanded node visibility
///
/// # Arguments
///
/// * `state` - Mutable reference to the TUI state
/// * `row` - The screen row where the click occurred
/// * `layout` - The current panel layout with rectangle areas
fn handle_agent_tree_click(state: &mut TuiState, row: u16, layout: &PanelLayout) {
    // Calculate inner area (excluding borders)
    let inner_y = layout.agent_tree.y + 1;
    let inner_height = layout.agent_tree.height.saturating_sub(2);

    // Check if click is within the inner (non-border) area
    if row >= inner_y && row < inner_y + inner_height {
        let visible_row = (row - inner_y) as usize;

        // Get visible items (respects collapsed/expanded state)
        let visible_items = state.agent_tree_state.visible_items();

        // Empty tree - nothing to select
        if visible_items.is_empty() {
            return;
        }

        // Account for scroll offset in tree_state
        let scroll_offset = state.agent_tree_state.tree_state.get_offset();
        let clicked_index = visible_row + scroll_offset;

        // Select the agent if clicked index is valid
        if let Some(session_id) = visible_items.get(clicked_index) {
            state.agent_tree_state.select(session_id);
            state.selected_agent_id = Some(session_id.clone());
            // Disable auto-follow on manual selection
            state.auto_follow_enabled = false;
        }
    }
}

// ============================================================================
// Mouse Scroll Handling
// ============================================================================

/// Number of lines to scroll per mouse wheel notch.
const MOUSE_SCROLL_LINES: u16 = 3;

/// Scroll direction from mouse wheel.
#[derive(Debug, Clone, Copy)]
pub enum ScrollDirection {
    /// Scroll up (wheel scrolled up)
    Up,
    /// Scroll down (wheel scrolled down)
    Down,
}

/// Handles a mouse scroll event to scroll the panel under the cursor.
///
/// Maps the scroll coordinates to panel areas and scrolls the appropriate
/// panel content. Scrolling behavior depends on the panel type:
///
/// - Agent Tree: Moves tree selection up/down
/// - Agent Output: Scrolls output content up/down
/// - Task List: Moves task selection up/down (or scrolls task detail if shown)
/// - App Logs: Scrolls log content up/down
pub fn handle_mouse_scroll(
    state: &mut TuiState,
    mouse_event: CrosstermMouseEvent,
    layout: &PanelLayout,
    direction: ScrollDirection,
) {
    // Don't process mouse scrolls if help or settings overlay is visible
    if state.help_visible || state.settings_visible {
        return;
    }

    let column = mouse_event.column;
    let row = mouse_event.row;

    // Determine which panel the mouse is over and scroll accordingly
    if let Some(panel) = panel_at_position(column, row, layout) {
        // Check if task detail is showing in center panel
        let task_detail_visible = state.current_focus == FocusedPanel::TaskList
            && state.task_list_state.get_selected_task().is_some();

        match panel {
            FocusedPanel::AgentTree => {
                scroll_agent_tree(state, direction);
            }
            FocusedPanel::AgentOutput => {
                // If task detail is showing in center panel, scroll that instead
                if task_detail_visible {
                    scroll_task_detail(state, direction, layout.agent_output.height);
                } else {
                    scroll_agent_output(state, direction, &layout.agent_output);
                }
            }
            FocusedPanel::TaskList => {
                // If task detail is showing (task selected), scroll the detail
                // Otherwise, scroll the task list selection
                if state.task_list_state.get_selected_task().is_some() {
                    scroll_task_detail(state, direction, layout.agent_output.height);
                } else {
                    scroll_task_list(state, direction);
                }
            }
            FocusedPanel::AppLogs => {
                scroll_app_logs(state, direction);
            }
        }
    }
}

/// Scrolls the agent tree selection up or down.
fn scroll_agent_tree(state: &mut TuiState, direction: ScrollDirection) {
    for _ in 0..MOUSE_SCROLL_LINES {
        match direction {
            ScrollDirection::Up => {
                state.agent_tree_state.tree_state.key_up();
            }
            ScrollDirection::Down => {
                state.agent_tree_state.tree_state.key_down();
            }
        }
    }
    // Update selected agent to match tree selection
    state.selected_agent_id = state
        .agent_tree_state
        .selected_session_id()
        .map(String::from);
}

/// Scrolls the agent output panel up or down.
fn scroll_agent_output(state: &mut TuiState, direction: ScrollDirection, area: &Rect) {
    // Calculate total lines from selected agent messages, accounting for text wrapping.
    // Subtract 2 from width to account for left/right borders.
    let inner_width = area.width.saturating_sub(2);
    let total_lines = state.selected_agent_messages().map_or(0, |messages| {
        calculate_wrapped_line_count(messages, inner_width)
    });

    let page_size = area.height.saturating_sub(2) as usize; // Account for borders

    match direction {
        ScrollDirection::Up => {
            state.agent_output_scroll =
                state.agent_output_scroll.saturating_sub(MOUSE_SCROLL_LINES);
        }
        ScrollDirection::Down => {
            // Truncation safe: scroll positions are UI coordinates, well under u16::MAX
            #[allow(clippy::cast_possible_truncation)]
            let max_scroll = total_lines.saturating_sub(page_size) as u16;
            state.agent_output_scroll =
                (state.agent_output_scroll + MOUSE_SCROLL_LINES).min(max_scroll);
        }
    }
}

/// Scrolls the task list selection up or down.
fn scroll_task_list(state: &mut TuiState, direction: ScrollDirection) {
    for _ in 0..MOUSE_SCROLL_LINES {
        match direction {
            ScrollDirection::Up => {
                state.task_list_state.select_previous();
            }
            ScrollDirection::Down => {
                state.task_list_state.select_next();
            }
        }
    }
}

/// Scrolls the task detail panel up or down.
fn scroll_task_detail(state: &mut TuiState, direction: ScrollDirection, visible_height: u16) {
    // Get total lines from task detail (approximation - use a conservative estimate)
    // The actual line count depends on description length, but for scrolling this is sufficient
    let total_lines = state.task_list_state.get_selected_task().map_or(0, |task| {
        // Basic line count: ID + Name + Status + Depth + empty + Description header +
        // description lines + empty + Dependencies header + dependency lines
        6 + task.description.lines().count() + 2 + task.dependencies.len().max(1)
    });

    let page_size = visible_height.saturating_sub(2) as usize; // Account for borders

    match direction {
        ScrollDirection::Up => {
            state.task_detail_scroll = state.task_detail_scroll.saturating_sub(MOUSE_SCROLL_LINES);
        }
        ScrollDirection::Down => {
            // Truncation safe: scroll positions are UI coordinates, well under u16::MAX
            #[allow(clippy::cast_possible_truncation)]
            let max_scroll = total_lines.saturating_sub(page_size) as u16;
            state.task_detail_scroll =
                (state.task_detail_scroll + MOUSE_SCROLL_LINES).min(max_scroll);
        }
    }
}

/// Scrolls the app logs panel up or down.
fn scroll_app_logs(state: &mut TuiState, direction: ScrollDirection) {
    use tui_logger::TuiWidgetEvent;

    // For tui-logger, we use its built-in transition events
    // Scroll multiple times to match the scroll lines count
    for _ in 0..MOUSE_SCROLL_LINES {
        match direction {
            ScrollDirection::Up => {
                state.logger_state.transition(TuiWidgetEvent::UpKey);
            }
            ScrollDirection::Down => {
                state.logger_state.transition(TuiWidgetEvent::DownKey);
            }
        }
    }
}
