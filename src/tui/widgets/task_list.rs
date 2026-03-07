//! Task list widget module.
//!
//! This module contains the [`render_task_list`] function which displays
//! the hierarchical task tree with status indicators.

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph};

use super::super::state::TuiState;

/// Returns the status indicator for a task status string.
///
/// Status indicators:
/// - `[ ]` - `NotStarted`/pending
/// - `[/]` - `InProgress`
/// - `[✓]` - Complete (success)
/// - `[✗]` - Failed
/// - `[-]` - Skipped
fn status_indicator(status: &str) -> &'static str {
    match status {
        "pending" => "[ ]",
        "in_progress" => "[/]",
        "completed" => "[✓]",
        "failed" => "[✗]",
        "skipped" => "[-]",
        _ => "[?]",
    }
}

/// Returns the color for a task status string.
fn status_color(status: &str) -> Color {
    match status {
        "pending" => Color::Gray,
        "in_progress" => Color::Yellow,
        "completed" => Color::Green,
        "failed" => Color::Red,
        "skipped" => Color::DarkGray,
        _ => Color::White,
    }
}

/// Renders the task list panel.
///
/// This function renders the task list using ratatui's List widget with:
/// - Status indicators showing task state
/// - Color-coded items based on status
/// - Selection highlighting
/// - Scrolling support for long lists
/// - Focus styling when the panel is focused
///
/// # Arguments
///
/// * `frame` - The ratatui Frame to render to
/// * `area` - The rectangular area to render within
/// * `state` - The TUI state containing task list data
/// * `focused` - Whether this panel currently has focus
pub fn render_task_list(frame: &mut Frame, area: Rect, state: &TuiState, focused: bool) {
    // Determine border style based on focus
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Tasks ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);

    // Handle empty task list case
    if state.task_list_state.is_empty() {
        let empty_message = Paragraph::new("No tasks yet")
            .style(Style::default().fg(Color::DarkGray).italic())
            .alignment(Alignment::Center)
            .block(block);

        frame.render_widget(empty_message, area);
        return;
    }

    // Build list items from tasks with depth-based indentation
    let tasks = state.task_list_state.tasks();
    let items: Vec<ListItem> = tasks
        .iter()
        .map(|task| {
            let indicator = status_indicator(&task.status);
            let color = status_color(&task.status);

            // Create indentation based on task depth (2 spaces per level)
            let indent = "  ".repeat(task.depth as usize);

            // Format: [indent][status] task_name
            let content = format!("{}{} {}", indent, indicator, task.name);

            ListItem::new(content).style(Style::default().fg(color))
        })
        .collect();

    // Create the list widget
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().fg(Color::Yellow).bold())
        .highlight_symbol("> ");

    // Create list state for selection and scrolling
    let mut list_state = ListState::default();
    list_state.select(state.task_list_state.selected_index);

    // Apply scroll offset if needed
    // ListState handles scrolling automatically based on selection,
    // but we also support manual scroll_offset from TaskListState
    if state.task_list_state.scroll_offset > 0 && state.task_list_state.selected_index.is_none() {
        // If there's a scroll offset but no selection, we need to adjust
        // by selecting the item at the scroll offset position
        let offset = state.task_list_state.scroll_offset;
        if offset < tasks.len() {
            *list_state.offset_mut() = offset;
        }
    }

    // Render the list with state for selection highlighting
    frame.render_stateful_widget(list, area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_indicator_displays_correct_symbols() {
        // Verify all status strings map to correct indicators
        assert_eq!(status_indicator("pending"), "[ ]");
        assert_eq!(status_indicator("in_progress"), "[/]");
        assert_eq!(status_indicator("completed"), "[✓]");
        assert_eq!(status_indicator("failed"), "[✗]");
        assert_eq!(status_indicator("skipped"), "[-]");

        // Unknown status shows question mark
        assert_eq!(status_indicator("unknown"), "[?]");
        assert_eq!(status_indicator(""), "[?]");
    }

    #[test]
    fn test_status_color_returns_correct_colors() {
        // Verify all status strings map to correct colors
        assert_eq!(status_color("pending"), Color::Gray);
        assert_eq!(status_color("in_progress"), Color::Yellow);
        assert_eq!(status_color("completed"), Color::Green);
        assert_eq!(status_color("failed"), Color::Red);
        assert_eq!(status_color("skipped"), Color::DarkGray);

        // Unknown status defaults to white
        assert_eq!(status_color("unknown"), Color::White);
        assert_eq!(status_color(""), Color::White);
    }
}
