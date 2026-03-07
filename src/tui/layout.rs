//! TUI layout management module.
//!
//! This module handles the calculation and management of UI component layouts.
//!
//! # Layout Structure
//!
//! ```text
//! ┌─────────────────────┬─────────────────────────────────┬───────────┐
//! │  Agent Tree         │  Selected Agent Output           │   Tasks   │
//! │  (navigation)       │  (streaming content)             │   List    │
//! │  ~20% width         │  ~50% width                      │  ~30% width│
//! ├─────────────────────┴─────────────────────────────────┴───────────┤
//! │  App Logs (filterable by target/level)                            │
//! │  ~30% height                                                       │
//! ├───────────────────────────────────────────────────────────────────┤
//! │  Status Bar (1 line)                                               │
//! └───────────────────────────────────────────────────────────────────┘
//! ```

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Minimum sizes for panels to remain usable.
mod min_sizes {
    /// Height of the status bar (always 1 line)
    pub const STATUS_BAR_HEIGHT: u16 = 1;
    /// Minimum total terminal width for reasonable layout
    pub const MIN_TERMINAL_WIDTH: u16 = 80;
    /// Minimum total terminal height for reasonable layout
    pub const MIN_TERMINAL_HEIGHT: u16 = 15;
}

/// Layout containing rectangles for all UI panels.
///
/// This struct is returned by [`calculate_layout`] and provides the
/// position and size of each panel in the TUI.
#[derive(Debug, Clone, Copy, Default)]
pub struct PanelLayout {
    /// Agent tree panel (left side, top section)
    /// Shows hierarchical agent navigation
    pub agent_tree: Rect,
    /// Agent output panel (center, top section)
    /// Shows streaming output from selected agent
    pub agent_output: Rect,
    /// Task list panel (right side, top section)
    /// Shows task status overview
    pub task_list: Rect,
    /// App logs panel (full width, middle section)
    /// Shows filterable application logs
    pub app_logs: Rect,
    /// Status bar (full width, bottom, 1 line)
    /// Shows summary info and help hints
    pub status_bar: Rect,
}

impl PanelLayout {
    /// Returns true if the layout represents a "too small" terminal.
    ///
    /// When the terminal is too small, the application should display
    /// a message asking the user to resize rather than rendering the
    /// full UI.
    #[must_use]
    pub const fn is_too_small(&self) -> bool {
        self.agent_output.width == 0 || self.agent_output.height == 0 || self.status_bar.width == 0
    }
}

/// Calculates the panel layout for the given terminal area.
///
/// This is the main layout function that divides the terminal into
/// the five panels according to the layout specification:
///
/// - Top section (~70% height): Agent Tree (20%), Agent Output (50%), Task List (30%)
/// - Middle section (~30% height): App Logs
/// - Bottom section (1 line): Status Bar
///
/// # Arguments
///
/// * `area` - The total available terminal area
///
/// # Returns
///
/// A [`PanelLayout`] with rectangles for each panel. If the terminal
/// is too small, some panels may have zero dimensions.
///
/// # Example
///
/// ```ignore
/// use ratatui::layout::Rect;
/// use villalobos::tui::layout::calculate_layout;
///
/// let area = Rect::new(0, 0, 120, 40);
/// let layout = calculate_layout(area);
///
/// // Use layout.agent_output to render agent output panel
/// // Use layout.task_list to render task list panel
/// // etc.
/// ```
pub fn calculate_layout(area: Rect) -> PanelLayout {
    // Handle degenerate cases
    if area.width < min_sizes::MIN_TERMINAL_WIDTH || area.height < min_sizes::MIN_TERMINAL_HEIGHT {
        return create_minimal_layout(area);
    }

    // First, split vertically into: top content, logs, status bar
    // Status bar is always 1 line
    // Top section gets 70% of remaining height, logs get 30%
    // Using Fill constraints ensures stable layout that doesn't shift based on content
    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(70),                             // Top section (70% of remaining)
            Constraint::Fill(30),                             // App logs (30% of remaining)
            Constraint::Length(min_sizes::STATUS_BAR_HEIGHT), // Status bar (1 line)
        ])
        .split(area);

    let top_section = vertical_chunks[0];
    let app_logs = vertical_chunks[1];
    let status_bar = vertical_chunks[2];

    // Split the top section horizontally: agent tree (20%), output (50%), tasks (30%)
    let horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20), // Agent tree
            Constraint::Percentage(50), // Agent output
            Constraint::Percentage(30), // Task list
        ])
        .split(top_section);

    let agent_tree = horizontal_chunks[0];
    let agent_output = horizontal_chunks[1];
    let task_list = horizontal_chunks[2];

    PanelLayout {
        agent_tree,
        agent_output,
        task_list,
        app_logs,
        status_bar,
    }
}

/// Creates a minimal layout for very small terminals.
///
/// When the terminal is too small for the full layout, this function
/// creates a simplified layout that shows at least the status bar
/// and as much content as possible.
fn create_minimal_layout(area: Rect) -> PanelLayout {
    if area.height < 2 || area.width < 10 {
        // Extremely small - just show status bar if possible
        return PanelLayout {
            agent_tree: Rect::default(),
            agent_output: Rect::default(),
            task_list: Rect::default(),
            app_logs: Rect::default(),
            status_bar: if area.height >= 1 {
                Rect::new(area.x, area.y, area.width, 1)
            } else {
                Rect::default()
            },
        };
    }

    // For small terminals, simplify to just output + status bar
    let content_height = area.height.saturating_sub(1);

    PanelLayout {
        agent_tree: Rect::default(),
        agent_output: Rect::new(area.x, area.y, area.width, content_height),
        task_list: Rect::default(),
        app_logs: Rect::default(),
        status_bar: Rect::new(area.x, area.y + content_height, area.width, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_layout_standard_terminal() {
        // Standard 120x40 terminal
        let area = Rect::new(0, 0, 120, 40);
        let layout = calculate_layout(area);

        // Verify all panels have non-zero dimensions
        assert!(layout.agent_tree.width > 0);
        assert!(layout.agent_tree.height > 0);
        assert!(layout.agent_output.width > 0);
        assert!(layout.agent_output.height > 0);
        assert!(layout.task_list.width > 0);
        assert!(layout.task_list.height > 0);
        assert!(layout.app_logs.width > 0);
        assert!(layout.app_logs.height > 0);
        assert_eq!(layout.status_bar.height, 1);
        assert_eq!(layout.status_bar.width, 120);

        // Verify horizontal proportions (approximately)
        let total_top_width =
            layout.agent_tree.width + layout.agent_output.width + layout.task_list.width;
        assert_eq!(total_top_width, 120);

        // Agent tree should be ~20% (24 columns)
        assert!(layout.agent_tree.width >= 20 && layout.agent_tree.width <= 30);
        // Agent output should be ~50% (60 columns)
        assert!(layout.agent_output.width >= 55 && layout.agent_output.width <= 65);
        // Task list should be ~30% (36 columns)
        assert!(layout.task_list.width >= 32 && layout.task_list.width <= 40);

        // Verify vertical layout
        assert_eq!(layout.status_bar.y + layout.status_bar.height, 40);
        assert!(layout.app_logs.height >= 3); // ~30% of 40 = 12, but with min constraint
    }

    #[test]
    fn test_calculate_layout_small_terminal() {
        // Small terminal that's still usable
        let area = Rect::new(0, 0, 80, 20);
        let layout = calculate_layout(area);

        // Should still have valid layout
        assert!(!layout.is_too_small());
        assert_eq!(layout.status_bar.height, 1);
        assert_eq!(layout.status_bar.width, 80);
    }

    #[test]
    fn test_calculate_layout_minimum_terminal() {
        // Below minimum size for full layout
        let area = Rect::new(0, 0, 60, 10);
        let layout = calculate_layout(area);

        // Should fall back to minimal layout (output + status bar only)
        // This is still usable, just simplified
        assert_eq!(layout.agent_tree.width, 0, "agent tree should be hidden");
        assert_eq!(layout.task_list.width, 0, "task list should be hidden");
        assert_eq!(layout.app_logs.height, 0, "app logs should be hidden");
        // But we should still have output and status bar
        assert!(layout.agent_output.width > 0, "output should be visible");
        assert_eq!(layout.status_bar.height, 1, "status bar should be visible");
    }

    #[test]
    fn test_calculate_layout_tiny_terminal() {
        // Extremely small terminal
        let area = Rect::new(0, 0, 5, 2);
        let layout = calculate_layout(area);

        // Should handle gracefully
        assert!(layout.is_too_small());
        // Status bar should still exist if possible
        assert_eq!(layout.status_bar.height, 1);
    }

    #[test]
    fn test_panel_layout_no_gaps() {
        let area = Rect::new(0, 0, 120, 40);
        let layout = calculate_layout(area);

        // Top panels should be adjacent
        assert_eq!(layout.agent_tree.x, 0);
        assert_eq!(
            layout.agent_output.x,
            layout.agent_tree.x + layout.agent_tree.width
        );
        assert_eq!(
            layout.task_list.x,
            layout.agent_output.x + layout.agent_output.width
        );

        // Top panels should have same y and height
        assert_eq!(layout.agent_tree.y, layout.agent_output.y);
        assert_eq!(layout.agent_output.y, layout.task_list.y);
        assert_eq!(layout.agent_tree.height, layout.agent_output.height);
        assert_eq!(layout.agent_output.height, layout.task_list.height);

        // App logs should be directly below top section
        assert_eq!(
            layout.app_logs.y,
            layout.agent_tree.y + layout.agent_tree.height
        );
        assert_eq!(layout.app_logs.x, 0);
        assert_eq!(layout.app_logs.width, 120);

        // Status bar should be at the bottom
        assert_eq!(
            layout.status_bar.y,
            layout.app_logs.y + layout.app_logs.height
        );
    }
}
