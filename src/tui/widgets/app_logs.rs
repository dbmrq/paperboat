//! Application logs widget module.
//!
//! This module contains the [`render_app_logs`] function which renders
//! application logs using tui-logger widgets with log level filtering.
//!
//! # Key Controls (when focused)
//!
//! - `↑`/`↓` - Scroll logs up/down
//! - `PgUp`/`PgDn` - Scroll logs by page

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_logger::{LevelFilter, TuiLoggerLevelOutput, TuiLoggerWidget, TuiWidgetState};

use super::super::state::{FocusedPanel, TuiState};

/// Colors used for the app logs widget.
pub mod colors {
    use ratatui::style::Color;

    /// Border color when focused
    pub const FOCUSED_BORDER: Color = Color::Cyan;
    /// Border color when not focused
    pub const UNFOCUSED_BORDER: Color = Color::Gray;
    /// Color for error level logs
    pub const ERROR: Color = Color::Red;
    /// Color for warning level logs
    pub const WARN: Color = Color::Yellow;
    /// Color for info level logs
    pub const INFO: Color = Color::Cyan;
    /// Color for debug level logs
    pub const DEBUG: Color = Color::Green;
    /// Color for trace level logs
    pub const TRACE: Color = Color::Magenta;
}

/// Renders the application logs panel using tui-logger.
///
/// This function renders application logs using [`TuiLoggerWidget`] which provides:
/// - Log level filtering per target
/// - Scrollback through log history
/// - Timestamp, level, target, and message display
///
/// Note: Uses `TuiLoggerWidget` directly instead of `TuiLoggerSmartWidget` to avoid
/// the built-in speed indicator (e.g., `[log=0.3/s]`) that appears in the title.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render to
/// * `area` - The rectangular area to render the widget in
/// * `state` - The TUI state (contains logger state for filtering and navigation)
/// * `focused` - Whether this panel currently has keyboard focus
///
/// # Example
///
/// ```ignore
/// use ratatui::Frame;
/// use ratatui::layout::Rect;
/// use paperboat::tui::widgets::render_app_logs;
/// use paperboat::tui::TuiState;
///
/// fn render(frame: &mut Frame, area: Rect, state: &TuiState) {
///     let focused = state.current_focus == FocusedPanel::AppLogs;
///     render_app_logs(frame, area, state, focused);
/// }
/// ```
pub fn render_app_logs(frame: &mut Frame, area: Rect, state: &TuiState, focused: bool) {
    let is_focused = focused || state.current_focus == FocusedPanel::AppLogs;

    // Create the border block with focus-dependent styling
    let border_color = if is_focused {
        colors::FOCUSED_BORDER
    } else {
        colors::UNFOCUSED_BORDER
    };

    // Use TuiLoggerWidget directly instead of TuiLoggerSmartWidget to avoid the
    // built-in speed indicator (e.g., [log=0.3/s]) in the title.
    let log_block = Block::default()
        .title(" App Logs ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let logger_widget = TuiLoggerWidget::default()
        .block(log_block)
        .style_error(Style::default().fg(colors::ERROR))
        .style_warn(Style::default().fg(colors::WARN))
        .style_info(Style::default().fg(colors::INFO))
        .style_debug(Style::default().fg(colors::DEBUG))
        .style_trace(Style::default().fg(colors::TRACE))
        .output_separator(':')
        .output_timestamp(Some("%H:%M:%S".to_string()))
        .output_level(Some(TuiLoggerLevelOutput::Abbreviated))
        .output_target(true)
        .output_file(false)
        .output_line(false)
        .state(&state.logger_state);

    frame.render_widget(logger_widget, area);
}

/// Checks if the app logs panel is currently focused.
#[must_use]
#[allow(dead_code)] // Reserved for future per-panel event routing
fn is_app_logs_focused(state: &TuiState) -> bool {
    state.current_focus == FocusedPanel::AppLogs
}

/// Creates a new [`TuiWidgetState`] with default configuration for the app logs widget.
///
/// This initializes the tui-logger widget state with:
/// - Default display level set to Info
///
/// # Returns
///
/// A new [`TuiWidgetState`] configured for the app logs widget.
#[must_use]
pub fn create_app_logs_state() -> TuiWidgetState {
    TuiWidgetState::new().set_default_display_level(LevelFilter::Info)
}
