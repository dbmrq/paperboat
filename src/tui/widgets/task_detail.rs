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
use ratatui::widgets::{Block, Borders, Paragraph};

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

/// Renders the task detail panel.
///
/// This function displays detailed information about a selected task,
/// including ID, name, status, depth, description, and dependencies.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The rectangular area to render into
/// * `task` - The task to display details for
/// * `focused` - Whether this panel currently has keyboard focus
pub fn render_task_detail(frame: &mut Frame, area: Rect, task: &TaskDisplay, focused: bool) {
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

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Build the detail content
    let mut lines = vec![];

    // Task ID
    lines.push(Line::from(vec![
        Span::styled("ID: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&task.task_id, Style::default().fg(Color::White)),
    ]));

    // Task Name
    lines.push(Line::from(vec![
        Span::styled("Name: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&task.name, Style::default().fg(Color::Cyan).bold()),
    ]));

    // Status with color coding
    let status_clr = status_color(&task.status);
    let symbol = status_symbol(&task.status);
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
        Span::styled(symbol, Style::default().fg(status_clr)),
        Span::raw(" "),
        Span::styled(&task.status, Style::default().fg(status_clr).bold()),
    ]));

    // Depth
    let depth_str = format!("{}", task.depth);
    let indent_indicator = "  ".repeat(task.depth as usize);
    lines.push(Line::from(vec![
        Span::styled("Depth: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&depth_str, Style::default().fg(Color::Magenta)),
        Span::styled(
            format!(" ({indent_indicator}└─ nested)"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Empty line separator
    lines.push(Line::from(""));

    // Description header
    lines.push(Line::from(Span::styled(
        "Description:",
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
        "Dependencies:",
        Style::default().fg(Color::DarkGray).underlined(),
    )));

    if task.dependencies.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (None)",
            Style::default().fg(Color::DarkGray).italic(),
        )));
    } else {
        for dep in &task.dependencies {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Yellow)),
                Span::styled(dep, Style::default().fg(Color::White)),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

