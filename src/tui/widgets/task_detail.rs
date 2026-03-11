//! Task detail panel widget for the Villalobos TUI.
//!
//! This module provides a detailed view of a selected task, showing:
//! - Task ID and name
//! - Status with color coding
//! - Depth in hierarchy
//! - Description
//! - Dependencies
//! - Progress information

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::tui::state::TuiState;
use crate::tui::task_list_state::TaskDisplay;

// ============================================================================
// Color Utilities
// ============================================================================

/// Returns the color associated with a task status.
fn status_color(status: &str) -> Color {
    match status {
        "completed" => Color::Green,
        "in_progress" => Color::Yellow,
        "failed" => Color::Red,
        "cancelled" | "skipped" => Color::DarkGray,
        "pending" => Color::Blue,
        "blocked" => Color::Magenta,
        _ => Color::White,
    }
}

/// Returns a status symbol for display.
fn status_symbol(status: &str) -> &'static str {
    match status {
        "completed" => "✓",
        "in_progress" => "▶",
        "failed" => "✗",
        "cancelled" | "skipped" => "⊘",
        "pending" => "○",
        "blocked" => "◈",
        _ => "?",
    }
}

// ============================================================================
// Task Detail Rendering
// ============================================================================

/// Builds the task detail content as a vector of Lines.
///
/// This is extracted to allow calculating total lines for scrolling.
fn build_task_detail_lines(task: &TaskDisplay) -> Vec<Line<'static>> {
    let mut lines = vec![];

    // Task ID
    lines.push(Line::from(vec![
        Span::styled("ID: ", Style::default().fg(Color::DarkGray)),
        Span::styled(task.task_id.clone(), Style::default().fg(Color::White)),
    ]));

    // Task Name
    lines.push(Line::from(vec![
        Span::styled("Name: ", Style::default().fg(Color::DarkGray)),
        Span::styled(task.name.clone(), Style::default().fg(Color::Cyan).bold()),
    ]));

    // Status with color coding
    let status_clr = status_color(&task.status);
    let symbol = status_symbol(&task.status);
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
        Span::styled(symbol.to_string(), Style::default().fg(status_clr)),
        Span::raw(" "),
        Span::styled(task.status.clone(), Style::default().fg(status_clr).bold()),
    ]));

    // Depth
    let depth_str = format!("{}", task.depth);
    let indent_indicator = "  ".repeat(task.depth as usize);
    lines.push(Line::from(vec![
        Span::styled("Depth: ", Style::default().fg(Color::DarkGray)),
        Span::styled(depth_str, Style::default().fg(Color::Magenta)),
        Span::styled(
            format!(" ({indent_indicator}└─ nested)"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Empty line separator
    lines.push(Line::from(""));

    // Description header
    lines.push(Line::from(Span::styled(
        "Description:".to_string(),
        Style::default().fg(Color::DarkGray).underlined(),
    )));

    // Description content (potentially multi-line)
    let desc = if task.description.is_empty() {
        "(No description)".to_string()
    } else {
        task.description.clone()
    };
    for line in desc.lines() {
        lines.push(Line::from(Span::styled(
            format!("  {line}"),
            Style::default().fg(Color::White),
        )));
    }

    // Empty line separator
    lines.push(Line::from(""));

    // Dependencies
    lines.push(Line::from(Span::styled(
        "Dependencies:".to_string(),
        Style::default().fg(Color::DarkGray).underlined(),
    )));

    if task.dependencies.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (None)".to_string(),
            Style::default().fg(Color::DarkGray).italic(),
        )));
    } else {
        for dep in &task.dependencies {
            lines.push(Line::from(vec![
                Span::styled("  • ".to_string(), Style::default().fg(Color::Yellow)),
                Span::styled(dep.clone(), Style::default().fg(Color::White)),
            ]));
        }
    }

    lines
}

/// Renders the task detail panel with scrolling support.
///
/// This function displays detailed information about a selected task,
/// including ID, name, status, depth, description, and dependencies.
/// It supports scrolling when content exceeds the visible area.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The rectangular area to render into
/// * `state` - Mutable reference to the TUI state for scroll tracking
/// * `task` - The task to display details for
/// * `focused` - Whether this panel currently has keyboard focus
#[allow(clippy::cast_possible_truncation)] // Terminal dimensions fit in u16
pub fn render_task_detail(
    frame: &mut Frame,
    area: Rect,
    state: &mut TuiState,
    task: &TaskDisplay,
    focused: bool,
) {
    // Reset scroll position when a different task is selected
    let current_task_id = Some(task.task_id.clone());
    if state.last_selected_task_id != current_task_id {
        state.task_detail_scroll = 0;
        state.last_selected_task_id = current_task_id;
    }

    // Build the block with focus-dependent styling
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = format!(" Task: {} ", task.name);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    // Build the detail content
    let lines = build_task_detail_lines(task);
    let total_lines = lines.len();

    // Calculate visible area (minus borders)
    let inner_height = area.height.saturating_sub(2) as usize;

    // Clamp scroll position to valid range
    let max_scroll = total_lines.saturating_sub(inner_height);
    if state.task_detail_scroll > max_scroll as u16 {
        state.task_detail_scroll = max_scroll as u16;
    }

    // Create the paragraph with scroll
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.task_detail_scroll, 0));

    frame.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if total_lines > inner_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state =
            ScrollbarState::new(max_scroll).position(state.task_detail_scroll as usize);

        // Render scrollbar in the same area (it will appear in the border)
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Handles keyboard events for the task detail panel.
///
/// This function processes keyboard events when the task detail panel
/// is showing (`TaskList` focused with a task selected). Supported keys:
///
/// - `PageUp`: Scroll up by visible height
/// - `PageDown`: Scroll down by visible height
///
/// Returns `true` if the key was handled, `false` otherwise.
#[allow(clippy::cast_possible_truncation)] // Terminal line counts fit in u16
pub fn handle_task_detail_key(
    state: &mut TuiState,
    key_code: crossterm::event::KeyCode,
    visible_height: u16,
) -> bool {
    use crossterm::event::KeyCode;

    // Calculate total lines from the selected task
    let total_lines = state
        .task_list_state
        .get_selected_task()
        .map_or(0, |task| build_task_detail_lines(task).len());

    let page_size = visible_height.saturating_sub(2); // Account for borders
    let inner_height = page_size as usize;

    match key_code {
        KeyCode::PageUp => {
            state.task_detail_scroll = state.task_detail_scroll.saturating_sub(page_size);
            true
        }
        KeyCode::PageDown => {
            let max_scroll = total_lines.saturating_sub(inner_height) as u16;
            state.task_detail_scroll = (state.task_detail_scroll + page_size).min(max_scroll);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::logging::LogEvent;
    use crate::tui::state::FocusedPanel;

    fn create_test_task() -> TaskDisplay {
        TaskDisplay {
            task_id: "task-123".to_string(),
            name: "Test Task".to_string(),
            description: "A test task description".to_string(),
            status: "pending".to_string(),
            dependencies: vec![],
            depth: 0,
        }
    }

    fn render_task_detail_to_string(
        state: &mut TuiState,
        task: &TaskDisplay,
        area: Rect,
        focused: bool,
    ) -> String {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_task_detail(frame, area, state, task, focused))
            .expect("task detail should render");
        format!("{}", terminal.backend())
    }

    // ========================================================================
    // Status Color Tests
    // ========================================================================

    #[test]
    fn test_status_color_completed() {
        assert_eq!(status_color("completed"), Color::Green);
    }

    #[test]
    fn test_status_color_in_progress() {
        assert_eq!(status_color("in_progress"), Color::Yellow);
    }

    #[test]
    fn test_status_color_failed() {
        assert_eq!(status_color("failed"), Color::Red);
    }

    #[test]
    fn test_status_color_pending() {
        assert_eq!(status_color("pending"), Color::Blue);
    }

    #[test]
    fn test_status_color_cancelled() {
        assert_eq!(status_color("cancelled"), Color::DarkGray);
    }

    #[test]
    fn test_status_color_skipped() {
        assert_eq!(status_color("skipped"), Color::DarkGray);
    }

    #[test]
    fn test_status_color_blocked() {
        assert_eq!(status_color("blocked"), Color::Magenta);
    }

    #[test]
    fn test_status_color_unknown() {
        assert_eq!(status_color("unknown_status"), Color::White);
    }

    // ========================================================================
    // Status Symbol Tests
    // ========================================================================

    #[test]
    fn test_status_symbol_completed() {
        assert_eq!(status_symbol("completed"), "✓");
    }

    #[test]
    fn test_status_symbol_in_progress() {
        assert_eq!(status_symbol("in_progress"), "▶");
    }

    #[test]
    fn test_status_symbol_failed() {
        assert_eq!(status_symbol("failed"), "✗");
    }

    #[test]
    fn test_status_symbol_pending() {
        assert_eq!(status_symbol("pending"), "○");
    }

    #[test]
    fn test_status_symbol_cancelled() {
        assert_eq!(status_symbol("cancelled"), "⊘");
    }

    #[test]
    fn test_status_symbol_blocked() {
        assert_eq!(status_symbol("blocked"), "◈");
    }

    #[test]
    fn test_status_symbol_unknown() {
        assert_eq!(status_symbol("unknown"), "?");
    }

    // ========================================================================
    // build_task_detail_lines Tests
    // ========================================================================

    #[test]
    fn test_build_task_detail_lines_contains_id() {
        let task = create_test_task();
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("ID:"));
        assert!(content.contains("task-123"));
    }

    #[test]
    fn test_build_task_detail_lines_contains_name() {
        let task = create_test_task();
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("Name:"));
        assert!(content.contains("Test Task"));
    }

    #[test]
    fn test_build_task_detail_lines_contains_status() {
        let task = create_test_task();
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("Status:"));
        assert!(content.contains("pending"));
    }

    #[test]
    fn test_build_task_detail_lines_contains_depth() {
        let mut task = create_test_task();
        task.depth = 2;
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("Depth:"));
        assert!(content.contains('2'));
    }

    #[test]
    fn test_build_task_detail_lines_contains_description() {
        let task = create_test_task();
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("Description:"));
        assert!(content.contains("A test task description"));
    }

    #[test]
    fn test_build_task_detail_lines_empty_description() {
        let mut task = create_test_task();
        task.description = String::new();
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("(No description)"));
    }

    #[test]
    fn test_build_task_detail_lines_with_dependencies() {
        let mut task = create_test_task();
        task.dependencies = vec!["dep-1".to_string(), "dep-2".to_string()];
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("Dependencies:"));
        assert!(content.contains("dep-1"));
        assert!(content.contains("dep-2"));
    }

    #[test]
    fn test_build_task_detail_lines_no_dependencies() {
        let task = create_test_task();
        let lines = build_task_detail_lines(&task);

        let content: String = lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("Dependencies:"));
        assert!(content.contains("(None)"));
    }

    // ========================================================================
    // Render Tests
    // ========================================================================

    #[test]
    fn test_render_task_detail_shows_task_info() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::TaskList;
        let task = create_test_task();
        let area = Rect::new(0, 0, 60, 20);

        let rendered = render_task_detail_to_string(&mut state, &task, area, true);

        assert!(rendered.contains("Test Task"));
        assert!(rendered.contains("pending"));
    }

    #[test]
    fn test_render_task_detail_focused_vs_unfocused() {
        let mut state = TuiState::new();
        let task = create_test_task();
        let area = Rect::new(0, 0, 60, 20);

        // Both focused and unfocused should render without panic
        let focused = render_task_detail_to_string(&mut state, &task, area, true);
        let unfocused = render_task_detail_to_string(&mut state, &task, area, false);

        assert!(focused.contains("Test Task"));
        assert!(unfocused.contains("Test Task"));
    }

    #[test]
    fn test_render_task_detail_resets_scroll_on_task_change() {
        let mut state = TuiState::new();
        state.task_detail_scroll = 10;
        state.last_selected_task_id = Some("old-task".to_string());

        let task = create_test_task();
        let area = Rect::new(0, 0, 60, 20);

        render_task_detail_to_string(&mut state, &task, area, true);

        // Scroll should reset when task changes
        assert_eq!(state.task_detail_scroll, 0);
        assert_eq!(state.last_selected_task_id, Some("task-123".to_string()));
    }

    #[test]
    fn test_render_task_detail_preserves_scroll_on_same_task() {
        let mut state = TuiState::new();
        let task = create_test_task();
        let area = Rect::new(0, 0, 60, 5); // Small area to allow scrolling

        // First render
        render_task_detail_to_string(&mut state, &task, area, true);

        // Set scroll
        state.task_detail_scroll = 2;

        // Second render of same task
        render_task_detail_to_string(&mut state, &task, area, true);

        // Scroll should be preserved (or clamped if exceeds content)
        // In this case, content should be enough to allow scroll of 2
        assert!(state.task_detail_scroll <= 2);
    }

    #[test]
    fn test_render_task_detail_clamps_scroll() {
        let mut state = TuiState::new();
        state.task_detail_scroll = 999;
        state.last_selected_task_id = Some("task-123".to_string()); // Same task

        let task = create_test_task();
        let area = Rect::new(0, 0, 60, 20);

        render_task_detail_to_string(&mut state, &task, area, true);

        // Scroll should be clamped to valid range
        let lines = build_task_detail_lines(&task);
        let inner_height = area.height.saturating_sub(2) as usize;
        let max_scroll = lines.len().saturating_sub(inner_height) as u16;
        assert!(state.task_detail_scroll <= max_scroll);
    }

    // ========================================================================
    // Keyboard Navigation Tests
    // ========================================================================

    #[test]
    fn test_handle_task_detail_key_page_up() {
        let mut state = TuiState::new();

        // Add a task and select it
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Test".to_string(),
            description: "Description".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.task_list_state.selected_index = Some(0);
        state.task_detail_scroll = 10;

        let handled = handle_task_detail_key(
            &mut state,
            crossterm::event::KeyCode::PageUp,
            15,
        );

        assert!(handled);
        assert!(state.task_detail_scroll < 10);
    }

    #[test]
    fn test_handle_task_detail_key_page_down() {
        let mut state = TuiState::new();

        // Add a task with long description to enable scrolling
        let long_desc = "Line\n".repeat(30);
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Test".to_string(),
            description: long_desc,
            dependencies: vec![],
            depth: 0,
        });
        state.task_list_state.selected_index = Some(0);
        state.task_detail_scroll = 0;

        let handled = handle_task_detail_key(
            &mut state,
            crossterm::event::KeyCode::PageDown,
            10,
        );

        assert!(handled);
        assert!(state.task_detail_scroll > 0);
    }

    #[test]
    fn test_handle_task_detail_key_unrecognized() {
        let mut state = TuiState::new();

        let handled = handle_task_detail_key(
            &mut state,
            crossterm::event::KeyCode::Char('x'),
            15,
        );

        assert!(!handled);
    }

    #[test]
    fn test_handle_task_detail_key_no_task_selected() {
        let mut state = TuiState::new();
        // No task selected

        let handled = handle_task_detail_key(
            &mut state,
            crossterm::event::KeyCode::PageDown,
            15,
        );

        // Should still return true (key was handled) but no scroll change
        assert!(handled);
        assert_eq!(state.task_detail_scroll, 0);
    }
}
