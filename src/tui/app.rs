//! Main TUI application module.
//!
//! This module contains [`run_tui`] which manages the overall TUI lifecycle,
//! including terminal initialization, the main event loop, and rendering.

use std::io::{self, Stdout};
use std::sync::mpsc::{Receiver, SyncSender};
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

use super::events::{
    handle_key_event, handle_mouse_click, handle_mouse_scroll, EventResult, ScrollDirection,
};
use super::layout::calculate_layout;
use super::state::{FocusedPanel, ModelConfigUpdate, TuiState};
use super::widgets::BackendSelectionState;
use super::widgets::{
    render_agent_output, render_agent_tree, render_app_logs, render_backend_selection_popup,
    render_help_overlay, render_settings_overlay, render_splash_screen, render_status_bar,
    render_task_detail, render_task_list,
};
use crate::backend::BackendKind;
use crate::logging::LogEvent;
use crate::models::ModelConfig;

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
#[allow(dead_code)] // Public API for simpler TUI usage without config
pub fn run_tui(event_rx: Receiver<LogEvent>) -> Result<()> {
    // Install panic hook to restore terminal on crash
    install_panic_hook();

    // Initialize terminal
    let mut terminal = init_terminal().context("Failed to initialize terminal")?;

    // Run the main event loop (capturing result)
    let result = run_event_loop(&mut terminal, event_rx, None, None);

    // Always restore terminal, regardless of how we exit
    restore_terminal(&mut terminal).context("Failed to restore terminal")?;

    result
}

/// Runs the TUI with model configuration support on a dedicated `std::thread`.
///
/// This is an enhanced version of [`run_tui`] that includes:
/// - Initial model configuration for display
/// - A channel to send model configuration updates back to the App
///
/// # Thread Safety
///
/// This function is designed to run on a `std::thread` (not a `tokio` task)
/// to avoid blocking the async runtime. Communication with the main app
/// happens via the provided channels.
///
/// # Arguments
///
/// * `event_rx` - Channel receiver for [`LogEvent`]s from the main application
/// * `model_config` - Initial model configuration to display
/// * `config_tx` - Channel sender for [`ModelConfigUpdate`]s to the main application
///
/// # Errors
///
/// Returns an error if terminal initialization or cleanup fails.
#[allow(dead_code)] // Public API for TUI with explicit config
pub fn run_tui_with_config(
    event_rx: Receiver<LogEvent>,
    model_config: ModelConfig,
    config_tx: SyncSender<ModelConfigUpdate>,
) -> Result<()> {
    // Install panic hook to restore terminal on crash
    install_panic_hook();

    // Initialize terminal
    let mut terminal = init_terminal().context("Failed to initialize terminal")?;

    // Run the main event loop with config support
    let result = run_event_loop(&mut terminal, event_rx, Some(model_config), Some(config_tx));

    // Always restore terminal, regardless of how we exit
    restore_terminal(&mut terminal).context("Failed to restore terminal")?;

    result
}

/// Runs the TUI with bidirectional channel support on a dedicated `std::thread`.
///
/// This is the most flexible version that uses a `TuiThreadChannels`
/// struct containing all necessary channels for communication with the main app.
///
/// The TUI will:
/// 1. Start immediately and show the splash screen
/// 2. Check for available backends (if multiple, show selection popup over splash)
/// 3. Wait for the initial model configuration via `initial_config_rx`
/// 4. Display model config and handle updates via `config_update_tx`
///
/// # Arguments
///
/// * `channels` - The `TuiThreadChannels` containing all communication channels
///
/// # Errors
///
/// Returns an error if terminal initialization or cleanup fails.
pub fn run_tui_with_channels(channels: super::TuiThreadChannels) -> Result<()> {
    // Install panic hook to restore terminal on crash
    install_panic_hook();

    // Initialize terminal
    let mut terminal = init_terminal().context("Failed to initialize terminal")?;

    // Run the main event loop with all channels
    let result = run_event_loop_with_backend_selection(
        &mut terminal,
        channels.event_rx,
        channels.initial_config_rx,
        channels.config_update_tx,
        channels.available_backends_rx,
        channels.selected_backend_tx,
    );

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
    model_config: Option<ModelConfig>,
    config_tx: Option<SyncSender<ModelConfigUpdate>>,
) -> Result<()> {
    // Initialize TUI state (with model config if provided)
    // TuiState now includes logger_state for the app logs panel
    let mut state = match model_config {
        Some(config) => TuiState::with_model_config(config),
        None => TuiState::new(),
    };

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
                        // Calculate layout from terminal size for accurate panel dimensions
                        let layout = terminal
                            .size()
                            .map(|r| calculate_layout(Rect::new(0, 0, r.width, r.height)))
                            .unwrap_or_default();

                        // Handle key event through the event handler
                        match handle_key_event(&mut state, key, &layout) {
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
                    // Get current terminal size to calculate layout
                    if let Ok(size) = terminal.size() {
                        let area = Rect::new(0, 0, size.width, size.height);
                        let layout = calculate_layout(area);

                        match mouse_event.kind {
                            // Handle mouse clicks for panel focus switching
                            event::MouseEventKind::Down(_) => {
                                handle_mouse_click(&mut state, mouse_event, &layout);
                            }
                            // Handle mouse wheel scroll up
                            event::MouseEventKind::ScrollUp => {
                                handle_mouse_scroll(
                                    &mut state,
                                    mouse_event,
                                    &layout,
                                    ScrollDirection::Up,
                                );
                            }
                            // Handle mouse wheel scroll down
                            event::MouseEventKind::ScrollDown => {
                                handle_mouse_scroll(
                                    &mut state,
                                    mouse_event,
                                    &layout,
                                    ScrollDirection::Down,
                                );
                            }
                            // Ignore other mouse events (drag, move, etc.)
                            _ => {}
                        }
                    }
                }
                // Ignore other events (FocusGained, FocusLost, Paste)
                _ => {}
            }
        }

        // =====================================================================
        // 3. Send pending config updates to the App (if channel is available)
        // =====================================================================
        if let Some(ref tx) = config_tx {
            if let Some(update) = state.take_pending_config_update() {
                // Try to send the update; if channel is full or disconnected, log and continue
                if let Err(e) = tx.try_send(update) {
                    tracing::warn!("Failed to send model config update to App: {}", e);
                }
            }
        }

        // =====================================================================
        // 4. Render frame (respecting frame rate limit)
        // =====================================================================
        if last_frame.elapsed() >= TARGET_FRAME_DURATION {
            // Increment animation frame counter (wrapping to prevent overflow)
            state.animation_frame = state.animation_frame.wrapping_add(1);

            terminal
                .draw(|frame| {
                    render_ui_frame(frame, &mut state);
                })
                .context("Failed to draw frame")?;
            last_frame = Instant::now();
        }
    }

    Ok(())
}

/// Main event loop with backend selection support.
///
/// This version handles the complete startup flow:
/// 1. Shows splash screen immediately
/// 2. Receives available backends and shows selection popup if multiple
/// 3. Sends selected backend back to App
/// 4. Receives model config and continues normal operation
#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_arguments)]
fn run_event_loop_with_backend_selection(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    event_rx: Receiver<LogEvent>,
    initial_config_rx: Receiver<ModelConfig>,
    config_tx: SyncSender<ModelConfigUpdate>,
    available_backends_rx: Receiver<Vec<BackendKind>>,
    selected_backend_tx: SyncSender<BackendKind>,
) -> Result<()> {
    // Initialize TUI state with splash visible
    let mut state = TuiState::new();

    // Track whether we've received model config yet
    let mut model_config_received = false;

    // Frame timing
    let mut last_frame = Instant::now();

    loop {
        // =====================================================================
        // 1. Check for available backends (non-blocking)
        // =====================================================================
        // Keep checking until we receive backends, even if splash is dismissed.
        // This prevents a race condition where backends arrive right as splash
        // auto-dismisses after 5 seconds.
        if !state.backends_received && !state.backend_selection_state.visible {
            if let Ok(backends) = available_backends_rx.try_recv() {
                state.backends_received = true;
                if backends.len() > 1 {
                    // Multiple backends available - show selection popup
                    // Also show splash again if it was dismissed (to show popup on top)
                    if !state.splash_visible {
                        state.splash_visible = true;
                    }
                    state.backend_selection_state = BackendSelectionState::with_backends(backends);
                } else if let Some(&backend) = backends.first() {
                    // Single backend - send it back immediately
                    let _ = selected_backend_tx.try_send(backend);
                }
            }
        }

        // =====================================================================
        // 2. Check for model config (non-blocking, keeps splash running)
        // =====================================================================
        if !model_config_received {
            if let Ok(config) = initial_config_rx.try_recv() {
                let available_tiers: Vec<_> = config.available_tiers.iter().copied().collect();
                state.model_config = config;
                state.available_tiers = available_tiers;
                model_config_received = true;
            }
        }

        // =====================================================================
        // 3. Process all available LogEvents (non-blocking, batched)
        // =====================================================================
        let mut events_processed = 0;
        while let Ok(log_event) = event_rx.try_recv() {
            state.handle_event(log_event);
            events_processed += 1;

            if events_processed >= MAX_EVENTS_PER_FRAME {
                break;
            }
        }

        // =====================================================================
        // 4. Poll for keyboard events (with short timeout for responsiveness)
        // =====================================================================
        let poll_timeout = Duration::from_millis(16);

        if event::poll(poll_timeout).context("Failed to poll for events")? {
            match event::read().context("Failed to read event")? {
                CrosstermEvent::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        // Handle backend selection popup keys
                        if state.backend_selection_state.visible {
                            use crossterm::event::KeyCode;
                            match key.code {
                                KeyCode::Up | KeyCode::Char('k') => {
                                    state.backend_selection_state.select_previous();
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    state.backend_selection_state.select_next();
                                }
                                KeyCode::Enter => {
                                    if let Some(backend) =
                                        state.backend_selection_state.confirm_selection()
                                    {
                                        let _ = selected_backend_tx.try_send(backend);
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            // Normal key handling
                            let layout = terminal
                                .size()
                                .map(|r| calculate_layout(Rect::new(0, 0, r.width, r.height)))
                                .unwrap_or_default();

                            match handle_key_event(&mut state, key, &layout) {
                                EventResult::Quit => break,
                                EventResult::Continue => {}
                            }
                        }
                    }
                }
                CrosstermEvent::Resize(_width, _height) => {
                    last_frame = Instant::now()
                        .checked_sub(TARGET_FRAME_DURATION)
                        .unwrap_or(last_frame);
                }
                CrosstermEvent::Mouse(mouse_event) => {
                    if !state.backend_selection_state.visible {
                        if let Ok(size) = terminal.size() {
                            let area = Rect::new(0, 0, size.width, size.height);
                            let layout = calculate_layout(area);

                            match mouse_event.kind {
                                event::MouseEventKind::Down(_) => {
                                    handle_mouse_click(&mut state, mouse_event, &layout);
                                }
                                event::MouseEventKind::ScrollUp => {
                                    handle_mouse_scroll(
                                        &mut state,
                                        mouse_event,
                                        &layout,
                                        ScrollDirection::Up,
                                    );
                                }
                                event::MouseEventKind::ScrollDown => {
                                    handle_mouse_scroll(
                                        &mut state,
                                        mouse_event,
                                        &layout,
                                        ScrollDirection::Down,
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // =====================================================================
        // 5. Send pending config updates to the App
        // =====================================================================
        if let Some(update) = state.take_pending_config_update() {
            if let Err(e) = config_tx.try_send(update) {
                tracing::warn!("Failed to send model config update to App: {}", e);
            }
        }

        // =====================================================================
        // 6. Render frame (respecting frame rate limit)
        // =====================================================================
        if last_frame.elapsed() >= TARGET_FRAME_DURATION {
            state.animation_frame = state.animation_frame.wrapping_add(1);

            terminal
                .draw(|frame| {
                    render_ui_frame(frame, &mut state);
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
/// - Splash screen (if visible, replaces everything)
/// - Agent tree (left)
/// - Agent output (center)
/// - Task list (right)
/// - App logs (bottom)
/// - Status bar (very bottom)
/// - Help overlay (if visible)
fn render_ui_frame(frame: &mut Frame, state: &mut TuiState) {
    let area = frame.area();

    // Show splash screen if visible (replaces the entire UI)
    // Auto-dismiss after 5 seconds (300 frames at 60fps), but only if backends have been received
    if state.splash_visible {
        // Always render the splash screen first
        render_splash_screen(frame, area, state.animation_frame);

        // If backend selection popup is visible, render it on top of splash
        if state.backend_selection_state.visible {
            render_backend_selection_popup(frame, area, &state.backend_selection_state);
            return;
        }

        // Auto-dismiss splash after 5 seconds, but ONLY if backends have been received.
        // This prevents the race condition where splash dismisses before we can show
        // the backend selection popup.
        if state.animation_frame >= 300 && state.backends_received {
            state.dismiss_splash();
        } else {
            return;
        }
    }

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
        render_task_detail(
            frame,
            layout.agent_output,
            state,
            &task_clone,
            task_list_focused,
        );
    } else {
        render_agent_output(frame, layout.agent_output, state, agent_output_focused);
    }

    // Render right panel (task list)
    render_task_list(frame, layout.task_list, state, task_list_focused);
    render_app_logs(frame, layout.app_logs, state, app_logs_focused);
    render_status_bar(frame, layout.status_bar, state);

    // Render overlays last (on top of everything)
    if state.help_visible {
        render_help_overlay(frame, area);
    }

    if state.settings_visible {
        render_settings_overlay(frame, area, state);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_ui_to_string(state: &mut TuiState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_ui_frame(frame, state))
            .expect("ui frame should render");
        format!("{}", terminal.backend())
    }

    #[test]
    fn test_render_ui_frame_keeps_splash_visible_until_backends_are_received() {
        let mut state = TuiState::new();
        state.animation_frame = 300;
        state.backends_received = false;

        render_ui_to_string(&mut state, 120, 40);

        assert!(state.splash_visible);

        state.backends_received = true;
        render_ui_to_string(&mut state, 120, 40);

        assert!(!state.splash_visible);
    }

    #[test]
    fn test_render_ui_frame_backend_selection_takes_precedence_during_splash() {
        let mut state = TuiState::new();
        state.help_visible = true;
        state.settings_visible = true;
        state.backend_selection_state =
            BackendSelectionState::with_backends(vec![BackendKind::Auggie, BackendKind::Cursor]);

        let rendered = render_ui_to_string(&mut state, 120, 40);

        assert!(rendered.contains("Select Backend"));
        assert!(!rendered.contains("Keyboard Shortcuts"));
        assert!(!rendered.contains("Model Settings"));
    }

    #[test]
    fn test_render_ui_frame_settings_overlay_renders_above_help_overlay() {
        let mut state = TuiState::new();
        state.splash_visible = false;
        state.help_visible = true;
        state.settings_visible = true;

        let rendered = render_ui_to_string(&mut state, 120, 40);

        assert!(rendered.contains("Model Settings"));
        assert!(!rendered.contains("Keyboard Shortcuts"));
    }
}
