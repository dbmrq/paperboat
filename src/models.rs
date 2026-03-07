//! Model configuration and discovery for Villalobos
//!
//! This module provides types for managing AI model configuration,
//! including model discovery via the `auggie model list` command.

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tokio::process::Command;

/// Known model identifiers with their CLI id strings
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelId {
    /// Auto mode: allows the orchestrator to choose the model based on task complexity.
    /// When set, the system will dynamically select an appropriate model for each task.
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "haiku4.5")]
    Haiku4_5,
    #[serde(rename = "opus4.5")]
    Opus4_5,
    #[serde(rename = "sonnet4")]
    Sonnet4,
    #[serde(rename = "sonnet4.5")]
    #[default]
    Sonnet4_5,
    #[serde(rename = "gpt5")]
    Gpt5,
    #[serde(rename = "gpt5.1")]
    Gpt5_1,
}

impl ModelId {
    /// Returns the CLI id string for this model
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Haiku4_5 => "haiku4.5",
            Self::Opus4_5 => "opus4.5",
            Self::Sonnet4 => "sonnet4",
            Self::Sonnet4_5 => "sonnet4.5",
            Self::Gpt5 => "gpt5",
            Self::Gpt5_1 => "gpt5.1",
        }
    }

    /// Returns all known model IDs (including Auto)
    #[allow(dead_code)]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Auto,
            Self::Haiku4_5,
            Self::Opus4_5,
            Self::Sonnet4,
            Self::Sonnet4_5,
            Self::Gpt5,
            Self::Gpt5_1,
        ]
    }

    /// Returns `true` if this is the `Auto` variant
    pub const fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Resolves the "auto" model to a concrete model based on complexity.
    ///
    /// If this model is not "auto", returns itself unchanged.
    /// If this model is "auto", maps the complexity to an appropriate model:
    /// - Simple → Haiku 4.5 (fast, efficient)
    /// - Medium → Sonnet 4.5 (balanced)
    /// - Complex → Opus 4.5 (most capable)
    ///
    /// If complexity is None and model is Auto, defaults to Sonnet 4.5.
    #[allow(clippy::missing_const_for_fn)] // Uses imported type in match, not const-compatible
    pub fn resolve_auto(&self, complexity: Option<crate::mcp_server::ModelComplexity>) -> Self {
        use crate::mcp_server::ModelComplexity;

        if !self.is_auto() {
            return *self;
        }

        // Each case intentionally mapped explicitly for clarity:
        // - Simple tasks get fast/cheap model (Haiku)
        // - Medium tasks get balanced model (Sonnet)
        // - Complex tasks get most capable model (Opus)
        // - Default (None) uses Sonnet as a safe middle ground
        #[allow(clippy::match_same_arms)]
        match complexity {
            Some(ModelComplexity::Simple) => Self::Haiku4_5,
            Some(ModelComplexity::Medium) => Self::Sonnet4_5,
            Some(ModelComplexity::Complex) => Self::Opus4_5,
            None => Self::Sonnet4_5,
        }
    }
}

impl FromStr for ModelId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(Self::Auto),
            "haiku4.5" => Ok(Self::Haiku4_5),
            "opus4.5" => Ok(Self::Opus4_5),
            "sonnet4" => Ok(Self::Sonnet4),
            "sonnet4.5" => Ok(Self::Sonnet4_5),
            "gpt5" => Ok(Self::Gpt5),
            "gpt5.1" => Ok(Self::Gpt5_1),
            _ => Err(anyhow!("Unknown model id: {s}")),
        }
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// An available model discovered from the CLI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableModel {
    /// The model identifier
    pub id: ModelId,
    /// Human-readable name (e.g., "Haiku 4.5")
    pub name: String,
    /// Description of the model's capabilities (e.g., "Fast and efficient responses")
    pub description: String,
}

/// Configuration for which models to use for different roles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// List of available models discovered from the CLI
    pub available_models: Vec<AvailableModel>,
    /// Model to use for orchestration (default: `Opus4_5` - best for complex tasks)
    pub orchestrator_model: ModelId,
    /// Model to use for planning (default: `Sonnet4_5` - great for everyday tasks)
    pub planner_model: ModelId,
    /// Model to use for implementation (default: `Sonnet4_5` - great for everyday tasks)
    pub implementer_model: ModelId,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            available_models: Vec::new(),
            orchestrator_model: ModelId::Opus4_5,
            planner_model: ModelId::Sonnet4_5,
            implementer_model: ModelId::Sonnet4_5,
        }
    }
}

impl ModelConfig {
    /// Creates a new `ModelConfig` with sensible defaults and the given available models
    pub const fn new(available_models: Vec<AvailableModel>) -> Self {
        Self {
            available_models,
            orchestrator_model: ModelId::Opus4_5,
            planner_model: ModelId::Sonnet4_5,
            implementer_model: ModelId::Sonnet4_5,
        }
    }

    /// Applies debug build model override.
    ///
    /// In debug builds, all models default to Haiku for fast, cheap testing.
    /// This can be overridden by setting the `PAPERBOAT_MODEL` environment variable.
    ///
    /// In release builds, this is a no-op (respects user configuration).
    #[cfg(debug_assertions)]
    pub fn apply_debug_override(&mut self) {
        // Check for environment variable override first
        if let Ok(model_str) = std::env::var("PAPERBOAT_MODEL") {
            if let Ok(model_id) = ModelId::from_str(&model_str) {
                tracing::info!(
                    "🧪 PAPERBOAT_MODEL override: using {} for all agents",
                    model_id
                );
                self.orchestrator_model = model_id;
                self.planner_model = model_id;
                self.implementer_model = model_id;
                return;
            }
            tracing::warn!(
                "⚠️  Invalid PAPERBOAT_MODEL '{}', falling back to debug default (haiku)",
                model_str
            );
        }

        // Debug build default: use Haiku for all agents (cheap and fast)
        tracing::info!(
            "🧪 Debug build: using haiku4.5 for all agents (override with PAPERBOAT_MODEL)"
        );
        self.orchestrator_model = ModelId::Haiku4_5;
        self.planner_model = ModelId::Haiku4_5;
        self.implementer_model = ModelId::Haiku4_5;
    }

    /// Applies debug build model override (no-op in release builds).
    #[cfg(not(debug_assertions))]
    #[allow(clippy::missing_const_for_fn)] // Paired with debug version which cannot be const
    pub fn apply_debug_override(&mut self) {
        // Release build: respect user configuration
    }

    /// Validates that all selected models are in the available list.
    /// The `Auto` variant is always valid (it will be resolved at runtime).
    pub fn validate(&self) -> Result<()> {
        let available_ids: Vec<ModelId> = self.available_models.iter().map(|m| m.id).collect();

        // Auto is always valid - it will be resolved at runtime
        if !self.orchestrator_model.is_auto() && !available_ids.contains(&self.orchestrator_model) {
            return Err(anyhow!(
                "Orchestrator model '{}' is not available",
                self.orchestrator_model
            ));
        }

        if !self.planner_model.is_auto() && !available_ids.contains(&self.planner_model) {
            return Err(anyhow!(
                "Planner model '{}' is not available",
                self.planner_model
            ));
        }

        if !self.implementer_model.is_auto() && !available_ids.contains(&self.implementer_model) {
            return Err(anyhow!(
                "Implementer model '{}' is not available",
                self.implementer_model
            ));
        }

        Ok(())
    }
}

/// Discovers available models by running `auggie model list`
///
/// Parses output in the format:
/// ```text
///  - Haiku 4.5 [haiku4.5]
///      Fast and efficient responses
/// ```
pub async fn discover_models() -> Result<Vec<AvailableModel>> {
    let output = Command::new("auggie")
        .args(["model", "list"])
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run 'auggie model list': {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "auggie model list failed with status {}: {}",
            output.status,
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_model_list(&stdout)
}

/// Parses the output of `auggie model list` into `AvailableModel` structs
fn parse_model_list(output: &str) -> Result<Vec<AvailableModel>> {
    let mut models = Vec::new();

    // Pattern to match lines like " - Haiku 4.5 [haiku4.5]"
    let model_re = Regex::new(r"^\s*-\s*(.+?)\s*\[([^\]]+)\]\s*$")?;

    let lines: Vec<&str> = output.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        if let Some(caps) = model_re.captures(lines[i]) {
            let name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
            let id_str = caps.get(2).map_or("", |m| m.as_str()).trim();

            // Try to parse the model ID - skip unknown models
            if let Ok(id) = ModelId::from_str(id_str) {
                // Look for description on the next line(s)
                let mut description = String::new();
                let mut j = i + 1;
                while j < lines.len() {
                    let line = lines[j].trim();
                    // Stop if we hit another model line or empty line
                    if model_re.is_match(lines[j]) || line.is_empty() {
                        break;
                    }
                    if !description.is_empty() {
                        description.push(' ');
                    }
                    description.push_str(line);
                    j += 1;
                }

                models.push(AvailableModel {
                    id,
                    name,
                    description,
                });
            }
        }
        i += 1;
    }

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // ModelId Tests
    // ========================================================================

    #[test]
    fn test_model_id_as_str() {
        assert_eq!(ModelId::Auto.as_str(), "auto");
        assert_eq!(ModelId::Haiku4_5.as_str(), "haiku4.5");
        assert_eq!(ModelId::Opus4_5.as_str(), "opus4.5");
        assert_eq!(ModelId::Sonnet4.as_str(), "sonnet4");
        assert_eq!(ModelId::Sonnet4_5.as_str(), "sonnet4.5");
        assert_eq!(ModelId::Gpt5.as_str(), "gpt5");
        assert_eq!(ModelId::Gpt5_1.as_str(), "gpt5.1");
    }

    #[test]
    fn test_model_id_from_str() {
        assert_eq!(ModelId::from_str("auto").unwrap(), ModelId::Auto);
        assert_eq!(ModelId::from_str("haiku4.5").unwrap(), ModelId::Haiku4_5);
        assert_eq!(ModelId::from_str("opus4.5").unwrap(), ModelId::Opus4_5);
        assert_eq!(ModelId::from_str("sonnet4").unwrap(), ModelId::Sonnet4);
        assert_eq!(ModelId::from_str("sonnet4.5").unwrap(), ModelId::Sonnet4_5);
        assert_eq!(ModelId::from_str("gpt5").unwrap(), ModelId::Gpt5);
        assert_eq!(ModelId::from_str("gpt5.1").unwrap(), ModelId::Gpt5_1);
    }

    #[test]
    fn test_model_id_from_str_invalid() {
        assert!(ModelId::from_str("invalid").is_err());
        assert!(ModelId::from_str("gpt4").is_err());
        assert!(ModelId::from_str("").is_err());
    }

    #[test]
    fn test_model_id_default() {
        assert_eq!(ModelId::default(), ModelId::Sonnet4_5);
    }

    #[test]
    fn test_model_id_display() {
        assert_eq!(format!("{}", ModelId::Opus4_5), "opus4.5");
        assert_eq!(format!("{}", ModelId::Sonnet4_5), "sonnet4.5");
    }

    #[test]
    fn test_model_id_serde_roundtrip() {
        for model_id in ModelId::all() {
            let json = serde_json::to_string(model_id).unwrap();
            let parsed: ModelId = serde_json::from_str(&json).unwrap();
            assert_eq!(*model_id, parsed);
        }
    }

    #[test]
    fn test_model_id_serde_format() {
        let json = serde_json::to_string(&ModelId::Sonnet4_5).unwrap();
        assert_eq!(json, "\"sonnet4.5\"");

        let json = serde_json::to_string(&ModelId::Gpt5_1).unwrap();
        assert_eq!(json, "\"gpt5.1\"");
    }

    #[test]
    fn test_model_id_all() {
        let all = ModelId::all();
        assert_eq!(all.len(), 7);
        assert!(all.contains(&ModelId::Auto));
        assert!(all.contains(&ModelId::Haiku4_5));
        assert!(all.contains(&ModelId::Opus4_5));
        assert!(all.contains(&ModelId::Sonnet4));
        assert!(all.contains(&ModelId::Sonnet4_5));
        assert!(all.contains(&ModelId::Gpt5));
        assert!(all.contains(&ModelId::Gpt5_1));
    }

    #[test]
    fn test_model_id_is_auto() {
        assert!(ModelId::Auto.is_auto());
        assert!(!ModelId::Haiku4_5.is_auto());
        assert!(!ModelId::Opus4_5.is_auto());
        assert!(!ModelId::Sonnet4.is_auto());
        assert!(!ModelId::Sonnet4_5.is_auto());
        assert!(!ModelId::Gpt5.is_auto());
        assert!(!ModelId::Gpt5_1.is_auto());
    }

    #[test]
    fn test_model_id_auto_serde() {
        let json = serde_json::to_string(&ModelId::Auto).unwrap();
        assert_eq!(json, "\"auto\"");

        let parsed: ModelId = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(parsed, ModelId::Auto);
    }

    #[test]
    fn test_model_id_resolve_auto_simple() {
        use crate::mcp_server::ModelComplexity;
        let model = ModelId::Auto;
        let resolved = model.resolve_auto(Some(ModelComplexity::Simple));
        assert_eq!(resolved, ModelId::Haiku4_5);
    }

    #[test]
    fn test_model_id_resolve_auto_medium() {
        use crate::mcp_server::ModelComplexity;
        let model = ModelId::Auto;
        let resolved = model.resolve_auto(Some(ModelComplexity::Medium));
        assert_eq!(resolved, ModelId::Sonnet4_5);
    }

    #[test]
    fn test_model_id_resolve_auto_complex() {
        use crate::mcp_server::ModelComplexity;
        let model = ModelId::Auto;
        let resolved = model.resolve_auto(Some(ModelComplexity::Complex));
        assert_eq!(resolved, ModelId::Opus4_5);
    }

    #[test]
    fn test_model_id_resolve_auto_none_defaults_to_medium() {
        let model = ModelId::Auto;
        let resolved = model.resolve_auto(None);
        assert_eq!(resolved, ModelId::Sonnet4_5);
    }

    #[test]
    fn test_model_id_resolve_auto_non_auto_unchanged() {
        use crate::mcp_server::ModelComplexity;

        // Non-auto models should be unchanged regardless of complexity
        assert_eq!(
            ModelId::Haiku4_5.resolve_auto(Some(ModelComplexity::Complex)),
            ModelId::Haiku4_5
        );
        assert_eq!(
            ModelId::Opus4_5.resolve_auto(Some(ModelComplexity::Simple)),
            ModelId::Opus4_5
        );
        assert_eq!(ModelId::Sonnet4_5.resolve_auto(None), ModelId::Sonnet4_5);
    }

    // ========================================================================
    // AvailableModel Tests
    // ========================================================================

    #[test]
    fn test_available_model_serde() {
        let model = AvailableModel {
            id: ModelId::Haiku4_5,
            name: "Haiku 4.5".to_string(),
            description: "Fast and efficient responses".to_string(),
        };

        let json = serde_json::to_string(&model).unwrap();
        let parsed: AvailableModel = serde_json::from_str(&json).unwrap();

        assert_eq!(model, parsed);
    }

    // ========================================================================
    // ModelConfig Tests
    // ========================================================================

    #[test]
    fn test_model_config_default() {
        let config = ModelConfig::default();
        assert!(config.available_models.is_empty());
        assert_eq!(config.orchestrator_model, ModelId::Opus4_5);
        assert_eq!(config.planner_model, ModelId::Sonnet4_5);
        assert_eq!(config.implementer_model, ModelId::Sonnet4_5);
    }

    #[test]
    fn test_model_config_new() {
        let models = vec![AvailableModel {
            id: ModelId::Sonnet4_5,
            name: "Sonnet 4.5".to_string(),
            description: "Great for everyday tasks".to_string(),
        }];

        let config = ModelConfig::new(models.clone());
        assert_eq!(config.available_models, models);
        assert_eq!(config.orchestrator_model, ModelId::Opus4_5);
    }

    #[test]
    fn test_model_config_validate_success() {
        let models = vec![
            AvailableModel {
                id: ModelId::Opus4_5,
                name: "Opus 4.5".to_string(),
                description: "Best for complex tasks".to_string(),
            },
            AvailableModel {
                id: ModelId::Sonnet4_5,
                name: "Sonnet 4.5".to_string(),
                description: "Great for everyday tasks".to_string(),
            },
        ];

        let config = ModelConfig::new(models);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_model_config_validate_missing_orchestrator() {
        let models = vec![AvailableModel {
            id: ModelId::Sonnet4_5,
            name: "Sonnet 4.5".to_string(),
            description: "Great for everyday tasks".to_string(),
        }];

        let config = ModelConfig::new(models);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("Orchestrator model"));
    }

    #[test]
    fn test_model_config_validate_missing_planner() {
        let models = vec![AvailableModel {
            id: ModelId::Opus4_5,
            name: "Opus 4.5".to_string(),
            description: "Best for complex tasks".to_string(),
        }];

        let mut config = ModelConfig::new(models);
        config.orchestrator_model = ModelId::Opus4_5;
        config.planner_model = ModelId::Haiku4_5; // Not in available

        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("Planner model"));
    }

    #[test]
    fn test_model_config_validate_auto_always_valid() {
        // Auto should be valid even if not in available_models list
        let models = vec![AvailableModel {
            id: ModelId::Sonnet4_5,
            name: "Sonnet 4.5".to_string(),
            description: "Great for everyday tasks".to_string(),
        }];

        let mut config = ModelConfig::new(models);
        config.orchestrator_model = ModelId::Auto;
        config.planner_model = ModelId::Auto;
        config.implementer_model = ModelId::Auto;

        // Should pass validation because Auto is always valid
        assert!(config.validate().is_ok());
    }

    // ========================================================================
    // parse_model_list Tests
    // ========================================================================

    #[test]
    fn test_parse_model_list_single() {
        let output = " - Haiku 4.5 [haiku4.5]\n      Fast and efficient responses";
        let models = parse_model_list(output).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, ModelId::Haiku4_5);
        assert_eq!(models[0].name, "Haiku 4.5");
        assert_eq!(models[0].description, "Fast and efficient responses");
    }

    #[test]
    fn test_parse_model_list_multiple() {
        let output = r" - Haiku 4.5 [haiku4.5]
      Fast and efficient responses
 - Opus 4.5 [opus4.5]
      Best for complex tasks
 - Sonnet 4.5 [sonnet4.5]
      Great for everyday tasks";

        let models = parse_model_list(output).unwrap();

        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, ModelId::Haiku4_5);
        assert_eq!(models[1].id, ModelId::Opus4_5);
        assert_eq!(models[2].id, ModelId::Sonnet4_5);
    }

    #[test]
    fn test_parse_model_list_skips_unknown() {
        let output = r" - Haiku 4.5 [haiku4.5]
      Fast and efficient responses
 - Unknown Model [unknown-model]
      This should be skipped
 - Sonnet 4.5 [sonnet4.5]
      Great for everyday tasks";

        let models = parse_model_list(output).unwrap();

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, ModelId::Haiku4_5);
        assert_eq!(models[1].id, ModelId::Sonnet4_5);
    }

    #[test]
    fn test_parse_model_list_empty() {
        let output = "";
        let models = parse_model_list(output).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn test_parse_model_list_multiline_description() {
        let output = r" - Opus 4.5 [opus4.5]
      Best for complex tasks
      with multiple lines
      of description";

        let models = parse_model_list(output).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].description,
            "Best for complex tasks with multiple lines of description"
        );
    }

    // ========================================================================
    // Debug Override Tests
    // ========================================================================

    // Use a mutex to serialize tests that modify PAPERBOAT_MODEL env var
    // Only needed for debug-mode tests
    #[cfg(debug_assertions)]
    use std::sync::Mutex;
    #[cfg(debug_assertions)]
    static ENV_VAR_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_sets_haiku() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Clear any env var that might interfere
        std::env::remove_var("PAPERBOAT_MODEL");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        assert_eq!(config.orchestrator_model, ModelId::Haiku4_5);
        assert_eq!(config.planner_model, ModelId::Haiku4_5);
        assert_eq!(config.implementer_model, ModelId::Haiku4_5);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_respects_env_var() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set env var to override
        std::env::set_var("PAPERBOAT_MODEL", "sonnet4.5");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        assert_eq!(config.orchestrator_model, ModelId::Sonnet4_5);
        assert_eq!(config.planner_model, ModelId::Sonnet4_5);
        assert_eq!(config.implementer_model, ModelId::Sonnet4_5);

        // Clean up
        std::env::remove_var("PAPERBOAT_MODEL");
    }

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_invalid_env_var_falls_back() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set invalid env var
        std::env::set_var("PAPERBOAT_MODEL", "invalid-model");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        // Should fall back to Haiku
        assert_eq!(config.orchestrator_model, ModelId::Haiku4_5);
        assert_eq!(config.planner_model, ModelId::Haiku4_5);
        assert_eq!(config.implementer_model, ModelId::Haiku4_5);

        // Clean up
        std::env::remove_var("PAPERBOAT_MODEL");
    }
}
