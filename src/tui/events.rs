//! TUI event handling module.
//!
//! This module handles keyboard and mouse events and routes them to the appropriate
//! handlers based on focus state.
//!
//! # Event Routing
//!
//! ## Keyboard Events
//!
//! Keyboard events are routed based on two levels:
//!
//! 1. **Global keys** - Always handled regardless of focus:
//!    - `Tab`: Cycle focus between panels
//!    - `Shift+Tab`: Cycle focus in reverse
//!    - `q`: Quit the TUI
//!    - `?`: Toggle help overlay
//!
//! 2. **Panel-specific keys** - Routed based on current focus:
//!    - Agent Tree: ↑/↓ navigate, ←/→ collapse/expand, Enter select, f toggle auto-follow
//!    - Agent Output: PgUp/PgDn scroll, Home/End jump
//!    - Task List: ↑/↓ navigate, PgUp/PgDn scroll
//!    - App Logs: h toggle target selector, ←/→ filter level, PgUp/PgDn scroll
//!
//! ## Mouse Events
//!
//! Mouse clicks on panels switch focus to the clicked panel. This provides an
//! alternative to using Tab/Shift+Tab for panel navigation.

use crossterm::event::{
    KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent,
    KeyModifiers as CrosstermKeyModifiers, MouseEvent as CrosstermMouseEvent,
};

use super::layout::PanelLayout;
use super::state::{FocusedPanel, TuiState};
use super::widgets::{handle_agent_output_key, handle_agent_tree_key};

// ============================================================================
// Event Result
// ============================================================================

/// Result of handling a key event.
///
/// This enum indicates whether the TUI should continue running or quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    /// Continue running the TUI
    Continue,
    /// Quit the TUI
    Quit,
}

// ============================================================================
// Key Event Handling
// ============================================================================

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
///
/// # Arguments
///
/// * `state` - Mutable reference to the TUI state
/// * `key` - The `crossterm` key event to handle
/// * `visible_height` - The visible height of panels (for scroll calculations)
///
/// # Returns
///
/// - [`EventResult::Quit`] if the user pressed 'q' to quit
/// - [`EventResult::Continue`] for all other events
///
/// # Example
///
/// ```ignore
/// use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
/// use paperboat::tui::events::{handle_key_event, EventResult};
/// use paperboat::tui::TuiState;
///
/// let mut state = TuiState::new();
/// let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::empty());
///
/// let result = handle_key_event(&mut state, key, 20);
/// assert_eq!(result, EventResult::Continue);
/// ```
pub fn handle_key_event(
    state: &mut TuiState,
    key: CrosstermKeyEvent,
    visible_height: u16,
) -> EventResult {
    // Phase 1: Handle global keys
    if let Some(result) = handle_global_key(state, &key) {
        return result;
    }

    // Phase 2: Route to panel-specific handler based on current focus
    handle_panel_key(state, &key, visible_height);

    EventResult::Continue
}

/// Handles global keys that work regardless of panel focus.
///
/// Returns `Some(EventResult)` if the key was handled, `None` if not a global key.
#[allow(clippy::missing_const_for_fn)] // Uses non-const match expressions
fn handle_global_key(state: &mut TuiState, key: &CrosstermKeyEvent) -> Option<EventResult> {
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

        // Esc: quit (or close help overlay if visible)
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

        _ => None,
    }
}

/// Routes key events to the appropriate panel handler based on current focus.
fn handle_panel_key(state: &mut TuiState, key: &CrosstermKeyEvent, visible_height: u16) {
    // If help is visible, don't process panel keys (except Esc handled above)
    if state.help_visible {
        return;
    }

    match state.current_focus {
        FocusedPanel::AgentTree => handle_agent_tree_panel_key(state, key),
        FocusedPanel::AgentOutput => handle_agent_output_panel_key(state, key, visible_height),
        FocusedPanel::TaskList => handle_task_list_panel_key(state, key, visible_height),
        FocusedPanel::AppLogs => handle_app_logs_panel_key(state, key),
    }
}

// ============================================================================
// Mouse Event Handling
// ============================================================================

/// Handles a mouse click event to switch panel focus.
///
/// Maps the click coordinates to panel areas and updates the current focus
/// if the click occurred within a valid panel area.
///
/// # Arguments
///
/// * `state` - Mutable reference to the TUI state
/// * `mouse_event` - The crossterm mouse event containing click coordinates
/// * `layout` - The current panel layout with rectangle areas
///
/// # Panel Areas
///
/// Clicks are mapped to the following panels:
/// - Agent Tree (left side, top section)
/// - Agent Output (center, top section)
/// - Task List (right side, top section)
/// - App Logs (full width, middle section)
///
/// Clicks on the status bar are ignored.
pub fn handle_mouse_click(
    state: &mut TuiState,
    mouse_event: CrosstermMouseEvent,
    layout: &PanelLayout,
) {
    // Don't process mouse clicks if help overlay is visible
    if state.help_visible {
        return;
    }

    let column = mouse_event.column;
    let row = mouse_event.row;

    // Check which panel was clicked and update focus accordingly
    if let Some(panel) = panel_at_position(column, row, layout) {
        state.current_focus = panel;
    }
}

/// Determines which panel (if any) is at the given screen position.
///
/// Returns `None` if the position is in the status bar or outside all panels.
fn panel_at_position(column: u16, row: u16, layout: &PanelLayout) -> Option<FocusedPanel> {
    // Check each panel area - order doesn't matter as they don't overlap
    if rect_contains(layout.agent_tree, column, row) {
        Some(FocusedPanel::AgentTree)
    } else if rect_contains(layout.agent_output, column, row) {
        Some(FocusedPanel::AgentOutput)
    } else if rect_contains(layout.task_list, column, row) {
        Some(FocusedPanel::TaskList)
    } else if rect_contains(layout.app_logs, column, row) {
        Some(FocusedPanel::AppLogs)
    } else {
        // Click was on status bar or outside valid panel areas
        None
    }
}

/// Checks if a point (column, row) is within a rectangle.
#[inline]
fn rect_contains(rect: ratatui::layout::Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
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
) {
    // Delegate to the widget handler which handles all scrolling
    handle_agent_output_key(state, key.code, visible_height);
}

/// Handles keys for the Task List panel.
///
/// Keys:
/// - `↑`/`↓`: Navigate tasks
/// - `PgUp`/`PgDn`: Scroll task list by page
fn handle_task_list_panel_key(state: &mut TuiState, key: &CrosstermKeyEvent, visible_height: u16) {
    let page_size = visible_height.saturating_sub(2) as usize; // Account for borders

    match key.code {
        CrosstermKeyCode::Up | CrosstermKeyCode::Char('k') => {
            state.task_list_state.select_previous();
        }
        CrosstermKeyCode::Down | CrosstermKeyCode::Char('j') => {
            state.task_list_state.select_next();
        }
        CrosstermKeyCode::PageUp => {
            // Move selection up by page size
            for _ in 0..page_size {
                state.task_list_state.select_previous();
            }
        }
        CrosstermKeyCode::PageDown => {
            // Move selection down by page size
            for _ in 0..page_size {
                state.task_list_state.select_next();
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
///
/// Note: tui-logger's `TuiWidgetState` handles these events internally.
/// We provide the key routing here, but actual handling requires
/// passing events to the logger state during rendering.
fn handle_app_logs_panel_key(state: &mut TuiState, key: &CrosstermKeyEvent) {
    // For basic scrolling without tui-logger's internal state,
    // we use our own scroll offset tracking.
    // Full tui-logger integration requires TuiWidgetState which
    // should be added to TuiState when rendering is implemented.

    match key.code {
        CrosstermKeyCode::PageUp => {
            // Scroll logs up
            state.app_logs_scroll = state.app_logs_scroll.saturating_sub(10);
        }
        CrosstermKeyCode::PageDown => {
            // Scroll logs down
            state.app_logs_scroll = state.app_logs_scroll.saturating_add(10);
        }
        CrosstermKeyCode::Up | CrosstermKeyCode::Char('k') => {
            state.app_logs_scroll = state.app_logs_scroll.saturating_sub(1);
        }
        CrosstermKeyCode::Down | CrosstermKeyCode::Char('j') => {
            state.app_logs_scroll = state.app_logs_scroll.saturating_add(1);
        }
        CrosstermKeyCode::Home => {
            state.app_logs_scroll = 0;
        }
        // h, ←, → are tui-logger specific controls that need TuiWidgetState
        // They are documented here for future implementation
        CrosstermKeyCode::Char('h') => {
            // TODO: Toggle target selector when TuiWidgetState is integrated
            state.set_status_message("Target selector: not yet implemented");
        }
        CrosstermKeyCode::Left => {
            // TODO: Decrease log level filter when TuiWidgetState is integrated
            state.set_status_message("Log level filter: not yet implemented");
        }
        CrosstermKeyCode::Right => {
            // TODO: Increase log level filter when TuiWidgetState is integrated
            state.set_status_message("Log level filter: not yet implemented");
        }
        _ => {}
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent as CtKeyEvent, KeyEventKind, KeyEventState};

    /// Helper to create a simple key event
    fn key(code: CrosstermKeyCode) -> CrosstermKeyEvent {
        CtKeyEvent {
            code,
            modifiers: CrosstermKeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn test_quit_key() {
        let mut state = TuiState::new();
        let result = handle_key_event(&mut state, key(CrosstermKeyCode::Char('q')), 20);
        assert_eq!(result, EventResult::Quit);
    }

    #[test]
    fn test_tab_cycles_focus() {
        let mut state = TuiState::new();
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), 20);
        assert_eq!(state.current_focus, FocusedPanel::AgentOutput);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), 20);
        assert_eq!(state.current_focus, FocusedPanel::TaskList);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), 20);
        assert_eq!(state.current_focus, FocusedPanel::AppLogs);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), 20);
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
    }

    #[test]
    fn test_shift_tab_cycles_focus_reverse() {
        let mut state = TuiState::new();
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);

        handle_key_event(&mut state, key(CrosstermKeyCode::BackTab), 20);
        assert_eq!(state.current_focus, FocusedPanel::AppLogs);

        handle_key_event(&mut state, key(CrosstermKeyCode::BackTab), 20);
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_question_mark_toggles_help() {
        let mut state = TuiState::new();
        assert!(!state.help_visible);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('?')), 20);
        assert!(state.help_visible);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('?')), 20);
        assert!(!state.help_visible);
    }

    #[test]
    fn test_esc_closes_help() {
        let mut state = TuiState::new();
        state.help_visible = true;

        let result = handle_key_event(&mut state, key(CrosstermKeyCode::Esc), 20);
        assert_eq!(result, EventResult::Continue);
        assert!(!state.help_visible);
    }

    #[test]
    fn test_f_toggles_auto_follow_in_agent_tree() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::AgentTree;
        assert!(state.auto_follow_enabled);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('f')), 20);
        assert!(!state.auto_follow_enabled);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('f')), 20);
        assert!(state.auto_follow_enabled);
    }

    #[test]
    fn test_task_list_navigation() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::TaskList;

        // Add some tasks
        state.task_list_state.handle_task_created(
            "t1".to_string(),
            "Task 1".to_string(),
            "Desc".to_string(),
            vec![],
            0,
        );
        state.task_list_state.handle_task_created(
            "t2".to_string(),
            "Task 2".to_string(),
            "Desc".to_string(),
            vec![],
            0,
        );
        state.task_list_state.handle_task_created(
            "t3".to_string(),
            "Task 3".to_string(),
            "Desc".to_string(),
            vec![],
            0,
        );

        // Down arrow selects first task
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), 20);
        assert_eq!(state.task_list_state.selected_index, Some(0));

        // Down again
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), 20);
        assert_eq!(state.task_list_state.selected_index, Some(1));

        // Up
        handle_key_event(&mut state, key(CrosstermKeyCode::Up), 20);
        assert_eq!(state.task_list_state.selected_index, Some(0));
    }

    #[test]
    fn test_task_list_home_end() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::TaskList;

        // Add tasks
        for i in 0..5 {
            state.task_list_state.handle_task_created(
                format!("t{i}"),
                format!("Task {i}"),
                "Desc".to_string(),
                vec![],
                0,
            );
        }

        // End jumps to last
        handle_key_event(&mut state, key(CrosstermKeyCode::End), 20);
        assert_eq!(state.task_list_state.selected_index, Some(4));

        // Home jumps to first
        handle_key_event(&mut state, key(CrosstermKeyCode::Home), 20);
        assert_eq!(state.task_list_state.selected_index, Some(0));
    }

    #[test]
    fn test_app_logs_scroll() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::AppLogs;
        assert_eq!(state.app_logs_scroll, 0);

        handle_key_event(&mut state, key(CrosstermKeyCode::PageDown), 20);
        assert_eq!(state.app_logs_scroll, 10);

        handle_key_event(&mut state, key(CrosstermKeyCode::PageUp), 20);
        assert_eq!(state.app_logs_scroll, 0);

        handle_key_event(&mut state, key(CrosstermKeyCode::Down), 20);
        assert_eq!(state.app_logs_scroll, 1);
    }

    #[test]
    fn test_help_visible_blocks_panel_keys() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::TaskList;
        state.help_visible = true;

        // Add a task
        state.task_list_state.handle_task_created(
            "t1".to_string(),
            "Task 1".to_string(),
            "Desc".to_string(),
            vec![],
            0,
        );

        // Down arrow should not change selection when help is visible
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), 20);
        assert!(state.task_list_state.selected_index.is_none());
    }

    #[test]
    fn test_global_keys_work_regardless_of_focus() {
        // Tab should work from any panel
        for panel in [
            FocusedPanel::AgentTree,
            FocusedPanel::AgentOutput,
            FocusedPanel::TaskList,
            FocusedPanel::AppLogs,
        ] {
            let mut state = TuiState::new();
            state.current_focus = panel;

            let result = handle_key_event(&mut state, key(CrosstermKeyCode::Tab), 20);
            assert_eq!(result, EventResult::Continue);
            assert_ne!(state.current_focus, panel); // Focus should have changed
        }

        // q should quit from any panel
        for panel in [
            FocusedPanel::AgentTree,
            FocusedPanel::AgentOutput,
            FocusedPanel::TaskList,
            FocusedPanel::AppLogs,
        ] {
            let mut state = TuiState::new();
            state.current_focus = panel;

            let result = handle_key_event(&mut state, key(CrosstermKeyCode::Char('q')), 20);
            assert_eq!(result, EventResult::Quit);
        }
    }

    #[test]
    fn test_event_result_variants() {
        assert_eq!(EventResult::Continue, EventResult::Continue);
        assert_ne!(EventResult::Continue, EventResult::Quit);
    }

    // ========================================================================
    // Mouse Click Tests
    // ========================================================================

    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::layout::Rect;

    /// Helper to create a mouse click event at (column, row)
    fn mouse_click(column: u16, row: u16) -> CrosstermMouseEvent {
        CrosstermMouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: CrosstermKeyModifiers::empty(),
        }
    }

    /// Helper to create a standard layout for testing
    fn test_layout() -> PanelLayout {
        // Simulate a 120x40 terminal layout
        PanelLayout {
            agent_tree: Rect::new(0, 0, 24, 27),
            agent_output: Rect::new(24, 0, 60, 27),
            task_list: Rect::new(84, 0, 36, 27),
            app_logs: Rect::new(0, 27, 120, 12),
            status_bar: Rect::new(0, 39, 120, 1),
        }
    }

    #[test]
    fn test_mouse_click_switches_to_agent_tree() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::AgentOutput;
        let layout = test_layout();

        handle_mouse_click(&mut state, mouse_click(10, 10), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
    }

    #[test]
    fn test_mouse_click_switches_to_agent_output() {
        let mut state = TuiState::new();
        let layout = test_layout();

        handle_mouse_click(&mut state, mouse_click(50, 10), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AgentOutput);
    }

    #[test]
    fn test_mouse_click_switches_to_task_list() {
        let mut state = TuiState::new();
        let layout = test_layout();

        handle_mouse_click(&mut state, mouse_click(100, 10), &layout);
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_mouse_click_switches_to_app_logs() {
        let mut state = TuiState::new();
        let layout = test_layout();

        handle_mouse_click(&mut state, mouse_click(60, 30), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AppLogs);
    }

    #[test]
    fn test_mouse_click_on_status_bar_no_change() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::AgentTree;
        let layout = test_layout();

        handle_mouse_click(&mut state, mouse_click(60, 39), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
    }

    #[test]
    fn test_mouse_click_ignored_when_help_visible() {
        let mut state = TuiState::new();
        state.current_focus = FocusedPanel::AgentTree;
        state.help_visible = true;
        let layout = test_layout();

        handle_mouse_click(&mut state, mouse_click(50, 10), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
    }

    #[test]
    fn test_rect_contains_basic() {
        let rect = Rect::new(10, 10, 20, 15);

        assert!(rect_contains(rect, 10, 10));
        assert!(rect_contains(rect, 29, 24));
        assert!(rect_contains(rect, 20, 17));

        assert!(!rect_contains(rect, 9, 10));
        assert!(!rect_contains(rect, 10, 9));
        assert!(!rect_contains(rect, 30, 10));
        assert!(!rect_contains(rect, 10, 25));
    }

    #[test]
    fn test_panel_at_position() {
        let layout = test_layout();

        assert_eq!(
            panel_at_position(0, 0, &layout),
            Some(FocusedPanel::AgentTree)
        );
        assert_eq!(
            panel_at_position(24, 0, &layout),
            Some(FocusedPanel::AgentOutput)
        );
        assert_eq!(
            panel_at_position(84, 0, &layout),
            Some(FocusedPanel::TaskList)
        );
        assert_eq!(
            panel_at_position(0, 27, &layout),
            Some(FocusedPanel::AppLogs)
        );
        assert_eq!(panel_at_position(60, 39, &layout), None);
    }
}
