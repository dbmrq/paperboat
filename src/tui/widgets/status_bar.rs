//! Status bar widget module.
//!
//! This module contains [`render_status_bar`] which displays
//! status information, keybindings, and messages at the bottom
//! of the terminal.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::super::state::TuiState;

/// Renders the status bar at the bottom of the terminal.
///
/// Display format: ` Status: Running │ Agents: ✓{succeeded} ✗{failed} ~{in_progress} ({total} total, {active} active) │ Tasks: 2/4 │ ?=help `
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The rectangular area to render the status bar
/// * `state` - The current TUI state to extract statistics from
pub fn render_status_bar(frame: &mut Frame, area: Rect, state: &TuiState) {
    let (succeeded, failed, in_progress, total, active) = state.get_agent_stats();
    let (completed_tasks, total_tasks) = state.get_task_progress();
    let is_running = state.is_running();

    // Build status text
    let status_text = if is_running { "Running" } else { "Idle" };

    // Build the spans with styling
    // Start with single space padding
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            status_text,
            Style::default().fg(if is_running {
                Color::Green
            } else {
                Color::Yellow
            }),
        ),
        Span::raw(" │ "),
        Span::styled("Agents: ", Style::default().add_modifier(Modifier::BOLD)),
        // Succeeded count (green)
        Span::styled(format!("✓{succeeded}"), Style::default().fg(Color::Green)),
        Span::raw(" "),
        // Failed count (red)
        Span::styled(format!("✗{failed}"), Style::default().fg(Color::Red)),
        Span::raw(" "),
        // In-progress count (yellow)
        Span::styled(format!("~{in_progress}"), Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        // Total and active (cyan)
        Span::styled(
            format!("({total} total, {active} active)"),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(" │ "),
        Span::styled("Tasks: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{completed_tasks}/{total_tasks}"),
            Style::default().fg(if completed_tasks == total_tasks && total_tasks > 0 {
                Color::Green
            } else if completed_tasks > 0 {
                Color::Yellow
            } else {
                Color::White
            }),
        ),
    ];

    // Add status message if present
    if let Some(ref msg) = state.status_message {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Magenta),
        ));
    }

    // Always add help hint at the end
    spans.push(Span::raw(" │ "));
    spans.push(Span::styled(
        "?=help",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    ));

    // End with single space padding
    spans.push(Span::raw(" "));

    let line = Line::from(spans);
    let paragraph =
        Paragraph::new(line).style(Style::default().bg(Color::DarkGray).fg(Color::White));

    frame.render_widget(paragraph, area);
}
