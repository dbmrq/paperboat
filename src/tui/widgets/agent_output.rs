//! Agent output widget module.
//!
//! This module contains [`render_agent_output`] which displays
//! real-time output from agents, including support for ANSI colors
//! and scrolling.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

use super::super::state::{FocusedPanel, TuiState};

/// Renders the agent output panel.
///
/// This function displays streaming output from the selected agent,
/// with support for scrollback and auto-scroll to bottom when new
/// content arrives.
///
/// # Display Format
///
/// Messages are displayed with tool calls highlighted:
/// ```text
/// > view src/auth.rs
/// Looking at the auth module...
/// I see we need to add JWT...
/// > str-replace-editor auth.rs
/// Adding authenticate() function
/// ```
///
/// # Arguments
#[allow(clippy::cast_possible_truncation)] // Terminal dimensions fit in u16
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The rectangular area to render into
/// * `state` - The TUI state containing agent messages and scroll position
/// * `focused` - Whether this panel currently has keyboard focus
pub fn render_agent_output(frame: &mut Frame, area: Rect, state: &mut TuiState, focused: bool) {
    let is_focused = focused || state.current_focus == FocusedPanel::AgentOutput;

    // Build the block with focus-dependent styling
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if let Some(agent) = state.selected_agent() {
        format!(" {} Output ", agent.display_name())
    } else {
        " Agent Output ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    // Get messages for the selected agent
    let messages = state.selected_agent_messages();

    // Handle empty selection or no messages
    let (content, total_lines) = if state.selected_agent_id.is_none() {
        let text = Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No agent selected",
                Style::default().fg(Color::DarkGray).italic(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Select an agent from the tree on the left",
                Style::default().fg(Color::DarkGray),
            )),
        ]);
        (text, 4)
    } else if messages.is_none() || messages.is_none_or(Vec::is_empty) {
        let text = Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Waiting for output...",
                Style::default().fg(Color::DarkGray).italic(),
            )),
        ]);
        (text, 2)
    } else {
        // Format messages with styling
        let msgs = messages.unwrap();
        let lines = format_messages(msgs);
        let line_count = lines.len();
        (Text::from(lines), line_count)
    };

    // Calculate visible area (minus borders)
    let inner_height = area.height.saturating_sub(2) as usize;

    // Auto-scroll: if new messages arrived, scroll to bottom
    let current_message_count = messages.map_or(0, Vec::len);
    if current_message_count > state.last_message_count {
        // New content arrived, auto-scroll to bottom
        if total_lines > inner_height {
            state.agent_output_scroll = (total_lines - inner_height) as u16;
        } else {
            state.agent_output_scroll = 0;
        }
        state.last_message_count = current_message_count;
    }

    // Clamp scroll position to valid range
    let max_scroll = total_lines.saturating_sub(inner_height);
    if state.agent_output_scroll > max_scroll as u16 {
        state.agent_output_scroll = max_scroll as u16;
    }

    // Create the paragraph with scroll
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((state.agent_output_scroll, 0));

    frame.render_widget(paragraph, area);

    // Render scrollbar if content exceeds visible area
    if total_lines > inner_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        let mut scrollbar_state =
            ScrollbarState::new(max_scroll).position(state.agent_output_scroll as usize);

        // Render scrollbar in the same area (it will appear in the border)
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Formats messages for display with styling.
///
/// Tool calls are highlighted with emoji indicators and different colors.
/// Single line breaks are preserved as blank lines, while consecutive empty
/// messages (from double/multiple line breaks) are collapsed to a single
/// blank line to avoid wasting space.
fn format_messages(messages: &[String]) -> Vec<Line<'static>> {
    let mut result: Vec<Line<'static>> = Vec::new();
    let mut prev_was_empty = false;

    for msg in messages {
        if msg.is_empty() {
            // For empty messages (line breaks), only add if previous wasn't empty
            // This collapses consecutive line breaks into one
            if !prev_was_empty {
                result.push(Line::from(""));
            }
            prev_was_empty = true;
        } else {
            // For non-empty messages, split into lines and format each
            for line in msg.lines() {
                result.push(format_line(line));
            }
            // .lines() strips trailing newlines, so if the message ends with \n,
            // add an empty line to preserve the line break in the display.
            // This ensures single \n displays as a single line break.
            if msg.ends_with('\n') {
                result.push(Line::from(""));
                prev_was_empty = true;
            } else {
                prev_was_empty = false;
            }
        }
    }

    result
}

/// Formats a single line with appropriate styling.
fn format_line(line: &str) -> Line<'static> {
    let line_owned = line.to_string();

    // Tool call indicators (using simple text symbols, not image emoji)
    if line_owned.starts_with(">") {
        Line::from(Span::styled(
            line_owned,
            Style::default().fg(Color::Yellow).bold(),
        ))
    } else if line_owned.starts_with("✓") {
        Line::from(Span::styled(line_owned, Style::default().fg(Color::Green)))
    } else if line_owned.starts_with("✗") {
        Line::from(Span::styled(line_owned, Style::default().fg(Color::Red)))
    } else if line_owned.starts_with("+") {
        Line::from(Span::styled(line_owned, Style::default().fg(Color::Blue)))
    } else {
        Line::from(line_owned)
    }
}

/// Scrolls the agent output up by the specified number of lines.
const fn scroll_up(state: &mut TuiState, lines: u16) {
    state.agent_output_scroll = state.agent_output_scroll.saturating_sub(lines);
}

/// Scrolls the agent output down by the specified number of lines.
#[allow(clippy::cast_possible_truncation)] // Terminal line counts fit in u16
fn scroll_down(state: &mut TuiState, lines: u16, max_lines: usize, visible_height: usize) {
    let max_scroll = max_lines.saturating_sub(visible_height) as u16;
    state.agent_output_scroll = (state.agent_output_scroll + lines).min(max_scroll);
}

/// Scrolls to the top of the agent output.
const fn scroll_to_top(state: &mut TuiState) {
    state.agent_output_scroll = 0;
}

/// Scrolls to the bottom of the agent output.
#[allow(clippy::cast_possible_truncation)] // Terminal line counts fit in u16
#[allow(clippy::missing_const_for_fn)] // if-else prevents const
fn scroll_to_bottom(state: &mut TuiState, total_lines: usize, visible_height: usize) {
    if total_lines > visible_height {
        state.agent_output_scroll = (total_lines - visible_height) as u16;
    } else {
        state.agent_output_scroll = 0;
    }
}

/// Handles keyboard events for the agent output panel.
///
/// This function processes keyboard events when the agent output panel
/// has focus. Supported keys:
///
/// - `PageUp`: Scroll up by visible height
/// - `PageDown`: Scroll down by visible height
/// - `Up`/`k`: Scroll up by 1 line
/// - `Down`/`j`: Scroll down by 1 line
/// - `Home`/`g`: Scroll to top
/// - `End`/`G`: Scroll to bottom
///
/// Returns `true` if the key was handled, `false` otherwise.
pub fn handle_agent_output_key(
    state: &mut TuiState,
    key_code: crossterm::event::KeyCode,
    visible_height: u16,
) -> bool {
    use crossterm::event::KeyCode;

    // Calculate total lines from current messages
    // This must match format_messages() logic: single empty messages become blank lines,
    // consecutive empty messages are collapsed
    let total_lines = state
        .selected_agent_messages()
        .map_or(0, |msgs| format_messages(msgs).len());

    let page_size = visible_height.saturating_sub(2); // Account for borders

    match key_code {
        KeyCode::PageUp => {
            scroll_up(state, page_size);
            true
        }
        KeyCode::PageDown => {
            scroll_down(state, page_size, total_lines, page_size as usize);
            true
        }
        KeyCode::Up | KeyCode::Char('k') => {
            scroll_up(state, 1);
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            scroll_down(state, 1, total_lines, page_size as usize);
            true
        }
        KeyCode::Home | KeyCode::Char('g') => {
            scroll_to_top(state);
            true
        }
        KeyCode::End | KeyCode::Char('G') => {
            scroll_to_bottom(state, total_lines, page_size as usize);
            true
        }
        _ => false,
    }
}
