//! Model configuration update types for TUI -> App communication.
//!
//! This module contains the [`ModelConfigUpdate`] struct which is used
//! to send model configuration changes from the TUI to the main application.

use crate::models::ModelTier;

// ============================================================================
// Model Configuration Updates
// ============================================================================

/// Message type for sending model configuration updates from the TUI to the App.
///
/// This enables the TUI to modify the model configuration at runtime,
/// with changes taking effect for newly spawned agents.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)] // Field names match ModelConfig for clarity
pub struct ModelConfigUpdate {
    /// New orchestrator model tier (if changed)
    pub orchestrator_model: Option<ModelTier>,
    /// New planner model tier (if changed)
    pub planner_model: Option<ModelTier>,
    /// New implementer model tier (if changed)
    pub implementer_model: Option<ModelTier>,
}

impl ModelConfigUpdate {
    /// Creates a new update with only the orchestrator model changed.
    #[allow(dead_code)] // Builder method for partial updates
    pub const fn orchestrator(model: ModelTier) -> Self {
        Self {
            orchestrator_model: Some(model),
            planner_model: None,
            implementer_model: None,
        }
    }

    /// Creates a new update with only the planner model changed.
    #[allow(dead_code)] // Builder method for partial updates
    pub const fn planner(model: ModelTier) -> Self {
        Self {
            orchestrator_model: None,
            planner_model: Some(model),
            implementer_model: None,
        }
    }

    /// Creates a new update with only the implementer model changed.
    #[allow(dead_code)] // Builder method for partial updates
    pub const fn implementer(model: ModelTier) -> Self {
        Self {
            orchestrator_model: None,
            planner_model: None,
            implementer_model: Some(model),
        }
    }

    /// Creates a new update with all models changed.
    #[allow(dead_code)] // Builder method for full updates
    pub const fn all(orchestrator: ModelTier, planner: ModelTier, implementer: ModelTier) -> Self {
        Self {
            orchestrator_model: Some(orchestrator),
            planner_model: Some(planner),
            implementer_model: Some(implementer),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_config_update_orchestrator() {
        let update = ModelConfigUpdate::orchestrator(ModelTier::Opus);

        assert_eq!(update.orchestrator_model, Some(ModelTier::Opus));
        assert!(update.planner_model.is_none());
        assert!(update.implementer_model.is_none());
    }

    #[test]
    fn test_model_config_update_planner() {
        let update = ModelConfigUpdate::planner(ModelTier::Sonnet);

        assert!(update.orchestrator_model.is_none());
        assert_eq!(update.planner_model, Some(ModelTier::Sonnet));
        assert!(update.implementer_model.is_none());
    }

    #[test]
    fn test_model_config_update_implementer() {
        let update = ModelConfigUpdate::implementer(ModelTier::Haiku);

        assert!(update.orchestrator_model.is_none());
        assert!(update.planner_model.is_none());
        assert_eq!(update.implementer_model, Some(ModelTier::Haiku));
    }

    #[test]
    fn test_model_config_update_all() {
        let update = ModelConfigUpdate::all(ModelTier::Opus, ModelTier::Sonnet, ModelTier::Haiku);

        assert_eq!(update.orchestrator_model, Some(ModelTier::Opus));
        assert_eq!(update.planner_model, Some(ModelTier::Sonnet));
        assert_eq!(update.implementer_model, Some(ModelTier::Haiku));
    }

    #[test]
    fn test_model_config_update_debug() {
        // Test Debug implementation
        let update = ModelConfigUpdate::orchestrator(ModelTier::Opus);
        let debug_str = format!("{update:?}");
        assert!(debug_str.contains("Opus"));
    }

    #[test]
    fn test_model_config_update_clone() {
        let update = ModelConfigUpdate::all(ModelTier::Opus, ModelTier::Sonnet, ModelTier::Haiku);
        let cloned = update;

        assert_eq!(update.orchestrator_model, cloned.orchestrator_model);
        assert_eq!(update.planner_model, cloned.planner_model);
        assert_eq!(update.implementer_model, cloned.implementer_model);
    }
}
