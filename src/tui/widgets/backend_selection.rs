//! Backend selection popup widget module.
//!
//! This module provides a popup overlay for selecting the AI backend
//! when multiple backends are available. It appears over the splash screen
//! during startup.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::backend::BackendKind;

// ============================================================================
// Backend Selection State
// ============================================================================

/// State for the backend selection popup.
#[derive(Debug, Clone, Default)]
pub struct BackendSelectionState {
    /// Available backends to choose from
    pub available_backends: Vec<BackendKind>,
    /// Currently selected index
    pub selected_index: usize,
    /// Whether the selection popup should be visible
    pub visible: bool,
}

impl BackendSelectionState {
    /// Creates a new backend selection state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a backend selection state with the given available backends.
    ///
    /// The popup is automatically made visible if there are multiple backends.
    #[must_use]
    pub fn with_backends(backends: Vec<BackendKind>) -> Self {
        let visible = backends.len() > 1;
        Self {
            available_backends: backends,
            selected_index: 0,
            visible,
        }
    }

    /// Selects the next backend in the list.
    pub fn select_next(&mut self) {
        if !self.available_backends.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.available_backends.len();
        }
    }

    /// Selects the previous backend in the list.
    pub fn select_previous(&mut self) {
        if !self.available_backends.is_empty() {
            self.selected_index = self
                .selected_index
                .checked_sub(1)
                .unwrap_or(self.available_backends.len() - 1);
        }
    }

    /// Returns the currently selected backend, if any.
    #[must_use]
    pub fn selected_backend(&self) -> Option<BackendKind> {
        self.available_backends.get(self.selected_index).copied()
    }

    /// Confirms selection and hides the popup.
    ///
    /// Returns the selected backend.
    pub fn confirm_selection(&mut self) -> Option<BackendKind> {
        self.visible = false;
        self.selected_backend()
    }
}

// ============================================================================
// Rendering
// ============================================================================

/// Renders the backend selection popup on top of the splash screen.
///
/// This function renders a centered popup for selecting between available
/// AI backends. It uses the `Clear` widget to clear the area behind the
/// popup, ensuring the selection is readable over the animated splash.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The full terminal area to render the overlay on
/// * `state` - The backend selection state
pub fn render_backend_selection_popup(
    frame: &mut Frame,
    area: Rect,
    state: &BackendSelectionState,
) {
    // Calculate popup dimensions - small centered popup
    let popup_area = centered_rect(area, 50, 40);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build the content
    let lines = build_selection_content(state);
    let content = Text::from(lines);

    // Create the block with styled borders
    let block = Block::default()
        .title(" Select Backend ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(content)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, popup_area);
}

/// Builds the content lines for the backend selection popup.
fn build_selection_content(state: &BackendSelectionState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Multiple AI backends are available:",
        Style::default().fg(Color::Gray),
    )));
    lines.push(Line::from(""));

    // Backend options
    for (idx, &backend) in state.available_backends.iter().enumerate() {
        let is_selected = idx == state.selected_index;
        lines.push(build_backend_line(backend, is_selected, idx));
    }

    // Instructions at the bottom
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
        Span::raw(" Navigate  "),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" Select"),
    ]));

    lines
}

/// Builds a single backend option line with selection indicator.
fn build_backend_line(backend: BackendKind, is_selected: bool, index: usize) -> Line<'static> {
    let mut spans = Vec::new();

    // Selection indicator (cursor)
    let cursor = if is_selected { "▶ " } else { "  " };
    spans.push(Span::styled(
        cursor.to_string(),
        Style::default().fg(Color::Cyan),
    ));

    // Number prefix
    spans.push(Span::styled(
        format!("{}. ", index + 1),
        Style::default().fg(Color::DarkGray),
    ));

    // Backend name
    let name = backend.as_str();
    let name_style = if is_selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    spans.push(Span::styled(name.to_string(), name_style));

    // Description
    let description = match backend {
        BackendKind::Auggie => " - Augment CLI (auggie)",
        BackendKind::Cursor => " - Cursor Agent CLI",
    };
    spans.push(Span::styled(
        description.to_string(),
        Style::default().fg(Color::DarkGray),
    ));

    Line::from(spans)
}

/// Creates a centered rectangle within the given area.
fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_popup_to_string(state: &BackendSelectionState) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_backend_selection_popup(frame, frame.area(), state))
            .expect("popup should render");
        format!("{}", terminal.backend())
    }

    // ========================================================================
    // Visibility Rules
    // ========================================================================

    #[test]
    fn test_with_backends_visibility_rules() {
        assert!(!BackendSelectionState::with_backends(vec![]).visible);
        assert!(!BackendSelectionState::with_backends(vec![BackendKind::Auggie]).visible);
        assert!(
            BackendSelectionState::with_backends(vec![BackendKind::Auggie, BackendKind::Cursor])
                .visible
        );
    }

    #[test]
    fn test_popup_visible_only_with_multiple_backends() {
        // Zero backends: not visible
        let zero = BackendSelectionState::with_backends(vec![]);
        assert!(!zero.visible, "Zero backends should not show popup");
        assert_eq!(zero.available_backends.len(), 0);

        // One backend: not visible
        let one = BackendSelectionState::with_backends(vec![BackendKind::Auggie]);
        assert!(!one.visible, "Single backend should not show popup");
        assert_eq!(one.available_backends.len(), 1);

        // Two backends: visible
        let two = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);
        assert!(two.visible, "Multiple backends should show popup");
        assert_eq!(two.available_backends.len(), 2);
    }

    #[test]
    fn test_new_creates_default_invisible_state() {
        let state = BackendSelectionState::new();
        assert!(!state.visible);
        assert!(state.available_backends.is_empty());
        assert_eq!(state.selected_index, 0);
    }

    // ========================================================================
    // Navigation (Wraparound)
    // ========================================================================

    #[test]
    fn test_select_next_wraps_to_start() {
        let mut state =
            BackendSelectionState::with_backends(vec![BackendKind::Auggie, BackendKind::Cursor]);

        state.selected_index = 1;
        state.select_next();

        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));
    }

    #[test]
    fn test_select_previous_wraps_to_end() {
        let mut state =
            BackendSelectionState::with_backends(vec![BackendKind::Auggie, BackendKind::Cursor]);

        state.select_previous();

        assert_eq!(state.selected_index, 1);
        assert_eq!(state.selected_backend(), Some(BackendKind::Cursor));
    }

    #[test]
    fn test_select_next_on_empty_does_not_panic() {
        let mut state = BackendSelectionState::with_backends(vec![]);
        state.select_next();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_select_previous_on_empty_does_not_panic() {
        let mut state = BackendSelectionState::with_backends(vec![]);
        state.select_previous();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_navigation_cycle_through_all_backends() {
        let mut state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);

        // Start at index 0
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));

        // Navigate forward through all items and wrap
        state.select_next();
        assert_eq!(state.selected_index, 1);
        assert_eq!(state.selected_backend(), Some(BackendKind::Cursor));

        state.select_next();
        assert_eq!(state.selected_index, 0); // wrapped
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));

        // Navigate backward through all items and wrap
        state.select_previous();
        assert_eq!(state.selected_index, 1); // wrapped to end
        assert_eq!(state.selected_backend(), Some(BackendKind::Cursor));

        state.select_previous();
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));
    }

    // ========================================================================
    // selected_backend() after navigation
    // ========================================================================

    #[test]
    fn test_selected_backend_returns_none_for_out_of_bounds_index() {
        let mut state = BackendSelectionState::with_backends(vec![BackendKind::Auggie]);
        state.selected_index = 99;

        assert_eq!(state.selected_backend(), None);
    }

    #[test]
    fn test_selected_backend_returns_none_for_empty_backends() {
        let state = BackendSelectionState::with_backends(vec![]);
        assert_eq!(state.selected_backend(), None);
    }

    #[test]
    fn test_selected_backend_returns_correct_backend_after_navigation() {
        let mut state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);

        // Initially at index 0
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));

        // After navigating down
        state.select_next();
        assert_eq!(state.selected_backend(), Some(BackendKind::Cursor));

        // After navigating up (wraps to end)
        state.select_previous();
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));
    }

    // ========================================================================
    // Confirmation
    // ========================================================================

    #[test]
    fn test_confirm_selection_returns_backend_and_hides_popup() {
        let mut state =
            BackendSelectionState::with_backends(vec![BackendKind::Auggie, BackendKind::Cursor]);
        state.selected_index = 1;

        let selected = state.confirm_selection();

        assert_eq!(selected, Some(BackendKind::Cursor));
        assert!(!state.visible);
    }

    #[test]
    fn test_confirm_selection_hides_popup_even_if_already_hidden() {
        let mut state = BackendSelectionState::with_backends(vec![BackendKind::Auggie]);
        assert!(!state.visible); // Single backend, not visible

        let selected = state.confirm_selection();

        assert_eq!(selected, Some(BackendKind::Auggie));
        assert!(!state.visible);
    }

    #[test]
    fn test_confirm_selection_returns_first_backend_by_default() {
        let mut state = BackendSelectionState::with_backends(vec![
            BackendKind::Cursor,
            BackendKind::Auggie,
        ]);

        let selected = state.confirm_selection();

        assert_eq!(selected, Some(BackendKind::Cursor)); // First in list
    }

    #[test]
    fn test_confirm_selection_returns_none_for_empty_backends() {
        let mut state = BackendSelectionState::with_backends(vec![]);

        let selected = state.confirm_selection();

        assert_eq!(selected, None);
        assert!(!state.visible);
    }

    // ========================================================================
    // Render Tests
    // ========================================================================

    #[test]
    fn test_render_popup_shows_backends_and_instructions() {
        let state =
            BackendSelectionState::with_backends(vec![BackendKind::Auggie, BackendKind::Cursor]);

        let rendered = render_popup_to_string(&state);

        assert!(rendered.contains("Select Backend"));
        assert!(rendered.contains("Multiple AI backends are available:"));
        assert!(rendered.contains("auggie"));
        assert!(rendered.contains("cursor"));
        assert!(rendered.contains("Navigate"));
        assert!(rendered.contains("Select"));
    }

    #[test]
    fn test_render_popup_shows_selection_indicator() {
        let mut state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);

        // First backend selected - should show cursor indicator (▶)
        let rendered = render_popup_to_string(&state);
        assert!(rendered.contains('▶'));

        // Navigate to second backend
        state.select_next();
        let rendered2 = render_popup_to_string(&state);
        assert!(rendered2.contains('▶'));
    }

    #[test]
    fn test_render_popup_shows_numbered_options() {
        let state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);

        let rendered = render_popup_to_string(&state);

        // Should show numbered options
        assert!(rendered.contains("1."), "Should show numbered option 1");
        assert!(rendered.contains("2."), "Should show numbered option 2");
    }

    #[test]
    fn test_render_popup_centered_in_area() {
        let state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);

        // Render to a larger area
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_backend_selection_popup(frame, frame.area(), &state))
            .expect("popup should render");

        // Check that the popup renders without panic
        let rendered = format!("{}", terminal.backend());
        assert!(rendered.contains("Select Backend"));
    }

    #[test]
    fn test_centered_rect_returns_valid_rect() {
        let area = Rect::new(0, 0, 100, 50);
        let popup = centered_rect(area, 50, 40);

        // Popup should be smaller than the area
        assert!(popup.width < area.width);
        assert!(popup.height < area.height);
        assert!(popup.width > 0);
        assert!(popup.height > 0);
    }

    // ========================================================================
    // Integration Tests
    // ========================================================================

    #[test]
    fn test_full_selection_flow() {
        // User opens backend selection with multiple backends
        let mut state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);
        assert!(state.visible);
        assert_eq!(state.selected_index, 0);

        // User navigates down
        state.select_next();
        assert_eq!(state.selected_backend(), Some(BackendKind::Cursor));

        // User navigates up (back to first)
        state.select_previous();
        assert_eq!(state.selected_backend(), Some(BackendKind::Auggie));

        // User navigates up again (wraps to last)
        state.select_previous();
        assert_eq!(state.selected_backend(), Some(BackendKind::Cursor));

        // User confirms selection
        let selected = state.confirm_selection();
        assert_eq!(selected, Some(BackendKind::Cursor));
        assert!(!state.visible);
    }

    #[test]
    fn test_build_backend_line_selected_vs_unselected() {
        // Test selected line
        let selected_line = build_backend_line(BackendKind::Auggie, true, 0);
        let selected_spans: Vec<_> = selected_line.spans.iter().collect();
        assert!(!selected_spans.is_empty());

        // Test unselected line
        let unselected_line = build_backend_line(BackendKind::Cursor, false, 1);
        let unselected_spans: Vec<_> = unselected_line.spans.iter().collect();
        assert!(!unselected_spans.is_empty());

        // Selected line should have cursor indicator
        let has_cursor = selected_line.spans.iter().any(|s| s.content.contains('▶'));
        assert!(has_cursor, "Selected line should have cursor indicator");
    }

    #[test]
    fn test_build_selection_content_empty_backends() {
        let state = BackendSelectionState::with_backends(vec![]);
        let lines = build_selection_content(&state);

        // Should still have header and instructions even with no backends
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_build_selection_content_includes_all_backends() {
        let state = BackendSelectionState::with_backends(vec![
            BackendKind::Auggie,
            BackendKind::Cursor,
        ]);
        let lines = build_selection_content(&state);

        // Convert lines to string for easier assertion
        let content: String = lines.iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();

        assert!(content.contains("auggie"));
        assert!(content.contains("cursor"));
    }
}
