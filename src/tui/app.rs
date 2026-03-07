//! Main TUI application module.
//!
//! This module contains [`run_tui`] which manages the overall TUI lifecycle,
//! including terminal initialization, the main event loop, and rendering.

use std::io::{self, Stdout};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tui_logger::TuiWidgetState;

use super::events::{handle_key_event, handle_mouse_click, EventResult};
use super::layout::calculate_layout;
use super::state::{FocusedPanel, TuiState};
use super::widgets::{
    create_app_logs_state, render_agent_output, render_agent_tree, render_app_logs,
    render_help_overlay, render_status_bar, render_task_detail, render_task_list,
};
use crate::logging::LogEvent;

/// Frame rate target (60 FPS max)
const TARGET_FRAME_DURATION: Duration = Duration::from_millis(16);

/// Runs the TUI on a dedicated `std::thread`.
///
/// This is the main entry point for the TUI. It:
/// 1. Sets up a panic hook to restore the terminal on crash
/// 2. Initializes the terminal (raw mode, alternate screen)
/// 3. Runs the main event loop
/// 4. Restores the terminal on exit (normal, error, or panic)
///
/// # Thread Safety
///
/// This function is designed to run on a `std::thread` (not a `tokio` task)
/// to avoid blocking the async runtime. Communication with the main app
/// happens via the provided `event_rx` channel.
///
/// # Arguments
///
/// * `event_rx` - Channel receiver for [`LogEvent`]s from the main application
///
/// # Errors
///
/// Returns an error if terminal initialization or cleanup fails.
pub fn run_tui(event_rx: Receiver<LogEvent>) -> Result<()> {
    // Install panic hook to restore terminal on crash
    install_panic_hook();

    // Initialize terminal
    let mut terminal = init_terminal().context("Failed to initialize terminal")?;

    // Run the main event loop (capturing result)
    let result = run_event_loop(&mut terminal, event_rx);

    // Always restore terminal, regardless of how we exit
    restore_terminal(&mut terminal).context("Failed to restore terminal")?;

    result
}

/// Installs a panic hook that restores the terminal before the default panic handler runs.
///
/// This ensures that even if the application panics, the terminal will be left in a usable state.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |panic_info| {
        // Attempt to restore terminal - ignore errors since we're already panicking
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);

        // Call the original panic hook
        original_hook(panic_info);
    }));
}

/// Initializes the terminal for TUI rendering.
///
/// Enables raw mode, mouse capture, and switches to the alternate screen.
fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("Failed to enter alternate screen and enable mouse capture")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("Failed to create terminal")
}

/// Restores the terminal to its original state.
///
/// Disables raw mode, mouse capture, and leaves the alternate screen.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .context("Failed to disable mouse capture and leave alternate screen")?;
    terminal.show_cursor().context("Failed to show cursor")?;
    Ok(())
}

/// Main event loop for the TUI.
///
/// Polls for terminal events, processes `LogEvent`s from the channel,
/// and renders frames at up to 60 FPS.
///
/// # Event Processing
///
/// The loop follows this structure on each iteration:
/// 1. **Process `LogEvent`s** - Non-blocking batch processing of all pending events
/// 2. **Poll for keyboard events** - With a short timeout to stay responsive
/// 3. **Render frame** - Only when enough time has passed since the last frame
///
/// # Performance
///
/// - Frame rate is capped at 60 FPS to avoid excessive CPU usage
/// - `LogEvent`s are batch processed for efficiency
/// - Terminal resize events trigger immediate redraw
const MAX_EVENTS_PER_FRAME: usize = 100;

#[allow(clippy::needless_pass_by_value)] // Receiver ownership transfer is intentional
fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    event_rx: Receiver<LogEvent>,
) -> Result<()> {
    // Initialize TUI state
    let mut state = TuiState::new();

    // Initialize tui-logger widget state for app logs panel
    let logger_state = create_app_logs_state();

    // Frame timing
    let mut last_frame = Instant::now();

    loop {
        // =====================================================================
        // 1. Process all available LogEvents (non-blocking, batched)
        // =====================================================================
        let mut events_processed = 0;
        while let Ok(log_event) = event_rx.try_recv() {
            state.handle_event(log_event);
            events_processed += 1;

            // Limit events per frame to prevent blocking on burst
            if events_processed >= MAX_EVENTS_PER_FRAME {
                break;
            }
        }

        // =====================================================================
        // 2. Poll for keyboard events (with short timeout for responsiveness)
        // =====================================================================
        // Use a short poll timeout to stay responsive while allowing batching
        let poll_timeout = Duration::from_millis(16); // ~60 FPS budget

        if event::poll(poll_timeout).context("Failed to poll for events")? {
            match event::read().context("Failed to read event")? {
                CrosstermEvent::Key(key) => {
                    // Only process key press events (not release or repeat)
                    if key.kind == KeyEventKind::Press {
                        // Get visible height from the current terminal size for scroll calculations
                        let visible_height = terminal
                            .size()
                            .map(|r| r.height.saturating_sub(2)) // Account for borders
                            .unwrap_or(20);

                        // Handle key event through the event handler
                        match handle_key_event(&mut state, key, visible_height) {
                            EventResult::Quit => break,
                            EventResult::Continue => {}
                        }
                    }
                }
                CrosstermEvent::Resize(_width, _height) => {
                    // Terminal will automatically handle resize on next draw
                    // Force a redraw by resetting frame timing
                    last_frame = Instant::now()
                        .checked_sub(TARGET_FRAME_DURATION)
                        .unwrap_or(last_frame);
                }
                CrosstermEvent::Mouse(mouse_event) => {
                    // Only handle mouse button down events for panel switching
                    if matches!(mouse_event.kind, event::MouseEventKind::Down(_)) {
                        // Get current terminal size to calculate layout
                        if let Ok(size) = terminal.size() {
                            let area = Rect::new(0, 0, size.width, size.height);
                            let layout = calculate_layout(area);
                            handle_mouse_click(&mut state, mouse_event, &layout);
                        }
                    }
                }
                // Ignore other events (FocusGained, FocusLost, Paste)
                _ => {}
            }
        }

        // =====================================================================
        // 3. Render frame (respecting frame rate limit)
        // =====================================================================
        if last_frame.elapsed() >= TARGET_FRAME_DURATION {
            terminal
                .draw(|frame| {
                    render_ui_frame(frame, &mut state, &logger_state);
                })
                .context("Failed to draw frame")?;
            last_frame = Instant::now();
        }
    }

    Ok(())
}

/// Renders a complete UI frame with all widgets.
///
/// This function orchestrates rendering of all panels:
/// - Agent tree (left)
/// - Agent output (center)
/// - Task list (right)
/// - App logs (bottom)
/// - Status bar (very bottom)
/// - Help overlay (if visible)
fn render_ui_frame(frame: &mut Frame, state: &mut TuiState, logger_state: &TuiWidgetState) {
    let area = frame.area();

    // Calculate layout for all panels
    let layout = calculate_layout(area);

    // Check if terminal is too small
    if layout.is_too_small() {
        render_too_small_message(frame, area);
        return;
    }

    // Determine focus state for each panel
    let agent_tree_focused = state.current_focus == FocusedPanel::AgentTree;
    let agent_output_focused = state.current_focus == FocusedPanel::AgentOutput;
    let task_list_focused = state.current_focus == FocusedPanel::TaskList;
    let app_logs_focused = state.current_focus == FocusedPanel::AppLogs;

    // Render left panel (agent tree)
    render_agent_tree(frame, layout.agent_tree, state, agent_tree_focused);

    // Render middle panel: task detail when task is selected in focused task list,
    // otherwise show agent output
    if let Some(task) = state.selected_task() {
        // Clone the task data to avoid borrow checker issues with state
        let task_clone = task.clone();
        render_task_detail(frame, layout.agent_output, &task_clone, task_list_focused);
    } else {
        render_agent_output(frame, layout.agent_output, state, agent_output_focused);
    }

    // Render right panel (task list)
    render_task_list(frame, layout.task_list, state, task_list_focused);
    render_app_logs(
        frame,
        layout.app_logs,
        state,
        logger_state,
        app_logs_focused,
    );
    render_status_bar(frame, layout.status_bar, state);

    // Render help overlay last (on top of everything)
    if state.help_visible {
        render_help_overlay(frame, area);
    }
}

/// Renders a message when the terminal is too small for the UI.
fn render_too_small_message(frame: &mut Frame, area: Rect) {
    let message = "Terminal too small\n\nPlease resize to at least 80x15";

    let block = Block::default()
        .title(" Villalobos TUI ")
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded);

    let paragraph = Paragraph::new(message)
        .block(block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow));

    frame.render_widget(paragraph, area);
}
