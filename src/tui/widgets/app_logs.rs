//! Application logs widget module.
//!
//! This module contains the [`render_app_logs`] function which renders
//! application logs using tui-logger's smart widget with built-in filtering
//! and keyboard controls.
//!
//! # Key Controls (when focused)
//!
//! - `h` - Toggle target selector visibility
//! - `f` - Toggle focus on selected target only
//! - `↑`/`↓` - Select target in selector
//! - `←`/`→` - Adjust display level filter
//! - `+`/`-` - Adjust capture level filter
//! - `PgUp`/`PgDn` - Scroll through log history
//! - `Esc` - Exit scroll mode
//! - `Space` - Toggle hidden targets

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::Frame;
use tui_logger::{LevelFilter, TuiLoggerLevelOutput, TuiLoggerSmartWidget, TuiWidgetState};

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
/// This function renders application logs using [`TuiLoggerSmartWidget`] which provides:
/// - Built-in target selector (toggled with `h`)
/// - Log level filtering per target
/// - Scrollback through log history
/// - Timestamp, level, target, and message display
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render to
/// * `area` - The rectangular area to render the widget in
/// * `state` - The TUI state (used for focus checking)
/// * `logger_state` - The tui-logger widget state for filtering and navigation
/// * `focused` - Whether this panel currently has keyboard focus
///
/// # Example
///
/// ```ignore
/// use ratatui::Frame;
/// use ratatui::layout::Rect;
/// use paperboat::tui::widgets::render_app_logs;
/// use paperboat::tui::TuiState;
/// use tui_logger::TuiWidgetState;
///
/// fn render(frame: &mut Frame, area: Rect, state: &TuiState, logger_state: &TuiWidgetState) {
///     let focused = state.current_focus == FocusedPanel::AppLogs;
///     render_app_logs(frame, area, state, logger_state, focused);
/// }
/// ```
pub fn render_app_logs(
    frame: &mut Frame,
    area: Rect,
    state: &TuiState,
    logger_state: &TuiWidgetState,
    focused: bool,
) {
    let is_focused = focused || state.current_focus == FocusedPanel::AppLogs;

    // Create the border block with focus-dependent styling
    let border_color = if is_focused {
        colors::FOCUSED_BORDER
    } else {
        colors::UNFOCUSED_BORDER
    };

    // Build the TuiLoggerSmartWidget with styling and configuration
    let logger_widget = TuiLoggerSmartWidget::default()
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
        .title_log(" App Logs ")
        .title_target(" Targets ")
        .border_style(Style::default().fg(border_color))
        .state(logger_state);

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
/// - Target selector hidden by default (toggle with 'h')
///
/// # Returns
///
/// A new [`TuiWidgetState`] configured for the app logs widget.
#[must_use]
pub fn create_app_logs_state() -> TuiWidgetState {
    let state = TuiWidgetState::new().set_default_display_level(LevelFilter::Info);
    // Hide the target selector by default - it can be toggled with 'h'
    // This gives more space to the log messages
    state.transition(tui_logger::TuiWidgetEvent::HideKey);
    state
}
