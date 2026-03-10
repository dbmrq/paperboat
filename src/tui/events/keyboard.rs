//! Keyboard event handling.
//!
//! This module handles keyboard events and routes them to the appropriate
//! handlers based on focus state.

use crossterm::event::{
    KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent,
    KeyModifiers as CrosstermKeyModifiers,
};

use crate::tui::layout::PanelLayout;
use crate::tui::state::{FocusedPanel, TuiState};
use crate::tui::widgets::{handle_agent_output_key, handle_agent_tree_key, handle_task_detail_key};

use super::EventResult;

/// Handles a keyboard event and updates TUI state accordingly.
///
/// This function processes keyboard input in two phases:
///
/// 1. **Global keys** - Handled regardless of panel focus
/// 2. **Panel-specific keys** - Routed to focused panel's handler
///
/// # Global Keys
///
/// | Key | Action |
/// |-----|--------|
/// | `Tab` | Cycle focus: `AgentTree` → `AgentOutput` → `TaskList` → `AppLogs` → `AgentTree` |
/// | `Shift+Tab` | Cycle focus in reverse order |
/// | `q` | Quit the TUI |
/// | `?` | Toggle help overlay |
/// | `Esc` | Close help overlay if visible |
pub fn handle_key_event(
    state: &mut TuiState,
    key: CrosstermKeyEvent,
    layout: &PanelLayout,
) -> EventResult {
    // Phase 1: Handle global keys
    if let Some(result) = handle_global_key(state, &key) {
        return result;
    }

    // Phase 2: Route to panel-specific handler based on current focus
    handle_panel_key(state, &key, layout);

    EventResult::Continue
}

/// Handles global keys that work regardless of panel focus.
///
/// Returns `Some(EventResult)` if the key was handled, `None` if not a global key.
#[allow(clippy::missing_const_for_fn)] // Uses non-const match expressions
fn handle_global_key(state: &mut TuiState, key: &CrosstermKeyEvent) -> Option<EventResult> {
    // Handle splash screen dismissal first (any key dismisses it)
    if state.splash_visible {
        state.dismiss_splash();
        return Some(EventResult::Continue);
    }

    // Handle settings overlay keys first (modal)
    if state.settings_visible {
        return handle_settings_key(state, key);
    }

    match key.code {
        // Tab: cycle focus forward (without Shift)
        CrosstermKeyCode::Tab if !key.modifiers.contains(CrosstermKeyModifiers::SHIFT) => {
            state.cycle_focus();
            Some(EventResult::Continue)
        }

        // BackTab (Shift+Tab on most terminals): cycle focus backward
        CrosstermKeyCode::BackTab => {
            state.cycle_focus_reverse();
            Some(EventResult::Continue)
        }

        // Tab with Shift modifier (some terminals): cycle focus backward
        CrosstermKeyCode::Tab if key.modifiers.contains(CrosstermKeyModifiers::SHIFT) => {
            state.cycle_focus_reverse();
            Some(EventResult::Continue)
        }

        // q: quit
        CrosstermKeyCode::Char('q') => Some(EventResult::Quit),

        // Ctrl+C: quit
        CrosstermKeyCode::Char('c') if key.modifiers.contains(CrosstermKeyModifiers::CONTROL) => {
            Some(EventResult::Quit)
        }

        // Esc: quit (or close help/settings overlay if visible)
        CrosstermKeyCode::Esc => {
            if state.help_visible {
                state.help_visible = false;
                Some(EventResult::Continue)
            } else {
                Some(EventResult::Quit)
            }
        }

        // ?: toggle help overlay
        CrosstermKeyCode::Char('?') => {
            state.toggle_help();
            Some(EventResult::Continue)
        }

        // s: toggle settings overlay
        CrosstermKeyCode::Char('s') => {
            state.toggle_settings();
            Some(EventResult::Continue)
        }

        _ => None,
    }
}

/// Handles keyboard input when the settings overlay is visible.
///
/// The settings overlay is modal - all keys are captured here.
#[allow(clippy::unnecessary_wraps)] // Returns Option for consistency with other key handlers
fn handle_settings_key(state: &mut TuiState, key: &CrosstermKeyEvent) -> Option<EventResult> {
    let available_count = state.available_tiers.len();

    match key.code {
        // Esc: close settings
        CrosstermKeyCode::Esc => {
            state.settings_visible = false;
            // Discard pending changes on close
            state.settings_state.clear_pending();
            Some(EventResult::Continue)
        }

        // Tab: switch between agent type tabs
        CrosstermKeyCode::Tab if !key.modifiers.contains(CrosstermKeyModifiers::SHIFT) => {
            state.settings_state.next_agent_type();
            Some(EventResult::Continue)
        }

        // Shift+Tab or BackTab: switch tabs in reverse
        CrosstermKeyCode::BackTab | CrosstermKeyCode::Tab
            if key.modifiers.contains(CrosstermKeyModifiers::SHIFT)
                || key.code == CrosstermKeyCode::BackTab =>
        {
            state.settings_state.prev_agent_type();
            Some(EventResult::Continue)
        }

        // Left: switch to previous agent type tab
        CrosstermKeyCode::Left => {
            state.settings_state.prev_agent_type();
            Some(EventResult::Continue)
        }

        // Right: switch to next agent type tab
        CrosstermKeyCode::Right => {
            state.settings_state.next_agent_type();
            Some(EventResult::Continue)
        }

        // Up: navigate to previous model
        CrosstermKeyCode::Up => {
            state.settings_state.select_previous_model(available_count);
            Some(EventResult::Continue)
        }

        // Down: navigate to next model
        CrosstermKeyCode::Down => {
            state.settings_state.select_next_model(available_count);
            Some(EventResult::Continue)
        }

        // Enter: select the highlighted model tier
        CrosstermKeyCode::Enter => {
            if let Some(&tier) = state
                .available_tiers
                .get(state.settings_state.selected_model_index)
            {
                state.settings_state.set_pending_model(tier);
            }
            Some(EventResult::Continue)
        }

        // s: apply changes and close settings (same key that opens it)
        CrosstermKeyCode::Char('s') => {
            // Apply any pending changes before closing
            state.apply_settings_changes();
            state.settings_visible = false;
            Some(EventResult::Continue)
        }

        // Explicitly handle 'q' in settings to not quit (same as default but documented)
        // Fall through to default for all other keys
        #[allow(clippy::match_same_arms)]
        CrosstermKeyCode::Char('q') => Some(EventResult::Continue),

        _ => Some(EventResult::Continue),
    }
}

/// Routes key events to the appropriate panel handler based on current focus.
fn handle_panel_key(state: &mut TuiState, key: &CrosstermKeyEvent, layout: &PanelLayout) {
    // If help or settings is visible, don't process panel keys
    if state.help_visible || state.settings_visible {
        return;
    }

    match state.current_focus {
        FocusedPanel::AgentTree => handle_agent_tree_panel_key(state, key),
        FocusedPanel::AgentOutput => {
            handle_agent_output_panel_key(
                state,
                key,
                layout.agent_output.height,
                layout.agent_output.width,
            );
        }
        FocusedPanel::TaskList => {
            // Pass both task list height and agent output height (where task detail is shown)
            handle_task_list_panel_key(
                state,
                key,
                layout.task_list.height,
                layout.agent_output.height,
            );
        }
        FocusedPanel::AppLogs => handle_app_logs_panel_key(state, key),
    }
}

// ============================================================================
// Panel-Specific Key Handlers
// ============================================================================

/// Handles keys for the Agent Tree panel.
///
/// Keys:
/// - `↑`/`↓`: Navigate agents in tree
/// - `←`/`→`: Collapse/expand tree nodes
/// - `Enter`: Select agent for detail view (toggles expand/collapse)
/// - `f`: Toggle auto-follow mode
fn handle_agent_tree_panel_key(state: &mut TuiState, key: &CrosstermKeyEvent) {
    match key.code {
        CrosstermKeyCode::Char('f') => {
            state.toggle_auto_follow();
        }
        // Delegate tree navigation to the widget handler
        _ => {
            handle_agent_tree_key(state, key.code);
        }
    }
}

/// Handles keys for the Agent Output panel.
///
/// Keys:
/// - `PgUp`/`PgDn`: Scroll output history by page
/// - `Home`/`End`: Jump to top/bottom of output
/// - `↑`/`↓` or `k`/`j`: Scroll by single line
fn handle_agent_output_panel_key(
    state: &mut TuiState,
    key: &CrosstermKeyEvent,
    visible_height: u16,
    visible_width: u16,
) {
    // Delegate to the widget handler which handles all scrolling
    handle_agent_output_key(state, key.code, visible_height, visible_width);
}

/// Handles keys for the Task List panel.
///
/// Keys:
/// - `↑`/`↓` or `j`/`k`: Navigate tasks in the task list
/// - `PgUp`/`PgDn`: Scroll task detail (when a task is selected), otherwise scroll task list
/// - `Home`/`End`: Jump to first/last task
fn handle_task_list_panel_key(
    state: &mut TuiState,
    key: &CrosstermKeyEvent,
    task_list_height: u16,
    task_detail_height: u16,
) {
    let page_size = task_list_height.saturating_sub(2) as usize; // Account for borders

    // Check if a task is selected (task detail is showing)
    let task_selected = state.task_list_state.get_selected_task().is_some();

    match key.code {
        CrosstermKeyCode::Up | CrosstermKeyCode::Char('k') => {
            state.task_list_state.select_previous();
        }
        CrosstermKeyCode::Down | CrosstermKeyCode::Char('j') => {
            state.task_list_state.select_next();
        }
        CrosstermKeyCode::PageUp => {
            // When a task is selected, scroll the task detail panel
            if task_selected {
                handle_task_detail_key(state, key.code, task_detail_height);
            } else {
                // Move selection up by page size in the task list
                for _ in 0..page_size {
                    state.task_list_state.select_previous();
                }
            }
        }
        CrosstermKeyCode::PageDown => {
            // When a task is selected, scroll the task detail panel
            if task_selected {
                handle_task_detail_key(state, key.code, task_detail_height);
            } else {
                // Move selection down by page size in the task list
                for _ in 0..page_size {
                    state.task_list_state.select_next();
                }
            }
        }
        CrosstermKeyCode::Home => {
            // Jump to first task
            if !state.task_list_state.is_empty() {
                state.task_list_state.selected_index = Some(0);
            }
        }
        CrosstermKeyCode::End => {
            // Jump to last task
            let len = state.task_list_state.len();
            if len > 0 {
                state.task_list_state.selected_index = Some(len - 1);
            }
        }
        _ => {}
    }
}

/// Handles keys for the App Logs panel.
///
/// Keys are delegated to tui-logger's built-in controls:
/// - `h`: Toggle target selector (show/hide log targets)
/// - `←`/`→`: Filter by log level (decrease/increase minimum level)
/// - `PgUp`/`PgDn`: Scroll logs up/down
/// - `Space`: Toggle focus between target list and log view (when target selector visible)
/// - `↑`/`↓`: Navigate targets (when target selector visible) or scroll logs
fn handle_app_logs_panel_key(state: &mut TuiState, key: &CrosstermKeyEvent) {
    use tui_logger::TuiWidgetEvent;

    match key.code {
        CrosstermKeyCode::PageUp => {
            state.logger_state.transition(TuiWidgetEvent::PrevPageKey);
        }
        CrosstermKeyCode::PageDown => {
            state.logger_state.transition(TuiWidgetEvent::NextPageKey);
        }
        CrosstermKeyCode::Up | CrosstermKeyCode::Char('k') => {
            state.logger_state.transition(TuiWidgetEvent::UpKey);
        }
        CrosstermKeyCode::Down | CrosstermKeyCode::Char('j') => {
            state.logger_state.transition(TuiWidgetEvent::DownKey);
        }
        CrosstermKeyCode::Home => {
            state.app_logs_scroll = 0;
        }
        CrosstermKeyCode::Char('h') => {
            // Toggle target selector visibility
            state.logger_state.transition(TuiWidgetEvent::HideKey);
        }
        CrosstermKeyCode::Left => {
            // Decrease log level filter
            state.logger_state.transition(TuiWidgetEvent::LeftKey);
        }
        CrosstermKeyCode::Right => {
            // Increase log level filter
            state.logger_state.transition(TuiWidgetEvent::RightKey);
        }
        CrosstermKeyCode::Char(' ') => {
            // Toggle focus between target list and log view
            state.logger_state.transition(TuiWidgetEvent::SpaceKey);
        }
        CrosstermKeyCode::Esc => {
            // Exit scroll mode
            state.logger_state.transition(TuiWidgetEvent::EscapeKey);
        }
        CrosstermKeyCode::Char('+') => {
            // Increase capture level filter
            state.logger_state.transition(TuiWidgetEvent::PlusKey);
        }
        CrosstermKeyCode::Char('-') => {
            // Decrease capture level filter
            state.logger_state.transition(TuiWidgetEvent::MinusKey);
        }
        CrosstermKeyCode::Char('f') => {
            // Toggle focus on selected target only
            state.logger_state.transition(TuiWidgetEvent::FocusKey);
        }
        _ => {}
    }
}
