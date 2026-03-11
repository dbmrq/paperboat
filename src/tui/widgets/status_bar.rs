//! Status bar widget module.
//!
//! This module contains [`render_status_bar`] which displays
//! status information, keybindings, and messages at the bottom
//! of the terminal.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::super::state::TuiState;

/// Renders the status bar at the bottom of the terminal.
///
/// Display format: `` Status: Running │ Agents: ✓{succeeded} ✗{failed} ~{in_progress} ({total} total, {active} active) │ Tasks: 2/4 │ ?=help ``
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The rectangular area to render the status bar
/// * `state` - The current TUI state to extract statistics from
pub fn render_status_bar(frame: &mut Frame, area: Rect, state: &TuiState) {
    let (succeeded, failed, in_progress, total, active) = state.get_agent_stats();
    let (completed_tasks, total_tasks) = state.get_task_progress();
    let is_running = state.is_running();

    // Build status text
    let status_text = if is_running { "Running" } else { "Idle" };

    // Build the spans with styling
    // Start with single space padding
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            status_text,
            Style::default().fg(if is_running {
                Color::Green
            } else {
                Color::Yellow
            }),
        ),
        Span::raw(" │ "),
        Span::styled("Agents: ", Style::default().add_modifier(Modifier::BOLD)),
        // Succeeded count (green)
        Span::styled(format!("✓{succeeded}"), Style::default().fg(Color::Green)),
        Span::raw(" "),
        // Failed count (red)
        Span::styled(format!("✗{failed}"), Style::default().fg(Color::Red)),
        Span::raw(" "),
        // In-progress count (yellow)
        Span::styled(
            format!("~{in_progress}"),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" "),
        // Total and active (cyan)
        Span::styled(
            format!("({total} total, {active} active)"),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw(" │ "),
        Span::styled("Tasks: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{completed_tasks}/{total_tasks}"),
            Style::default().fg(if completed_tasks == total_tasks && total_tasks > 0 {
                Color::Green
            } else if completed_tasks > 0 {
                Color::Yellow
            } else {
                Color::White
            }),
        ),
    ];

    // Add status message if present
    if let Some(ref msg) = state.status_message {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Magenta),
        ));
    }

    // Always add help and settings hints at the end
    spans.push(Span::raw(" │ "));
    spans.push(Span::styled("?=help", Style::default().fg(Color::Gray)));
    spans.push(Span::raw(" "));
    spans.push(Span::styled("s=settings", Style::default().fg(Color::Gray)));

    // End with single space padding
    spans.push(Span::raw(" "));

    let line = Line::from(spans);
    let paragraph =
        Paragraph::new(line).style(Style::default().bg(Color::DarkGray).fg(Color::White));

    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::logging::{AgentType, LogEvent};

    fn render_status_bar_to_string(state: &TuiState, area: Rect) -> String {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_status_bar(frame, area, state))
            .expect("status bar should render");
        format!("{}", terminal.backend())
    }

    // ========================================================================
    // Basic Render Tests
    // ========================================================================

    #[test]
    fn test_render_status_bar_shows_status_label() {
        let state = TuiState::new();
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("Status:"));
    }

    #[test]
    fn test_render_status_bar_shows_idle_when_no_agents() {
        let state = TuiState::new();
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("Idle"));
    }

    #[test]
    fn test_render_status_bar_shows_running_with_active_agent() {
        let mut state = TuiState::new();
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Test task".to_string(),
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("Running"));
    }

    #[test]
    fn test_render_status_bar_shows_agents_label() {
        let state = TuiState::new();
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("Agents:"));
    }

    #[test]
    fn test_render_status_bar_shows_tasks_label() {
        let state = TuiState::new();
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("Tasks:"));
    }

    #[test]
    fn test_render_status_bar_shows_help_hint() {
        let state = TuiState::new();
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("?=help"));
    }

    #[test]
    fn test_render_status_bar_shows_settings_hint() {
        let state = TuiState::new();
        // Use wider terminal to ensure settings hint fits
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 120, 1));

        assert!(rendered.contains("settings"));
    }

    // ========================================================================
    // Agent Stats Tests
    // ========================================================================

    #[test]
    fn test_render_status_bar_shows_succeeded_count() {
        let mut state = TuiState::new();
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Task".to_string(),
        });
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("sess-1".to_string()),
            depth: 0,
            success: true,
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 100, 1));

        assert!(rendered.contains("✓1"));
    }

    #[test]
    fn test_render_status_bar_shows_failed_count() {
        let mut state = TuiState::new();
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Task".to_string(),
        });
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("sess-1".to_string()),
            depth: 0,
            success: false,
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 100, 1));

        assert!(rendered.contains("✗1"));
    }

    #[test]
    fn test_render_status_bar_shows_in_progress_count() {
        let mut state = TuiState::new();
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Task".to_string(),
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 100, 1));

        assert!(rendered.contains("~1"));
    }

    #[test]
    fn test_render_status_bar_shows_total_agents() {
        let mut state = TuiState::new();
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "sess-1".to_string(),
            depth: 0,
            task: "Task 1".to_string(),
        });
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Planner,
            session_id: "sess-2".to_string(),
            depth: 0,
            task: "Task 2".to_string(),
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 120, 1));

        assert!(rendered.contains("2 total"));
    }

    // ========================================================================
    // Task Progress Tests
    // ========================================================================

    #[test]
    fn test_render_status_bar_shows_task_progress_zero() {
        let state = TuiState::new();
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("0/0"));
    }

    #[test]
    fn test_render_status_bar_shows_task_progress() {
        let mut state = TuiState::new();
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            description: "Desc".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.handle_event(LogEvent::TaskCreated {
            task_id: "task-2".to_string(),
            name: "Task 2".to_string(),
            description: "Desc".to_string(),
            dependencies: vec![],
            depth: 0,
        });
        state.handle_event(LogEvent::TaskStateChanged {
            task_id: "task-1".to_string(),
            name: "Task 1".to_string(),
            old_status: "pending".to_string(),
            new_status: "completed".to_string(),
            depth: 0,
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 80, 1));

        assert!(rendered.contains("1/2"));
    }

    // ========================================================================
    // Status Message Tests
    // ========================================================================

    #[test]
    fn test_render_status_bar_shows_status_message() {
        let mut state = TuiState::new();
        state.status_message = Some("Custom message".to_string());

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 100, 1));

        assert!(rendered.contains("Custom message"));
    }

    #[test]
    fn test_render_status_bar_no_status_message_when_none() {
        let state = TuiState::new();

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 100, 1));

        // Should not contain any random status message
        assert!(!rendered.contains("Custom message"));
    }

    // ========================================================================
    // Integration Tests
    // ========================================================================

    #[test]
    fn test_status_bar_full_agent_workflow() {
        let mut state = TuiState::new();

        // Start agents
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Orchestrator,
            session_id: "orch".to_string(),
            depth: 0,
            task: "Orchestrate".to_string(),
        });
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Planner,
            session_id: "plan".to_string(),
            depth: 0,
            task: "Plan".to_string(),
        });
        state.handle_event(LogEvent::AgentStarted {
            agent_type: AgentType::Implementer { index: 1 },
            session_id: "impl".to_string(),
            depth: 1,
            task: "Implement".to_string(),
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 120, 1));
        assert!(rendered.contains("Running"));
        assert!(rendered.contains("~3")); // 3 in progress

        // Complete some
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Planner,
            session_id: Some("plan".to_string()),
            depth: 0,
            success: true,
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 120, 1));
        assert!(rendered.contains("✓1")); // 1 succeeded
        assert!(rendered.contains("~2")); // 2 still in progress

        // Fail one
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Implementer { index: 1 },
            session_id: Some("impl".to_string()),
            depth: 1,
            success: false,
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 120, 1));
        assert!(rendered.contains("✗1")); // 1 failed
        assert!(rendered.contains("~1")); // 1 still in progress

        // Complete last
        state.handle_event(LogEvent::AgentComplete {
            agent_type: AgentType::Orchestrator,
            session_id: Some("orch".to_string()),
            depth: 0,
            success: true,
        });

        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 120, 1));
        assert!(rendered.contains("Idle")); // No more running
        assert!(rendered.contains("✓2")); // 2 succeeded
        assert!(rendered.contains("✗1")); // 1 failed
        assert!(rendered.contains("~0")); // 0 in progress
    }

    #[test]
    fn test_status_bar_renders_on_narrow_terminal() {
        let state = TuiState::new();

        // Should not panic on narrow terminal
        let rendered = render_status_bar_to_string(&state, Rect::new(0, 0, 40, 1));

        // Core elements should still be present (may be truncated)
        assert!(rendered.contains("Status"));
    }
}
