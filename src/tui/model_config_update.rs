//! Model configuration update types for TUI -> App communication.
//!
//! This module contains the [`ModelConfigUpdate`] struct which is used
//! to send model configuration changes from the TUI to the main application.

use crate::models::ModelId;

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
    /// New orchestrator model (if changed)
    pub orchestrator_model: Option<ModelId>,
    /// New planner model (if changed)
    pub planner_model: Option<ModelId>,
    /// New implementer model (if changed)
    pub implementer_model: Option<ModelId>,
}

impl ModelConfigUpdate {
    /// Creates a new update with only the orchestrator model changed.
    #[allow(dead_code)]
    pub const fn orchestrator(model: ModelId) -> Self {
        Self {
            orchestrator_model: Some(model),
            planner_model: None,
            implementer_model: None,
        }
    }

    /// Creates a new update with only the planner model changed.
    #[allow(dead_code)]
    pub const fn planner(model: ModelId) -> Self {
        Self {
            orchestrator_model: None,
            planner_model: Some(model),
            implementer_model: None,
        }
    }

    /// Creates a new update with only the implementer model changed.
    #[allow(dead_code)]
    pub const fn implementer(model: ModelId) -> Self {
        Self {
            orchestrator_model: None,
            planner_model: None,
            implementer_model: Some(model),
        }
    }

    /// Creates a new update with all models changed.
    #[allow(dead_code)]
    pub const fn all(orchestrator: ModelId, planner: ModelId, implementer: ModelId) -> Self {
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
        let update = ModelConfigUpdate::orchestrator(ModelId::Opus4_5);

        assert_eq!(update.orchestrator_model, Some(ModelId::Opus4_5));
        assert!(update.planner_model.is_none());
        assert!(update.implementer_model.is_none());
    }

    #[test]
    fn test_model_config_update_planner() {
        let update = ModelConfigUpdate::planner(ModelId::Sonnet4_5);

        assert!(update.orchestrator_model.is_none());
        assert_eq!(update.planner_model, Some(ModelId::Sonnet4_5));
        assert!(update.implementer_model.is_none());
    }

    #[test]
    fn test_model_config_update_implementer() {
        let update = ModelConfigUpdate::implementer(ModelId::Haiku4_5);

        assert!(update.orchestrator_model.is_none());
        assert!(update.planner_model.is_none());
        assert_eq!(update.implementer_model, Some(ModelId::Haiku4_5));
    }

    #[test]
    fn test_model_config_update_all() {
        let update =
            ModelConfigUpdate::all(ModelId::Opus4_5, ModelId::Sonnet4_5, ModelId::Haiku4_5);

        assert_eq!(update.orchestrator_model, Some(ModelId::Opus4_5));
        assert_eq!(update.planner_model, Some(ModelId::Sonnet4_5));
        assert_eq!(update.implementer_model, Some(ModelId::Haiku4_5));
    }

    #[test]
    fn test_model_config_update_debug() {
        // Test Debug implementation
        let update = ModelConfigUpdate::orchestrator(ModelId::Opus4_5);
        let debug_str = format!("{:?}", update);
        assert!(debug_str.contains("Opus4_5"));
    }

    #[test]
    fn test_model_config_update_clone() {
        let update =
            ModelConfigUpdate::all(ModelId::Opus4_5, ModelId::Sonnet4_5, ModelId::Haiku4_5);
        let cloned = update.clone();

        assert_eq!(update.orchestrator_model, cloned.orchestrator_model);
        assert_eq!(update.planner_model, cloned.planner_model);
        assert_eq!(update.implementer_model, cloned.implementer_model);
    }
}
