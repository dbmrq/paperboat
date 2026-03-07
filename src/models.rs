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
            Self::Haiku4_5 => "haiku4.5",
            Self::Opus4_5 => "opus4.5",
            Self::Sonnet4 => "sonnet4",
            Self::Sonnet4_5 => "sonnet4.5",
            Self::Gpt5 => "gpt5",
            Self::Gpt5_1 => "gpt5.1",
        }
    }

    /// Returns all known model IDs
    #[allow(dead_code)]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Haiku4_5,
            Self::Opus4_5,
            Self::Sonnet4,
            Self::Sonnet4_5,
            Self::Gpt5,
            Self::Gpt5_1,
        ]
    }
}

impl FromStr for ModelId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
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
    /// This can be overridden by setting the `VILLALOBOS_MODEL` environment variable.
    ///
    /// In release builds, this is a no-op (respects user configuration).
    #[cfg(debug_assertions)]
    pub fn apply_debug_override(&mut self) {
        // Check for environment variable override first
        if let Ok(model_str) = std::env::var("VILLALOBOS_MODEL") {
            if let Ok(model_id) = ModelId::from_str(&model_str) {
                tracing::info!(
                    "🧪 VILLALOBOS_MODEL override: using {} for all agents",
                    model_id
                );
                self.orchestrator_model = model_id;
                self.planner_model = model_id;
                self.implementer_model = model_id;
                return;
            }
            tracing::warn!(
                "⚠️  Invalid VILLALOBOS_MODEL '{}', falling back to debug default (haiku)",
                model_str
            );
        }

        // Debug build default: use Haiku for all agents (cheap and fast)
        tracing::info!(
            "🧪 Debug build: using haiku4.5 for all agents (override with VILLALOBOS_MODEL)"
        );
        self.orchestrator_model = ModelId::Haiku4_5;
        self.planner_model = ModelId::Haiku4_5;
        self.implementer_model = ModelId::Haiku4_5;
    }

    /// Applies debug build model override (no-op in release builds).
    #[cfg(not(debug_assertions))]
    pub fn apply_debug_override(&mut self) {
        // Release build: respect user configuration
    }

    /// Validates that all selected models are in the available list
    pub fn validate(&self) -> Result<()> {
        let available_ids: Vec<ModelId> = self.available_models.iter().map(|m| m.id).collect();

        if !available_ids.contains(&self.orchestrator_model) {
            return Err(anyhow!(
                "Orchestrator model '{}' is not available",
                self.orchestrator_model
            ));
        }

        if !available_ids.contains(&self.planner_model) {
            return Err(anyhow!(
                "Planner model '{}' is not available",
                self.planner_model
            ));
        }

        if !available_ids.contains(&self.implementer_model) {
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
        assert_eq!(ModelId::Haiku4_5.as_str(), "haiku4.5");
        assert_eq!(ModelId::Opus4_5.as_str(), "opus4.5");
        assert_eq!(ModelId::Sonnet4.as_str(), "sonnet4");
        assert_eq!(ModelId::Sonnet4_5.as_str(), "sonnet4.5");
        assert_eq!(ModelId::Gpt5.as_str(), "gpt5");
        assert_eq!(ModelId::Gpt5_1.as_str(), "gpt5.1");
    }

    #[test]
    fn test_model_id_from_str() {
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
        assert_eq!(all.len(), 6);
        assert!(all.contains(&ModelId::Haiku4_5));
        assert!(all.contains(&ModelId::Opus4_5));
        assert!(all.contains(&ModelId::Sonnet4));
        assert!(all.contains(&ModelId::Sonnet4_5));
        assert!(all.contains(&ModelId::Gpt5));
        assert!(all.contains(&ModelId::Gpt5_1));
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

    // Use a mutex to serialize tests that modify VILLALOBOS_MODEL env var
    use std::sync::Mutex;
    static ENV_VAR_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_sets_haiku() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Clear any env var that might interfere
        std::env::remove_var("VILLALOBOS_MODEL");

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
        std::env::set_var("VILLALOBOS_MODEL", "sonnet4.5");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        assert_eq!(config.orchestrator_model, ModelId::Sonnet4_5);
        assert_eq!(config.planner_model, ModelId::Sonnet4_5);
        assert_eq!(config.implementer_model, ModelId::Sonnet4_5);

        // Clean up
        std::env::remove_var("VILLALOBOS_MODEL");
    }

    #[test]
    #[cfg(debug_assertions)]
    fn test_apply_debug_override_invalid_env_var_falls_back() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set invalid env var
        std::env::set_var("VILLALOBOS_MODEL", "invalid-model");

        let mut config = ModelConfig::default();
        config.apply_debug_override();

        // Should fall back to Haiku
        assert_eq!(config.orchestrator_model, ModelId::Haiku4_5);
        assert_eq!(config.planner_model, ModelId::Haiku4_5);
        assert_eq!(config.implementer_model, ModelId::Haiku4_5);

        // Clean up
        std::env::remove_var("VILLALOBOS_MODEL");
    }
}
