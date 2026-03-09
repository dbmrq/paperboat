//! Settings overlay widget module.
//!
//! This module provides the settings overlay UI for viewing and changing model
//! configurations in the TUI. It allows users to:
//!
//! - View current model assignments for each agent type (Orchestrator, Planner, Implementer)
//! - Browse available models with descriptions
//! - Select different models for each agent type
//! - See pending changes before they are saved
//!
//! # Usage
//!
//! Press `s` to open the settings overlay. Navigate with:
//! - `←`/`→` or `Tab`: Switch between agent type tabs
//! - `↑`/`↓`: Navigate the model list
//! - `Enter`: Select the highlighted model
//! - `Esc`: Close the overlay (pending changes are saved on selection)
//!
//! # Architecture
//!
//! The settings system has three components:
//! 1. **UI Layer** (this module): Renders the overlay and manages UI state via [`SettingsState`]
//! 2. **Config Loader** ([`crate::config::loader`]): Loads TOML configs from disk at startup
//! 3. **Config Writer** ([`crate::config::writer`]): Persists changes to `.paperboat/agents/*.toml`
//!
//! Model changes are applied to the [`crate::models::ModelConfig`] but only affect
//! newly spawned agents. Running agents continue with their original model assignment.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::models::ModelTier;
use crate::tui::TuiState;

// ============================================================================
// Agent Type Selection
// ============================================================================

/// The agent type currently selected in the settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectedAgentType {
    /// Orchestrator agent configuration
    #[default]
    Orchestrator,
    /// Planner agent configuration
    Planner,
    /// Implementer agent configuration
    Implementer,
}

impl SelectedAgentType {
    /// Returns the display name for this agent type.
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Orchestrator => "Orchestrator",
            Self::Planner => "Planner",
            Self::Implementer => "Implementer",
        }
    }

    /// Returns all agent types in order.
    pub const fn all() -> &'static [Self] {
        &[Self::Orchestrator, Self::Planner, Self::Implementer]
    }

    /// Cycles to the next agent type.
    pub const fn next(&self) -> Self {
        match self {
            Self::Orchestrator => Self::Planner,
            Self::Planner => Self::Implementer,
            Self::Implementer => Self::Orchestrator,
        }
    }

    /// Cycles to the previous agent type.
    pub const fn prev(&self) -> Self {
        match self {
            Self::Orchestrator => Self::Implementer,
            Self::Planner => Self::Orchestrator,
            Self::Implementer => Self::Planner,
        }
    }
}

// ============================================================================
// Settings State
// ============================================================================

/// State for the settings overlay.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    /// Currently selected agent type (tab)
    pub selected_agent_type: SelectedAgentType,
    /// Currently selected model index within the available tiers list
    pub selected_model_index: usize,
    /// Pending orchestrator model change (not yet applied)
    pub pending_orchestrator: Option<ModelTier>,
    /// Pending planner model change (not yet applied)
    pub pending_planner: Option<ModelTier>,
    /// Pending implementer model change (not yet applied)
    pub pending_implementer: Option<ModelTier>,
}

impl SettingsState {
    /// Creates a new settings state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects the next model in the list.
    pub const fn select_next_model(&mut self, available_models_count: usize) {
        if available_models_count > 0 {
            self.selected_model_index = (self.selected_model_index + 1) % available_models_count;
        }
    }

    /// Selects the previous model in the list.
    pub fn select_previous_model(&mut self, available_models_count: usize) {
        if available_models_count > 0 {
            self.selected_model_index = self
                .selected_model_index
                .checked_sub(1)
                .unwrap_or(available_models_count - 1);
        }
    }

    /// Cycles to the next agent type tab.
    pub const fn next_agent_type(&mut self) {
        self.selected_agent_type = self.selected_agent_type.next();
        self.selected_model_index = 0; // Reset model selection when changing tabs
    }

    /// Cycles to the previous agent type tab.
    pub const fn prev_agent_type(&mut self) {
        self.selected_agent_type = self.selected_agent_type.prev();
        self.selected_model_index = 0; // Reset model selection when changing tabs
    }

    /// Gets the current model tier for the selected agent type.
    ///
    /// Returns the pending tier if set, otherwise the primary tier from config.
    pub fn get_current_model(&self, state: &TuiState) -> Option<ModelTier> {
        match self.selected_agent_type {
            SelectedAgentType::Orchestrator => self
                .pending_orchestrator
                .or_else(|| state.model_config.orchestrator_model.primary()),
            SelectedAgentType::Planner => self
                .pending_planner
                .or_else(|| state.model_config.planner_model.primary()),
            SelectedAgentType::Implementer => self
                .pending_implementer
                .or_else(|| state.model_config.implementer_model.primary()),
        }
    }

    /// Sets the pending model tier for the selected agent type.
    pub const fn set_pending_model(&mut self, tier: ModelTier) {
        match self.selected_agent_type {
            SelectedAgentType::Orchestrator => self.pending_orchestrator = Some(tier),
            SelectedAgentType::Planner => self.pending_planner = Some(tier),
            SelectedAgentType::Implementer => self.pending_implementer = Some(tier),
        }
    }

    /// Returns true if there are any pending changes.
    pub const fn has_pending_changes(&self) -> bool {
        self.pending_orchestrator.is_some()
            || self.pending_planner.is_some()
            || self.pending_implementer.is_some()
    }

    /// Clears all pending changes.
    pub const fn clear_pending(&mut self) {
        self.pending_orchestrator = None;
        self.pending_planner = None;
        self.pending_implementer = None;
    }
}

// ============================================================================
// Rendering
// ============================================================================

/// Renders the settings overlay on top of the main UI.
///
/// This function renders a centered popup for viewing and changing model
/// configurations. It uses the `Clear` widget to clear the area behind the
/// popup, ensuring the settings are readable.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render into
/// * `area` - The full terminal area to render the overlay on
/// * `state` - The TUI state containing model config and settings state
pub fn render_settings_overlay(frame: &mut Frame, area: Rect, state: &TuiState) {
    // Calculate popup dimensions
    let popup_area = centered_rect(area, 70, 85);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build the settings content
    let lines = build_settings_content(state);

    let settings_text = Text::from(lines);

    // Create the block with styled borders
    let block = Block::default()
        .title(" Model Settings ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::Magenta))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(settings_text).block(block);

    frame.render_widget(paragraph, popup_area);
}

/// Builds the content lines for the settings overlay.
fn build_settings_content(state: &TuiState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let settings = &state.settings_state;

    // Tab bar for agent types
    lines.push(build_tab_bar(settings.selected_agent_type));
    lines.push(Line::from(""));

    // Get current model tier for the selected agent type
    let current_tier = settings.get_current_model(state);
    let original_tier = get_original_model(state, settings.selected_agent_type);

    // Section header
    lines.push(Line::from(Span::styled(
        format!("{} Model", settings.selected_agent_type.display_name()),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    lines.push(Line::from(""));

    // Available model tiers list
    let available_tiers = &state.available_tiers;
    if available_tiers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No models available",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (idx, &tier) in available_tiers.iter().enumerate() {
            let is_selected = idx == settings.selected_model_index;
            let is_current = current_tier == Some(tier);
            let is_original = original_tier == Some(tier);
            lines.push(build_model_line(tier, is_selected, is_current, is_original));
        }
    }

    // Show pending changes indicator
    lines.push(Line::from(""));
    if settings.has_pending_changes() {
        lines.push(Line::from(Span::styled(
            "  ● Unsaved changes",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // Instructions at the bottom
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("←/→/Tab", Style::default().fg(Color::Yellow)),
        Span::raw(" Switch agent type  "),
        Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
        Span::raw(" Navigate  "),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" Select  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" Close"),
    ]));

    lines
}

/// Builds the tab bar showing agent type selection.
fn build_tab_bar(selected: SelectedAgentType) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::raw(" "));

    for agent_type in SelectedAgentType::all() {
        let is_selected = *agent_type == selected;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        spans.push(Span::styled(
            format!(" {} ", agent_type.display_name()),
            style,
        ));
        spans.push(Span::raw(" "));
    }

    Line::from(spans)
}

/// Builds a single model tier line with selection and status indicators.
fn build_model_line(
    tier: ModelTier,
    is_selected: bool,
    is_current: bool,
    is_original: bool,
) -> Line<'static> {
    let mut spans = Vec::new();

    // Selection indicator (cursor)
    let cursor = if is_selected { "▶ " } else { "  " };
    spans.push(Span::styled(
        cursor.to_string(),
        Style::default().fg(Color::Magenta),
    ));

    // Radio button indicator
    let radio = if is_current { "◉" } else { "○" };
    let radio_color = if is_current {
        Color::Green
    } else {
        Color::Gray
    };
    spans.push(Span::styled(
        format!("{radio} "),
        Style::default().fg(radio_color),
    ));

    // Tier name (capitalize first letter)
    let name = tier.as_str();
    let display_name = name
        .chars()
        .enumerate()
        .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
        .collect::<String>();
    let name_style = if is_selected {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if is_current {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::White)
    };
    spans.push(Span::styled(display_name, name_style));

    // Pending change indicator
    if is_current && !is_original {
        spans.push(Span::styled(
            " (pending)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    Line::from(spans)
}

/// Gets the original (non-pending) model tier for an agent type.
fn get_original_model(state: &TuiState, agent_type: SelectedAgentType) -> Option<ModelTier> {
    match agent_type {
        SelectedAgentType::Orchestrator => state.model_config.orchestrator_model.primary(),
        SelectedAgentType::Planner => state.model_config.planner_model.primary(),
        SelectedAgentType::Implementer => state.model_config.implementer_model.primary(),
    }
}

/// Creates a centered rectangle within the given area.
///
/// # Arguments
///
/// * `area` - The area to center within
/// * `percent_x` - Percentage of horizontal space to use
/// * `percent_y` - Percentage of vertical space to use
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

    // ========================================================================
    // SelectedAgentType Tests
    // ========================================================================

    #[test]
    fn test_selected_agent_type_display_name() {
        assert_eq!(
            SelectedAgentType::Orchestrator.display_name(),
            "Orchestrator"
        );
        assert_eq!(SelectedAgentType::Planner.display_name(), "Planner");
        assert_eq!(SelectedAgentType::Implementer.display_name(), "Implementer");
    }

    #[test]
    fn test_selected_agent_type_all() {
        let all = SelectedAgentType::all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], SelectedAgentType::Orchestrator);
        assert_eq!(all[1], SelectedAgentType::Planner);
        assert_eq!(all[2], SelectedAgentType::Implementer);
    }

    #[test]
    fn test_selected_agent_type_next_cycles() {
        assert_eq!(
            SelectedAgentType::Orchestrator.next(),
            SelectedAgentType::Planner
        );
        assert_eq!(
            SelectedAgentType::Planner.next(),
            SelectedAgentType::Implementer
        );
        assert_eq!(
            SelectedAgentType::Implementer.next(),
            SelectedAgentType::Orchestrator
        );
    }

    #[test]
    fn test_selected_agent_type_prev_cycles() {
        assert_eq!(
            SelectedAgentType::Orchestrator.prev(),
            SelectedAgentType::Implementer
        );
        assert_eq!(
            SelectedAgentType::Planner.prev(),
            SelectedAgentType::Orchestrator
        );
        assert_eq!(
            SelectedAgentType::Implementer.prev(),
            SelectedAgentType::Planner
        );
    }

    // ========================================================================
    // SettingsState Tests
    // ========================================================================

    #[test]
    fn test_settings_state_new() {
        let state = SettingsState::new();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Orchestrator);
        assert_eq!(state.selected_model_index, 0);
        assert!(state.pending_orchestrator.is_none());
        assert!(state.pending_planner.is_none());
        assert!(state.pending_implementer.is_none());
    }

    #[test]
    fn test_settings_state_select_next_model() {
        let mut state = SettingsState::new();
        assert_eq!(state.selected_model_index, 0);

        state.select_next_model(3);
        assert_eq!(state.selected_model_index, 1);

        state.select_next_model(3);
        assert_eq!(state.selected_model_index, 2);

        // Should wrap around
        state.select_next_model(3);
        assert_eq!(state.selected_model_index, 0);
    }

    #[test]
    fn test_settings_state_select_previous_model() {
        let mut state = SettingsState::new();
        state.selected_model_index = 2;

        state.select_previous_model(3);
        assert_eq!(state.selected_model_index, 1);

        state.select_previous_model(3);
        assert_eq!(state.selected_model_index, 0);

        // Should wrap around
        state.select_previous_model(3);
        assert_eq!(state.selected_model_index, 2);
    }

    #[test]
    fn test_settings_state_navigation_with_empty_list() {
        let mut state = SettingsState::new();

        // Should not panic with 0 models
        state.select_next_model(0);
        assert_eq!(state.selected_model_index, 0);

        state.select_previous_model(0);
        assert_eq!(state.selected_model_index, 0);
    }

    #[test]
    fn test_settings_state_next_agent_type() {
        let mut state = SettingsState::new();
        state.selected_model_index = 5; // Set some non-zero index

        state.next_agent_type();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Planner);
        assert_eq!(state.selected_model_index, 0); // Should reset

        state.next_agent_type();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Implementer);

        state.next_agent_type();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Orchestrator);
    }

    #[test]
    fn test_settings_state_prev_agent_type() {
        let mut state = SettingsState::new();
        state.selected_model_index = 5;

        state.prev_agent_type();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Implementer);
        assert_eq!(state.selected_model_index, 0);

        state.prev_agent_type();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Planner);

        state.prev_agent_type();
        assert_eq!(state.selected_agent_type, SelectedAgentType::Orchestrator);
    }

    #[test]
    fn test_settings_state_set_pending_model() {
        let mut state = SettingsState::new();

        // Test orchestrator
        state.selected_agent_type = SelectedAgentType::Orchestrator;
        state.set_pending_model(ModelTier::Sonnet);
        assert_eq!(state.pending_orchestrator, Some(ModelTier::Sonnet));

        // Test planner
        state.selected_agent_type = SelectedAgentType::Planner;
        state.set_pending_model(ModelTier::Haiku);
        assert_eq!(state.pending_planner, Some(ModelTier::Haiku));

        // Test implementer
        state.selected_agent_type = SelectedAgentType::Implementer;
        state.set_pending_model(ModelTier::Opus);
        assert_eq!(state.pending_implementer, Some(ModelTier::Opus));
    }

    #[test]
    fn test_settings_state_has_pending_changes() {
        let mut state = SettingsState::new();
        assert!(!state.has_pending_changes());

        state.pending_orchestrator = Some(ModelTier::Sonnet);
        assert!(state.has_pending_changes());

        state.clear_pending();
        assert!(!state.has_pending_changes());

        state.pending_planner = Some(ModelTier::Haiku);
        assert!(state.has_pending_changes());
    }

    #[test]
    fn test_settings_state_clear_pending() {
        let mut state = SettingsState::new();
        state.pending_orchestrator = Some(ModelTier::Sonnet);
        state.pending_planner = Some(ModelTier::Haiku);
        state.pending_implementer = Some(ModelTier::Opus);

        state.clear_pending();

        assert!(state.pending_orchestrator.is_none());
        assert!(state.pending_planner.is_none());
        assert!(state.pending_implementer.is_none());
    }

    // ========================================================================
    // get_current_model Tests
    // ========================================================================

    #[test]
    fn test_get_current_model_returns_config_when_no_pending() {
        use crate::models::{ModelConfig, ModelFallbackChain};

        let mut config = ModelConfig::default();
        config.orchestrator_model = ModelFallbackChain::single(ModelTier::Opus);
        config.planner_model = ModelFallbackChain::single(ModelTier::Sonnet);
        config.implementer_model = ModelFallbackChain::single(ModelTier::Haiku);

        let tui_state = TuiState::with_model_config(config);

        // Test orchestrator
        let mut settings = SettingsState::new();
        settings.selected_agent_type = SelectedAgentType::Orchestrator;
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Opus)
        );

        // Test planner
        settings.selected_agent_type = SelectedAgentType::Planner;
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Sonnet)
        );

        // Test implementer
        settings.selected_agent_type = SelectedAgentType::Implementer;
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Haiku)
        );
    }

    #[test]
    fn test_get_current_model_returns_pending_when_set() {
        use crate::models::{ModelConfig, ModelFallbackChain};

        let mut config = ModelConfig::default();
        config.orchestrator_model = ModelFallbackChain::single(ModelTier::Haiku);
        let tui_state = TuiState::with_model_config(config);

        let mut settings = SettingsState::new();
        settings.selected_agent_type = SelectedAgentType::Orchestrator;
        settings.pending_orchestrator = Some(ModelTier::Opus);

        // Should return pending, not config
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Opus)
        );
    }

    #[test]
    fn test_get_current_model_pending_per_agent_type() {
        use crate::models::{ModelConfig, ModelFallbackChain};

        let mut config = ModelConfig::default();
        config.orchestrator_model = ModelFallbackChain::single(ModelTier::Haiku);
        config.planner_model = ModelFallbackChain::single(ModelTier::Haiku);
        config.implementer_model = ModelFallbackChain::single(ModelTier::Haiku);
        let tui_state = TuiState::with_model_config(config);

        let mut settings = SettingsState::new();
        settings.pending_orchestrator = Some(ModelTier::Opus);
        settings.pending_planner = Some(ModelTier::Sonnet);
        // implementer has no pending

        settings.selected_agent_type = SelectedAgentType::Orchestrator;
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Opus)
        );

        settings.selected_agent_type = SelectedAgentType::Planner;
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Sonnet)
        );

        settings.selected_agent_type = SelectedAgentType::Implementer;
        assert_eq!(
            settings.get_current_model(&tui_state),
            Some(ModelTier::Haiku)
        ); // From config
    }
}
