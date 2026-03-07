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
        format!(" {} Output ", agent.display_name(state.animation_frame))
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
    let content = if state.selected_agent_id.is_none() {
        Text::from(vec![
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
        ])
    } else if let Some(msgs) = messages.filter(|v| !v.is_empty()) {
        // Format messages with styling
        let lines = format_messages(msgs);
        Text::from(lines)
    } else {
        Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Waiting for output...",
                Style::default().fg(Color::DarkGray).italic(),
            )),
        ])
    };

    // Calculate visible area (minus borders)
    let inner_height = area.height.saturating_sub(2) as usize;

    // Create the paragraph with block and wrapping to calculate line count.
    // This gives us the actual rendered line count, accounting for wrapped lines.
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false });

    // Use line_count() to get actual rendered lines including wrapped lines.
    // This accounts for the block borders internally.
    let total_lines = paragraph.line_count(area.width);

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

    // Add scroll offset to the paragraph and render
    let paragraph = paragraph.scroll((state.agent_output_scroll, 0));

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
///
/// # Line Break Handling
///
/// - Single `\n` in content creates separate messages (via `handle_agent_message`)
///   which display as consecutive lines (no blank line between)
/// - Double `\n\n` creates an empty message between two content messages,
///   displaying as content, blank line, content
/// - Consecutive empty messages are collapsed to a single blank line
/// - Messages with embedded newlines are split via `.lines()` for proper display
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
    if line_owned.starts_with('>') {
        Line::from(Span::styled(
            line_owned,
            Style::default().fg(Color::Yellow).bold(),
        ))
    } else if line_owned.starts_with('✓') {
        Line::from(Span::styled(line_owned, Style::default().fg(Color::Green)))
    } else if line_owned.starts_with('✗') {
        Line::from(Span::styled(line_owned, Style::default().fg(Color::Red)))
    } else if line_owned.starts_with('+') {
        Line::from(Span::styled(line_owned, Style::default().fg(Color::Blue)))
    } else {
        Line::from(line_owned)
    }
}

/// Calculates the total rendered line count for agent messages, accounting for text wrapping.
///
/// This function creates a temporary Paragraph to get the accurate wrapped line count,
/// matching the rendering logic in `render_agent_output`.
///
/// # Arguments
///
/// * `messages` - The agent messages to count lines for
/// * `inner_width` - The inner width of the panel (excluding borders)
///
/// # Returns
///
/// The total number of rendered lines, including wrapped lines.
#[must_use]
pub fn calculate_wrapped_line_count(messages: &[String], inner_width: u16) -> usize {
    let lines = format_messages(messages);
    let content = Text::from(lines);
    let paragraph = Paragraph::new(content).wrap(Wrap { trim: false });
    paragraph.line_count(inner_width)
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
    visible_width: u16,
) -> bool {
    use crossterm::event::KeyCode;

    // Calculate total lines from current messages, accounting for text wrapping.
    // We need to create a temporary Paragraph to get the accurate wrapped line count.
    // Subtract 2 from width to account for left/right borders, matching render_agent_output
    // which creates a paragraph with Block::default().borders(Borders::ALL).
    let inner_width = visible_width.saturating_sub(2);
    let total_lines = state.selected_agent_messages().map_or(0, |msgs| {
        let lines = format_messages(msgs);
        let content = Text::from(lines);
        let paragraph = Paragraph::new(content).wrap(Wrap { trim: false });
        paragraph.line_count(inner_width)
    });

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_messages_single_line_break() {
        // Simulates "Line 1\nLine 2" which gets split into ["Line 1", "Line 2"]
        let messages = vec!["Line 1".to_string(), "Line 2".to_string()];
        let lines = format_messages(&messages);

        // Should produce two separate lines
        assert_eq!(lines.len(), 2);

        // Verify the actual content
        let line1_spans = &lines[0].spans;
        let line2_spans = &lines[1].spans;
        assert_eq!(line1_spans.len(), 1);
        assert_eq!(line2_spans.len(), 1);
        assert_eq!(line1_spans[0].content.as_ref(), "Line 1");
        assert_eq!(line2_spans[0].content.as_ref(), "Line 2");
    }

    #[test]
    fn test_format_messages_double_line_break() {
        // Simulates "Line 1\n\nLine 2" which gets split into ["Line 1", "", "Line 2"]
        let messages = vec!["Line 1".to_string(), "".to_string(), "Line 2".to_string()];
        let lines = format_messages(&messages);

        // Should produce 3 lines: Line 1, blank, Line 2
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_format_messages_consecutive_empty() {
        // Simulates "Line 1\n\n\nLine 2" which gets split into ["Line 1", "", "", "Line 2"]
        let messages = vec![
            "Line 1".to_string(),
            "".to_string(),
            "".to_string(),
            "Line 2".to_string(),
        ];
        let lines = format_messages(&messages);

        // Consecutive empty messages should be collapsed to one blank line
        // So: Line 1, blank (collapsed), Line 2 = 3 lines
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_format_messages_embedded_newline() {
        // Message with embedded newline (from standalone_message that wasn't split)
        let messages = vec!["Line 1\nLine 2".to_string()];
        let lines = format_messages(&messages);

        // msg.lines() should split this into two lines
        assert_eq!(lines.len(), 2);

        // Verify the content is correct
        assert_eq!(lines[0].spans[0].content.as_ref(), "Line 1");
        assert_eq!(lines[1].spans[0].content.as_ref(), "Line 2");
    }

    #[test]
    fn test_lines_behavior() {
        // Verify that .lines() behaves as expected
        let s = "Line 1\nLine 2";
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines, vec!["Line 1", "Line 2"]);

        // With trailing newline
        let s2 = "Line 1\nLine 2\n";
        let lines2: Vec<_> = s2.lines().collect();
        assert_eq!(lines2, vec!["Line 1", "Line 2"]); // trailing newline is stripped
    }

    /// Test that verifies the exact scenario mentioned in the task:
    /// "Line 1\nLine 2" should display as two separate lines
    #[test]
    fn test_single_newline_splits_correctly() {
        // Simulate the input as it would come from handle_agent_message
        // with content "Line 1\nLine 2"
        // split('\n') gives ["Line 1", "Line 2"]
        let messages = vec!["Line 1".to_string(), "Line 2".to_string()];
        let lines = format_messages(&messages);

        // Should produce exactly 2 lines
        assert_eq!(lines.len(), 2, "Expected 2 lines, got {}", lines.len());

        // Verify content
        assert_eq!(lines[0].spans[0].content.as_ref(), "Line 1");
        assert_eq!(lines[1].spans[0].content.as_ref(), "Line 2");

        // Ensure they are NOT merged into one line
        // (if they were merged, we'd have 1 line with content "Line 1Line 2")
    }

    #[test]
    fn test_format_messages_text_followed_by_empty() {
        // Simulates streaming: "Hello" then "\n" arrives
        // handle_agent_message would produce ["Hello", ""]
        let messages = vec!["Hello".to_string(), "".to_string()];
        let lines = format_messages(&messages);

        // Should be: Hello, then a blank line = 2 lines
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_format_messages_mixed_content() {
        // Mixed: text, empty (newline), text, empty, empty (collapsed), text
        let messages = vec![
            "Line 1".to_string(),
            "".to_string(),
            "Line 2".to_string(),
            "".to_string(),
            "".to_string(),
            "Line 3".to_string(),
        ];
        let lines = format_messages(&messages);

        // Line 1 + blank + Line 2 + blank (collapsed from 2 empties) + Line 3 = 5 lines
        assert_eq!(lines.len(), 5);
    }
}
