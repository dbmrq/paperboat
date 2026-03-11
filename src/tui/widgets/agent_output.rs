//! Agent output widget module.
//!
//! This module contains [`render_agent_output`] which displays
//! real-time output from agents, including support for ANSI colors
//! and scrolling.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
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
        // Show animated "waiting" display with boat and waves
        // Pass inner width (minus borders) for centering
        render_waiting_animation(state, area.width.saturating_sub(2))
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

/// Renders an animated waiting display with a paper boat sailing on waves.
///
/// The animation consists of:
/// 1. A boat emoji (⛵) that moves forward across the screen, then "washes back" to start
/// 2. Two rows of animated waves that shift horizontally
/// 3. A status line showing "Agent working..." with elapsed time (if agent is running)
///
/// The boat moves forward over ~12 seconds, then quickly washes back over ~2 seconds.
// Wave characters that create a flowing water effect
const WAVE_CHARS: [char; 4] = ['~', '·', '~', '·'];
const WAVE_WIDTH: usize = 24;

// Boat movement: moves forward over ~1800 frames (30s), washes back over ~300 frames (5s)
// Total cycle: 2100 frames (~35 seconds at 60fps)
const FORWARD_FRAMES: u32 = 1800;
const BACKWARD_FRAMES: u32 = 300;
const CYCLE_FRAMES: u32 = FORWARD_FRAMES + BACKWARD_FRAMES;

fn render_waiting_animation(state: &TuiState, panel_width: u16) -> Text<'static> {
    let frame = state.animation_frame;

    let cycle_pos = frame % CYCLE_FRAMES;
    // Boat position in visual columns (0 to WAVE_WIDTH - 2, since boat takes 2 columns)
    let max_boat_pos = WAVE_WIDTH - 2;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let boat_pos = if cycle_pos < FORWARD_FRAMES {
        // Moving forward: ease-out for smooth deceleration
        let progress = f64::from(cycle_pos) / f64::from(FORWARD_FRAMES);
        let eased = (1.0 - progress).mul_add(-(1.0 - progress), 1.0); // ease-out quadratic
        (eased * max_boat_pos as f64) as usize
    } else {
        // Washing back: quick linear return
        let backward_progress = f64::from(cycle_pos - FORWARD_FRAMES) / f64::from(BACKWARD_FRAMES);
        ((1.0 - backward_progress) * max_boat_pos as f64) as usize
    };

    // Calculate centering padding
    let padding = (panel_width as usize).saturating_sub(WAVE_WIDTH) / 2;
    let pad_str = " ".repeat(padding);

    // Wave offsets - different speeds for parallax depth effect
    // Closer waves (bottom) move faster, distant waves (top) move slower
    let wave_offset_top = (frame / 24) as usize; // Slowest (horizon)
    let wave_offset_mid = (frame / 18) as usize; // Medium
    let wave_offset_bottom = (frame / 12) as usize; // Fastest (closest)
    let mut boat_line = String::with_capacity(WAVE_WIDTH + padding);
    let mut wave_line1 = String::with_capacity(WAVE_WIDTH + padding);
    let mut wave_line2 = String::with_capacity(WAVE_WIDTH + padding);

    // Add centering padding
    boat_line.push_str(&pad_str);
    wave_line1.push_str(&pad_str);
    wave_line2.push_str(&pad_str);

    // Build boat line (boat emoji is double-width, so we track visual position)
    let mut visual_col = 0;
    while visual_col < WAVE_WIDTH {
        if visual_col == boat_pos {
            boat_line.push('⛵');
            visual_col += 2; // Boat takes 2 visual columns
        } else {
            let wave_char = WAVE_CHARS[(visual_col + wave_offset_top) % WAVE_CHARS.len()];
            boat_line.push(wave_char);
            visual_col += 1;
        }
    }

    // Build wave lines with different offsets for parallax effect
    for i in 0..WAVE_WIDTH {
        let wave_char1 = WAVE_CHARS[(i + wave_offset_mid) % WAVE_CHARS.len()];
        let wave_char2 = WAVE_CHARS[(i + wave_offset_bottom) % WAVE_CHARS.len()];
        wave_line1.push(wave_char1);
        wave_line2.push(wave_char2);
    }

    // Build the status message with elapsed time
    let status_message = if let Some(agent) = state.selected_agent() {
        use super::super::agent_node::AgentStatus;
        match agent.status {
            AgentStatus::Running => {
                let elapsed = agent.start_time.elapsed();
                let secs = elapsed.as_secs();
                let mins = secs / 60;
                let secs = secs % 60;
                if mins > 0 {
                    format!("Agent working... {mins}m {secs}s")
                } else {
                    format!("Agent working... {secs}s")
                }
            }
            AgentStatus::Completed => "Agent completed".to_string(),
            AgentStatus::Failed => "Agent failed".to_string(),
        }
    } else {
        "Waiting for output...".to_string()
    };

    // Center the status message
    let status_padding = (panel_width as usize).saturating_sub(status_message.len()) / 2;
    let centered_status = format!("{}{}", " ".repeat(status_padding), status_message);

    // Use intensity modifiers instead of hardcoded colors for theme compatibility
    // DIM = faint/far (horizon), normal = mid, BOLD = bright/close
    Text::from(vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            boat_line,
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(wave_line1, Style::default())),
        Line::from(Span::styled(
            wave_line2,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            centered_status,
            Style::default()
                .add_modifier(Modifier::DIM)
                .add_modifier(Modifier::ITALIC),
        )),
    ])
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
    use ratatui::{backend::TestBackend, Terminal};

    use crate::logging::{AgentType, LogEvent};

    fn render_agent_output_to_string(state: &mut TuiState, area: Rect) -> String {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_agent_output(frame, area, state, true))
            .expect("agent output should render");
        format!("{}", terminal.backend())
    }

    fn state_with_selected_agent(session_id: &str) -> TuiState {
        let mut state = TuiState::new();
        state.splash_visible = false;
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: session_id.to_string(),
            depth: 0,
            task: "Test task".to_string(),
        });
        state
    }

    // ========================================================================
    // Format Messages Tests
    // ========================================================================

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
        let messages = vec!["Line 1".to_string(), String::new(), "Line 2".to_string()];
        let lines = format_messages(&messages);

        // Should produce 3 lines: Line 1, blank, Line 2
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_format_messages_consecutive_empty() {
        // Simulates "Line 1\n\n\nLine 2" which gets split into ["Line 1", "", "", "Line 2"]
        let messages = vec![
            "Line 1".to_string(),
            String::new(),
            String::new(),
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
        let messages = vec!["Hello".to_string(), String::new()];
        let lines = format_messages(&messages);

        // Should be: Hello, then a blank line = 2 lines
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_format_messages_mixed_content() {
        // Mixed: text, empty (newline), text, empty, empty (collapsed), text
        let messages = vec![
            "Line 1".to_string(),
            String::new(),
            "Line 2".to_string(),
            String::new(),
            String::new(),
            "Line 3".to_string(),
        ];
        let lines = format_messages(&messages);

        // Line 1 + blank + Line 2 + blank (collapsed from 2 empties) + Line 3 = 5 lines
        assert_eq!(lines.len(), 5);
    }

    // ========================================================================
    // Auto-Scroll Tests
    // ========================================================================

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn test_render_agent_output_auto_scrolls_when_new_messages_arrive() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 40, 6);

        for idx in 0..8 {
            state
                .agent_tree_state
                .handle_standalone_message(Some("agent-1"), &format!("line {idx}"));
        }

        render_agent_output_to_string(&mut state, area);

        let expected_scroll = Paragraph::new(Text::from(
            state
                .selected_agent_messages()
                .expect("messages should exist")
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>(),
        ))
        .block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: false })
        .line_count(area.width)
        .saturating_sub(area.height.saturating_sub(2) as usize)
            as u16;

        assert_eq!(state.last_message_count, 8);
        assert_eq!(state.agent_output_scroll, expected_scroll);
    }

    #[test]
    fn test_auto_scroll_triggers_on_new_content() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 40, 6);

        // Add initial messages using standalone messages (simpler counting)
        for idx in 0..3 {
            state
                .agent_tree_state
                .handle_standalone_message(Some("agent-1"), &format!("message {idx}"));
        }

        // Render to set baseline
        render_agent_output_to_string(&mut state, area);
        let initial_count = state.last_message_count;
        assert_eq!(initial_count, 3);

        // User scrolls up manually
        state.agent_output_scroll = 0;

        // New message arrives
        state
            .agent_tree_state
            .handle_standalone_message(Some("agent-1"), "new message");

        // Render again - should auto-scroll
        render_agent_output_to_string(&mut state, area);
        assert!(
            state.last_message_count > initial_count,
            "Message count should increase after new message"
        );
    }

    #[test]
    fn test_auto_scroll_stays_at_zero_when_content_fits() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 80, 20); // Large area

        // Add a single short message
        state.handle_event(LogEvent::AgentMessage {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            content: "short".to_string(),
        });

        render_agent_output_to_string(&mut state, area);

        // Scroll should be 0 when content fits
        assert_eq!(state.agent_output_scroll, 0);
    }

    // ========================================================================
    // Scroll Clamp Tests
    // ========================================================================

    #[test]
    fn test_render_agent_output_clamps_scroll_after_content_shrinks() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 40, 6);
        state.agent_output_scroll = 99;

        state.handle_event(LogEvent::AgentMessage {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            content: "short output".to_string(),
        });
        state.last_message_count = 1;

        render_agent_output_to_string(&mut state, area);

        assert_eq!(state.agent_output_scroll, 0);
        assert_eq!(state.last_message_count, 1);
    }

    #[test]
    fn test_scroll_clamp_to_valid_range() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 40, 6);

        // Set an absurdly high scroll
        state.agent_output_scroll = 9999;

        // Add minimal content
        state.handle_event(LogEvent::AgentMessage {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            content: "line1".to_string(),
        });
        state.last_message_count = 1;

        render_agent_output_to_string(&mut state, area);

        // Scroll should be clamped to valid range (0 since content fits)
        assert_eq!(state.agent_output_scroll, 0);
    }

    #[test]
    fn test_scroll_maintains_position_when_valid() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 40, 6);

        // Add enough content to scroll
        for idx in 0..15 {
            state
                .agent_tree_state
                .handle_standalone_message(Some("agent-1"), &format!("line {idx}"));
        }

        // Set initial render
        render_agent_output_to_string(&mut state, area);
        let auto_scroll = state.agent_output_scroll;

        // User scrolls up by 2
        if auto_scroll > 2 {
            state.agent_output_scroll = auto_scroll - 2;
            let manual_scroll = state.agent_output_scroll;

            // Render again (no new messages)
            render_agent_output_to_string(&mut state, area);

            // Scroll should stay at manual position (no new messages)
            assert_eq!(state.agent_output_scroll, manual_scroll);
        }
    }

    // ========================================================================
    // Waiting State Text Tests (Running/Completed/Failed Agents)
    // ========================================================================

    #[test]
    fn test_render_agent_output_shows_waiting_text_for_running_agent() {
        let mut state = state_with_selected_agent("agent-1");

        let rendered = render_agent_output_to_string(&mut state, Rect::new(0, 0, 60, 10));

        assert!(rendered.contains("Agent working..."));
    }

    #[test]
    fn test_waiting_state_shows_completed_text() {
        let mut state = state_with_selected_agent("agent-1");

        // Mark agent as completed
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            success: true,
        });

        let rendered = render_agent_output_to_string(&mut state, Rect::new(0, 0, 60, 10));

        assert!(rendered.contains("Agent completed"));
    }

    #[test]
    fn test_waiting_state_shows_failed_text() {
        let mut state = state_with_selected_agent("agent-1");

        // Mark agent as failed
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            success: false,
        });

        let rendered = render_agent_output_to_string(&mut state, Rect::new(0, 0, 60, 10));

        assert!(rendered.contains("Agent failed"));
    }

    #[test]
    fn test_render_no_agent_selected_message() {
        let mut state = TuiState::new();
        state.splash_visible = false;
        // No agent selected

        let rendered = render_agent_output_to_string(&mut state, Rect::new(0, 0, 60, 10));

        assert!(rendered.contains("No agent selected"));
        assert!(rendered.contains("Select an agent"));
    }

    #[test]
    fn test_render_shows_agent_title_when_selected() {
        let mut state = state_with_selected_agent("agent-1");

        let rendered = render_agent_output_to_string(&mut state, Rect::new(0, 0, 60, 10));

        // Title should include some form of "Output"
        assert!(rendered.contains("Output"));
    }

    // ========================================================================
    // Keyboard Navigation Tests
    // ========================================================================

    #[test]
    #[allow(clippy::cast_possible_truncation)]
    fn test_handle_agent_output_key_end_scrolls_to_bottom_of_wrapped_content() {
        let mut state = state_with_selected_agent("agent-1");
        state.handle_event(LogEvent::AgentMessage {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            content: "this is a deliberately long line that wraps multiple times".to_string(),
        });

        let handled = handle_agent_output_key(&mut state, crossterm::event::KeyCode::End, 6, 18);

        let total_lines = calculate_wrapped_line_count(
            state
                .selected_agent_messages()
                .expect("messages should exist"),
            16,
        );
        let visible_height = 4;

        assert!(handled);
        assert_eq!(
            state.agent_output_scroll,
            total_lines.saturating_sub(visible_height) as u16
        );
    }

    #[test]
    fn test_handle_key_home_scrolls_to_top() {
        let mut state = state_with_selected_agent("agent-1");

        // Add content and scroll down
        for idx in 0..20 {
            state
                .agent_tree_state
                .handle_standalone_message(Some("agent-1"), &format!("line {idx}"));
        }
        state.agent_output_scroll = 10;

        let handled = handle_agent_output_key(&mut state, crossterm::event::KeyCode::Home, 10, 40);

        assert!(handled);
        assert_eq!(state.agent_output_scroll, 0);
    }

    #[test]
    fn test_handle_key_g_scrolls_to_top() {
        let mut state = state_with_selected_agent("agent-1");
        state.agent_output_scroll = 15;

        let handled =
            handle_agent_output_key(&mut state, crossterm::event::KeyCode::Char('g'), 10, 40);

        assert!(handled);
        assert_eq!(state.agent_output_scroll, 0);
    }

    #[test]
    fn test_handle_key_up_scrolls_up_one_line() {
        let mut state = state_with_selected_agent("agent-1");
        state.agent_output_scroll = 5;

        let handled = handle_agent_output_key(&mut state, crossterm::event::KeyCode::Up, 10, 40);

        assert!(handled);
        assert_eq!(state.agent_output_scroll, 4);
    }

    #[test]
    fn test_handle_key_k_scrolls_up_one_line() {
        let mut state = state_with_selected_agent("agent-1");
        state.agent_output_scroll = 5;

        let handled =
            handle_agent_output_key(&mut state, crossterm::event::KeyCode::Char('k'), 10, 40);

        assert!(handled);
        assert_eq!(state.agent_output_scroll, 4);
    }

    #[test]
    fn test_handle_key_down_scrolls_down_one_line() {
        let mut state = state_with_selected_agent("agent-1");

        // Add content
        for idx in 0..20 {
            state
                .agent_tree_state
                .handle_standalone_message(Some("agent-1"), &format!("line {idx}"));
        }
        state.agent_output_scroll = 0;
        state.last_message_count = 20;

        let handled = handle_agent_output_key(&mut state, crossterm::event::KeyCode::Down, 10, 40);

        assert!(handled);
        assert_eq!(state.agent_output_scroll, 1);
    }

    #[test]
    fn test_handle_key_j_scrolls_down_one_line() {
        let mut state = state_with_selected_agent("agent-1");

        // Add content
        for idx in 0..20 {
            state
                .agent_tree_state
                .handle_standalone_message(Some("agent-1"), &format!("line {idx}"));
        }
        state.agent_output_scroll = 0;
        state.last_message_count = 20;

        let handled =
            handle_agent_output_key(&mut state, crossterm::event::KeyCode::Char('j'), 10, 40);

        assert!(handled);
        assert_eq!(state.agent_output_scroll, 1);
    }

    #[test]
    fn test_handle_key_page_up_scrolls_by_page() {
        let mut state = state_with_selected_agent("agent-1");
        state.agent_output_scroll = 20;

        let handled = handle_agent_output_key(
            &mut state,
            crossterm::event::KeyCode::PageUp,
            10, // visible height
            40,
        );

        assert!(handled);
        // Page size is visible_height - 2 (for borders) = 8
        assert_eq!(state.agent_output_scroll, 12);
    }

    #[test]
    fn test_handle_unrecognized_key_returns_false() {
        let mut state = state_with_selected_agent("agent-1");

        let handled =
            handle_agent_output_key(&mut state, crossterm::event::KeyCode::Char('x'), 10, 40);

        assert!(!handled);
    }

    // ========================================================================
    // Line Formatting Tests
    // ========================================================================

    #[test]
    fn test_format_line_tool_call_styling() {
        let line = format_line("> Calling: view");
        // Should be styled as tool call (yellow/bold)
        assert!(!line.spans.is_empty());
        let span = &line.spans[0];
        assert!(span.content.starts_with('>'));
        assert_eq!(span.style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_format_line_success_styling() {
        let line = format_line("✓ view completed");
        assert!(!line.spans.is_empty());
        let span = &line.spans[0];
        assert!(span.content.starts_with('✓'));
        assert_eq!(span.style.fg, Some(Color::Green));
    }

    #[test]
    fn test_format_line_error_styling() {
        let line = format_line("✗ compile failed");
        assert!(!line.spans.is_empty());
        let span = &line.spans[0];
        assert!(span.content.starts_with('✗'));
        assert_eq!(span.style.fg, Some(Color::Red));
    }

    #[test]
    fn test_format_line_added_styling() {
        let line = format_line("+ Added file.rs");
        assert!(!line.spans.is_empty());
        let span = &line.spans[0];
        assert!(span.content.starts_with('+'));
        assert_eq!(span.style.fg, Some(Color::Blue));
    }

    #[test]
    fn test_format_line_regular_text() {
        let line = format_line("Regular text here");
        assert!(!line.spans.is_empty());
        let span = &line.spans[0];
        assert_eq!(span.content.as_ref(), "Regular text here");
        // Regular text has no special foreground color
        assert!(span.style.fg.is_none());
    }

    // ========================================================================
    // calculate_wrapped_line_count Tests
    // ========================================================================

    #[test]
    fn test_calculate_wrapped_line_count_empty() {
        let messages: Vec<String> = vec![];
        let count = calculate_wrapped_line_count(&messages, 40);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_calculate_wrapped_line_count_short_messages() {
        let messages = vec!["short".to_string(), "lines".to_string()];
        let count = calculate_wrapped_line_count(&messages, 40);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_calculate_wrapped_line_count_wrapped_messages() {
        // A message longer than the width should wrap
        let long_msg = "a".repeat(100);
        let messages = vec![long_msg];
        let count = calculate_wrapped_line_count(&messages, 20);
        // 100 chars / 20 width = 5 lines
        assert!(count >= 5);
    }

    // ========================================================================
    // Focus Styling Tests
    // ========================================================================

    #[test]
    fn test_render_focused_vs_unfocused_styling() {
        let mut state = state_with_selected_agent("agent-1");
        state.current_focus = FocusedPanel::AgentOutput;

        // Render focused
        let focused = {
            let backend = TestBackend::new(60, 10);
            let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
            terminal
                .draw(|frame| render_agent_output(frame, frame.area(), &mut state, true))
                .expect("should render");
            format!("{}", terminal.backend())
        };

        // Render unfocused
        state.current_focus = FocusedPanel::AgentTree;
        let unfocused = {
            let backend = TestBackend::new(60, 10);
            let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
            terminal
                .draw(|frame| render_agent_output(frame, frame.area(), &mut state, false))
                .expect("should render");
            format!("{}", terminal.backend())
        };

        // Both should render without panic and contain the title
        assert!(focused.contains("Output"));
        assert!(unfocused.contains("Output"));
    }

    // ========================================================================
    // Scroll Helper Function Tests
    // ========================================================================

    #[test]
    fn test_scroll_up_clamps_at_zero() {
        let mut state = TuiState::new();
        state.agent_output_scroll = 2;

        scroll_up(&mut state, 5);

        assert_eq!(state.agent_output_scroll, 0);
    }

    #[test]
    fn test_scroll_down_clamps_at_max() {
        let mut state = TuiState::new();
        state.agent_output_scroll = 0;

        scroll_down(&mut state, 100, 10, 8); // max_lines=10, visible=8, so max_scroll=2

        assert_eq!(state.agent_output_scroll, 2);
    }

    #[test]
    fn test_scroll_to_top_sets_zero() {
        let mut state = TuiState::new();
        state.agent_output_scroll = 50;

        scroll_to_top(&mut state);

        assert_eq!(state.agent_output_scroll, 0);
    }

    #[test]
    fn test_scroll_to_bottom_when_content_exceeds_height() {
        let mut state = TuiState::new();

        scroll_to_bottom(&mut state, 20, 10); // 20 lines, 10 visible

        assert_eq!(state.agent_output_scroll, 10); // 20 - 10 = 10
    }

    #[test]
    fn test_scroll_to_bottom_when_content_fits() {
        let mut state = TuiState::new();

        scroll_to_bottom(&mut state, 5, 10); // 5 lines, 10 visible

        assert_eq!(state.agent_output_scroll, 0);
    }

    // ========================================================================
    // Integration Tests
    // ========================================================================

    #[test]
    fn test_full_agent_lifecycle_output() {
        let mut state = state_with_selected_agent("agent-1");
        let area = Rect::new(0, 0, 60, 15);

        // Initial render - should show "Agent working..."
        let rendered = render_agent_output_to_string(&mut state, area);
        assert!(rendered.contains("Agent working..."));

        // Agent sends messages
        state.handle_event(LogEvent::AgentMessage {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            content: "I'm analyzing the code.".to_string(),
        });

        let rendered = render_agent_output_to_string(&mut state, area);
        assert!(rendered.contains("analyzing"));

        // Agent completes
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("agent-1".to_string()),
            depth: 0,
            success: true,
        });

        // Message should still be visible (not replaced by "completed")
        let rendered = render_agent_output_to_string(&mut state, area);
        assert!(rendered.contains("analyzing"));
    }
}
