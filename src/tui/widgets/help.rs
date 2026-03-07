//! Help overlay widget module.
//!
//! This module contains the [`render_help_overlay`] function which renders
//! a modal overlay displaying all keyboard shortcuts organized by context.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Keyboard shortcut section with title and key bindings.
struct HelpSection {
    title: &'static str,
    shortcuts: &'static [(&'static str, &'static str)],
}

/// All help sections with their keyboard shortcuts.
const HELP_SECTIONS: &[HelpSection] = &[
    HelpSection {
        title: "Global",
        shortcuts: &[
            ("Tab", "Cycle focus between panels"),
            ("q", "Quit TUI"),
            ("?", "Toggle this help"),
        ],
    },
    HelpSection {
        title: "Agent Tree (when focused)",
        shortcuts: &[
            ("↑/↓", "Navigate agents"),
            ("←/→", "Expand/collapse"),
            ("Enter", "Select agent"),
            ("f", "Toggle auto-follow mode"),
        ],
    },
    HelpSection {
        title: "Agent Output (when focused)",
        shortcuts: &[
            ("PgUp/PgDn", "Scroll output"),
            ("Home/End", "Jump to top/bottom"),
        ],
    },
    HelpSection {
        title: "Task List (when focused)",
        shortcuts: &[("↑/↓", "Navigate tasks"), ("PgUp/PgDn", "Scroll list")],
    },
    HelpSection {
        title: "App Logs (when focused)",
        shortcuts: &[
            ("h", "Toggle target selector"),
            ("←/→", "Filter by level"),
            ("PgUp/PgDn", "Scroll logs"),
        ],
    },
];

/// Renders the help overlay on top of the main UI.
///
/// This function renders a centered popup with all keyboard shortcuts
/// organized by context. It uses the `Clear` widget to clear the area
/// behind the popup, ensuring the help text is readable.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The full terminal area to render the overlay on
///
/// # Example
///
/// ```ignore
/// use villalobos::tui::widgets::render_help_overlay;
///
/// fn render(frame: &mut Frame, state: &TuiState) {
///     // Render main UI first...
///     
///     // Render help overlay last (on top of everything)
///     if state.help_visible {
///         render_help_overlay(frame, frame.area());
///     }
/// }
/// ```
pub fn render_help_overlay(frame: &mut Frame, area: Rect) {
    // Calculate popup dimensions
    let popup_area = centered_rect(area, 60, 80);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build the help content
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (idx, section) in HELP_SECTIONS.iter().enumerate() {
        // Add blank line between sections (but not before the first)
        if idx > 0 {
            lines.push(Line::from(""));
        }

        // Section title with styling
        lines.push(Line::from(Span::styled(
            section.title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));

        // Shortcuts with aligned columns
        for (key, description) in section.shortcuts {
            let key_span = Span::styled(
                format!("  {key:12}"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
            let desc_span = Span::raw(*description);
            lines.push(Line::from(vec![key_span, desc_span]));
        }
    }

    // Add dismissal hint at the bottom
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press ? or Esc to close",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let help_text = Text::from(lines);

    // Create the block with styled borders
    let block = Block::default()
        .title(" Keyboard Shortcuts ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(help_text).block(block);

    frame.render_widget(paragraph, popup_area);
}

/// Creates a centered rectangle within the given area.
///
/// # Arguments
///
/// * `area` - The area to center within
/// * `percent_x` - Percentage of horizontal space to use
/// * `percent_y` - Percentage of vertical space to use
fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
