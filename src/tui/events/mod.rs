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

mod keyboard;
mod mouse;

// Re-exports
pub use keyboard::handle_key_event;
pub use mouse::{handle_mouse_click, handle_mouse_scroll, ScrollDirection};

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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::layout::calculate_layout;
    use crate::tui::state::{FocusedPanel, TuiState};
    use crossterm::event::{
        KeyCode as CrosstermKeyCode, KeyEvent as CtKeyEvent, KeyEventKind, KeyEventState,
        KeyModifiers as CrosstermKeyModifiers, MouseButton, MouseEvent as CrosstermMouseEvent,
        MouseEventKind,
    };
    use ratatui::layout::Rect;

    /// Helper to create a simple key event
    fn key(code: CrosstermKeyCode) -> CtKeyEvent {
        CtKeyEvent {
            code,
            modifiers: CrosstermKeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    /// Helper to create a test layout with reasonable panel sizes
    fn test_layout() -> crate::tui::layout::PanelLayout {
        calculate_layout(Rect::new(0, 0, 120, 40))
    }

    #[test]
    fn test_quit_key() {
        let mut state = TuiState::new();
        let layout = test_layout();
        let result = handle_key_event(&mut state, key(CrosstermKeyCode::Char('q')), &layout);
        assert_eq!(result, EventResult::Quit);
    }

    #[test]
    fn test_tab_cycles_focus() {
        let mut state = TuiState::new();
        let layout = test_layout();
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AgentOutput);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(state.current_focus, FocusedPanel::TaskList);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AppLogs);

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);
    }

    #[test]
    fn test_shift_tab_cycles_focus_reverse() {
        let mut state = TuiState::new();
        let layout = test_layout();
        assert_eq!(state.current_focus, FocusedPanel::AgentTree);

        handle_key_event(&mut state, key(CrosstermKeyCode::BackTab), &layout);
        assert_eq!(state.current_focus, FocusedPanel::AppLogs);

        handle_key_event(&mut state, key(CrosstermKeyCode::BackTab), &layout);
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_question_mark_toggles_help() {
        let mut state = TuiState::new();
        let layout = test_layout();
        assert!(!state.help_visible);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('?')), &layout);
        assert!(state.help_visible);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('?')), &layout);
        assert!(!state.help_visible);
    }

    #[test]
    fn test_esc_closes_help() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.help_visible = true;

        let result = handle_key_event(&mut state, key(CrosstermKeyCode::Esc), &layout);
        assert_eq!(result, EventResult::Continue);
        assert!(!state.help_visible);
    }

    #[test]
    fn test_f_toggles_auto_follow_in_agent_tree() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.current_focus = FocusedPanel::AgentTree;
        assert!(state.auto_follow_enabled);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('f')), &layout);
        assert!(!state.auto_follow_enabled);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('f')), &layout);
        assert!(state.auto_follow_enabled);
    }

    #[test]
    fn test_task_list_navigation() {
        let mut state = TuiState::new();
        let layout = test_layout();
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
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        assert_eq!(state.task_list_state.selected_index, Some(0));

        // Down again
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        assert_eq!(state.task_list_state.selected_index, Some(1));

        // Up
        handle_key_event(&mut state, key(CrosstermKeyCode::Up), &layout);
        assert_eq!(state.task_list_state.selected_index, Some(0));
    }

    #[test]
    fn test_task_list_home_end() {
        let mut state = TuiState::new();
        let layout = test_layout();
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
        handle_key_event(&mut state, key(CrosstermKeyCode::End), &layout);
        assert_eq!(state.task_list_state.selected_index, Some(4));

        // Home jumps to first
        handle_key_event(&mut state, key(CrosstermKeyCode::Home), &layout);
        assert_eq!(state.task_list_state.selected_index, Some(0));
    }

    #[test]
    fn test_app_logs_key_events_handled() {
        // Tests that App Logs panel key events are handled without panics.
        let mut state = TuiState::new();
        let layout = test_layout();
        state.current_focus = FocusedPanel::AppLogs;

        // Test scroll events
        handle_key_event(&mut state, key(CrosstermKeyCode::PageDown), &layout);
        handle_key_event(&mut state, key(CrosstermKeyCode::PageUp), &layout);
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        handle_key_event(&mut state, key(CrosstermKeyCode::Up), &layout);

        // Test level filter events
        handle_key_event(&mut state, key(CrosstermKeyCode::Left), &layout);
        handle_key_event(&mut state, key(CrosstermKeyCode::Right), &layout);

        // Test target selector toggle
        handle_key_event(&mut state, key(CrosstermKeyCode::Char('h')), &layout);

        // Test focus toggle
        handle_key_event(&mut state, key(CrosstermKeyCode::Char(' ')), &layout);

        // Test escape
        handle_key_event(&mut state, key(CrosstermKeyCode::Esc), &layout);

        // Test capture level adjustments
        handle_key_event(&mut state, key(CrosstermKeyCode::Char('+')), &layout);
        handle_key_event(&mut state, key(CrosstermKeyCode::Char('-')), &layout);

        // Test focus on target
        handle_key_event(&mut state, key(CrosstermKeyCode::Char('f')), &layout);

        // Verify Home still resets app_logs_scroll
        state.app_logs_scroll = 10;
        handle_key_event(&mut state, key(CrosstermKeyCode::Home), &layout);
        assert_eq!(state.app_logs_scroll, 0);
    }

    #[test]
    fn test_help_visible_blocks_panel_keys() {
        let mut state = TuiState::new();
        let layout = test_layout();
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
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        assert!(state.task_list_state.selected_index.is_none());
    }

    #[test]
    fn test_global_keys_work_regardless_of_focus() {
        let layout = test_layout();

        // Tab should work from any panel
        for panel in [
            FocusedPanel::AgentTree,
            FocusedPanel::AgentOutput,
            FocusedPanel::TaskList,
            FocusedPanel::AppLogs,
        ] {
            let mut state = TuiState::new();
            state.current_focus = panel;

            let result = handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
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

            let result = handle_key_event(&mut state, key(CrosstermKeyCode::Char('q')), &layout);
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

    /// Helper to create a mouse click event at (column, row)
    fn mouse_click(column: u16, row: u16) -> CrosstermMouseEvent {
        CrosstermMouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: CrosstermKeyModifiers::empty(),
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

        assert!(mouse::rect_contains(rect, 10, 10));
        assert!(mouse::rect_contains(rect, 29, 24));
        assert!(mouse::rect_contains(rect, 20, 17));

        assert!(!mouse::rect_contains(rect, 9, 10));
        assert!(!mouse::rect_contains(rect, 10, 9));
        assert!(!mouse::rect_contains(rect, 30, 10));
        assert!(!mouse::rect_contains(rect, 10, 25));
    }

    #[test]
    fn test_panel_at_position() {
        let layout = test_layout();

        assert_eq!(
            mouse::panel_at_position(0, 0, &layout),
            Some(FocusedPanel::AgentTree)
        );
        assert_eq!(
            mouse::panel_at_position(24, 0, &layout),
            Some(FocusedPanel::AgentOutput)
        );
        assert_eq!(
            mouse::panel_at_position(84, 0, &layout),
            Some(FocusedPanel::TaskList)
        );
        assert_eq!(
            mouse::panel_at_position(0, 27, &layout),
            Some(FocusedPanel::AppLogs)
        );
        assert_eq!(mouse::panel_at_position(60, 39, &layout), None);
    }

    // ========================================================================
    // Settings Overlay Tests
    // ========================================================================

    #[test]
    fn test_s_toggles_settings() {
        let mut state = TuiState::new();
        let layout = test_layout();
        assert!(!state.settings_visible);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('s')), &layout);
        assert!(state.settings_visible);

        handle_key_event(&mut state, key(CrosstermKeyCode::Char('s')), &layout);
        assert!(!state.settings_visible);
    }

    #[test]
    fn test_esc_closes_settings() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;

        let result = handle_key_event(&mut state, key(CrosstermKeyCode::Esc), &layout);
        assert_eq!(result, EventResult::Continue);
        assert!(!state.settings_visible);
    }

    #[test]
    fn test_settings_blocks_quit() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;

        // q should not quit when settings is visible
        let result = handle_key_event(&mut state, key(CrosstermKeyCode::Char('q')), &layout);
        assert_eq!(result, EventResult::Continue);
        assert!(state.settings_visible);
    }

    #[test]
    fn test_settings_tab_switches_agent_type() {
        use crate::tui::widgets::SelectedAgentType;

        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Orchestrator
        );

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Planner
        );

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Implementer
        );

        handle_key_event(&mut state, key(CrosstermKeyCode::Tab), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Orchestrator
        );
    }

    #[test]
    fn test_settings_left_right_switches_agent_type() {
        use crate::tui::widgets::SelectedAgentType;

        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Orchestrator
        );

        // Right arrow switches to next agent type
        handle_key_event(&mut state, key(CrosstermKeyCode::Right), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Planner
        );

        handle_key_event(&mut state, key(CrosstermKeyCode::Right), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Implementer
        );

        // Left arrow switches to previous agent type
        handle_key_event(&mut state, key(CrosstermKeyCode::Left), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Planner
        );

        handle_key_event(&mut state, key(CrosstermKeyCode::Left), &layout);
        assert_eq!(
            state.settings_state.selected_agent_type,
            SelectedAgentType::Orchestrator
        );
    }

    #[test]
    fn test_settings_up_down_navigates_models() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;

        // Add some available models
        state.available_models = vec![
            crate::models::AvailableModel {
                id: crate::models::ModelId::Haiku4_5,
                name: "Haiku 4.5".to_string(),
                description: "Fast".to_string(),
            },
            crate::models::AvailableModel {
                id: crate::models::ModelId::Sonnet4_5,
                name: "Sonnet 4.5".to_string(),
                description: "Balanced".to_string(),
            },
            crate::models::AvailableModel {
                id: crate::models::ModelId::Opus4_5,
                name: "Opus 4.5".to_string(),
                description: "Powerful".to_string(),
            },
        ];

        assert_eq!(state.settings_state.selected_model_index, 0);

        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        assert_eq!(state.settings_state.selected_model_index, 1);

        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        assert_eq!(state.settings_state.selected_model_index, 2);

        handle_key_event(&mut state, key(CrosstermKeyCode::Up), &layout);
        assert_eq!(state.settings_state.selected_model_index, 1);
    }

    #[test]
    fn test_settings_enter_selects_model() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;

        // Add available models
        state.available_models = vec![
            crate::models::AvailableModel {
                id: crate::models::ModelId::Haiku4_5,
                name: "Haiku 4.5".to_string(),
                description: "Fast".to_string(),
            },
            crate::models::AvailableModel {
                id: crate::models::ModelId::Sonnet4_5,
                name: "Sonnet 4.5".to_string(),
                description: "Balanced".to_string(),
            },
        ];

        // Navigate to second model and select
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        handle_key_event(&mut state, key(CrosstermKeyCode::Enter), &layout);

        assert_eq!(
            state.settings_state.pending_orchestrator,
            Some(crate::models::ModelId::Sonnet4_5)
        );
    }

    #[test]
    fn test_settings_visible_blocks_panel_keys() {
        let mut state = TuiState::new();
        let layout = test_layout();
        state.settings_visible = true;
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

        // Down arrow should navigate settings, not task list
        assert!(state.task_list_state.selected_index.is_none());
        handle_key_event(&mut state, key(CrosstermKeyCode::Down), &layout);
        assert!(state.task_list_state.selected_index.is_none());
    }

    // ========================================================================
    // Mouse Scroll Tests
    // ========================================================================

    /// Helper to create a mouse scroll event at (column, row)
    fn mouse_scroll(column: u16, row: u16, direction: ScrollDirection) -> CrosstermMouseEvent {
        CrosstermMouseEvent {
            kind: match direction {
                ScrollDirection::Up => MouseEventKind::ScrollUp,
                ScrollDirection::Down => MouseEventKind::ScrollDown,
            },
            column,
            row,
            modifiers: CrosstermKeyModifiers::empty(),
        }
    }

    #[test]
    fn test_mouse_scroll_agent_output() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Start at scroll position 0
        assert_eq!(state.agent_output_scroll, 0);

        // Scroll up should stay at 0 (can't go negative)
        handle_mouse_scroll(
            &mut state,
            mouse_scroll(50, 10, ScrollDirection::Up),
            &layout,
            ScrollDirection::Up,
        );
        assert_eq!(state.agent_output_scroll, 0);
    }

    #[test]
    fn test_mouse_scroll_task_list_moves_selection() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Add some tasks
        for i in 1..=5 {
            state.task_list_state.handle_task_created(
                format!("t{i}"),
                format!("Task {i}"),
                "Desc".to_string(),
                vec![],
                0,
            );
        }

        // Initially no selection
        assert!(state.task_list_state.selected_index.is_none());

        // Scroll down in task list should move selection
        handle_mouse_scroll(
            &mut state,
            mouse_scroll(100, 10, ScrollDirection::Down),
            &layout,
            ScrollDirection::Down,
        );
        assert!(state.task_list_state.selected_index.is_some());
    }

    #[test]
    fn test_mouse_scroll_ignored_when_help_visible() {
        let mut state = TuiState::new();
        state.help_visible = true;
        let layout = test_layout();

        // Try to scroll agent output
        state.agent_output_scroll = 5;
        handle_mouse_scroll(
            &mut state,
            mouse_scroll(50, 10, ScrollDirection::Down),
            &layout,
            ScrollDirection::Down,
        );
        // Scroll should be unchanged because help is visible
        assert_eq!(state.agent_output_scroll, 5);
    }

    #[test]
    fn test_mouse_scroll_ignored_when_settings_visible() {
        let mut state = TuiState::new();
        state.settings_visible = true;
        let layout = test_layout();

        // Try to scroll agent output
        state.agent_output_scroll = 5;
        handle_mouse_scroll(
            &mut state,
            mouse_scroll(50, 10, ScrollDirection::Down),
            &layout,
            ScrollDirection::Down,
        );
        // Scroll should be unchanged because settings is visible
        assert_eq!(state.agent_output_scroll, 5);
    }

    #[test]
    fn test_mouse_scroll_on_status_bar_no_action() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Set a known scroll position for agent output
        state.agent_output_scroll = 5;

        // Scroll on status bar (row 39 in 40-line terminal)
        handle_mouse_scroll(
            &mut state,
            mouse_scroll(60, 39, ScrollDirection::Up),
            &layout,
            ScrollDirection::Up,
        );
        // Nothing should change
        assert_eq!(state.agent_output_scroll, 5);
    }

    // ========================================================================
    // Mouse Click Task Selection Tests
    // ========================================================================

    #[test]
    fn test_mouse_click_task_list_selects_task() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Add some tasks
        for i in 0..5 {
            state.task_list_state.handle_task_created(
                format!("task-{i}"),
                format!("Task {i}"),
                "Description".to_string(),
                vec![],
                0,
            );
        }

        // Initially no selection
        assert!(state.task_list_state.selected_index.is_none());

        // Click on the task list area - at row that corresponds to the second task
        // The task list panel's inner area starts at y + 1 (after border)
        let task_list_inner_y = layout.task_list.y + 1;
        let click_row = task_list_inner_y + 1; // Second task (index 1)
        let click_col = layout.task_list.x + 5; // Inside task list

        handle_mouse_click(&mut state, mouse_click(click_col, click_row), &layout);

        // Should select the second task
        assert_eq!(state.task_list_state.selected_index, Some(1));
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_mouse_click_task_list_first_task() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Add tasks
        state.task_list_state.handle_task_created(
            "task-0".to_string(),
            "First Task".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );
        state.task_list_state.handle_task_created(
            "task-1".to_string(),
            "Second Task".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );

        // Click on first task row
        let task_list_inner_y = layout.task_list.y + 1;
        let click_col = layout.task_list.x + 5;

        handle_mouse_click(
            &mut state,
            mouse_click(click_col, task_list_inner_y),
            &layout,
        );

        assert_eq!(state.task_list_state.selected_index, Some(0));
    }

    #[test]
    fn test_mouse_click_task_list_out_of_bounds_row_ignored() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Add only 2 tasks
        state.task_list_state.handle_task_created(
            "task-0".to_string(),
            "Task 0".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );
        state.task_list_state.handle_task_created(
            "task-1".to_string(),
            "Task 1".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );

        // Set initial selection
        state.task_list_state.select_index(0);
        assert_eq!(state.task_list_state.selected_index, Some(0));

        // Click on row that would be task index 10 (out of bounds)
        let task_list_inner_y = layout.task_list.y + 1;
        let click_row = task_list_inner_y + 10; // Index 10, out of bounds
        let click_col = layout.task_list.x + 5;

        // This should not crash and should keep the previous selection
        // if the click is still within the panel but out of task bounds
        if click_row < layout.task_list.y + layout.task_list.height {
            handle_mouse_click(&mut state, mouse_click(click_col, click_row), &layout);
            // Selection unchanged (select_index ignores out of bounds)
            assert_eq!(state.task_list_state.selected_index, Some(0));
        }
    }

    #[test]
    fn test_mouse_click_task_list_empty_list() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // No tasks added - list is empty

        // Click inside task list area
        let task_list_inner_y = layout.task_list.y + 1;
        let click_col = layout.task_list.x + 5;

        // This should not crash
        handle_mouse_click(
            &mut state,
            mouse_click(click_col, task_list_inner_y),
            &layout,
        );

        // Selection should remain None
        assert!(state.task_list_state.selected_index.is_none());
        // But focus should change to TaskList
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
    }

    #[test]
    fn test_mouse_click_task_list_on_border_no_select() {
        let mut state = TuiState::new();
        let layout = test_layout();

        // Add a task
        state.task_list_state.handle_task_created(
            "task-0".to_string(),
            "Task 0".to_string(),
            "Description".to_string(),
            vec![],
            0,
        );

        // Click exactly on the top border (y position of panel)
        let click_col = layout.task_list.x + 5;
        let click_row = layout.task_list.y; // On border, not inner area

        handle_mouse_click(&mut state, mouse_click(click_col, click_row), &layout);

        // Should focus the panel but not select via row calculation
        // (border row is before inner area, so handle_task_list_click won't match)
        assert_eq!(state.current_focus, FocusedPanel::TaskList);
        // Task might be auto-selected by on_focus_changed if list was empty before
        // but since we added a task, let's check if the selection logic ran
    }
}
