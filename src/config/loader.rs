//! Configuration file loading for Paperboat
//!
//! This module handles loading agent configuration from TOML files.
//! Configuration is loaded from two locations, with project-level
//! settings taking priority over user-level settings:
//!
//! 1. User-level: `~/.paperboat/agents/`
//! 2. Project-level: `.paperboat/agents/`
//!
//! Each agent type has its own configuration file:
//! - `orchestrator.toml`
//! - `planner.toml`
//! - `implementer.toml`

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::config::resolver::resolve_model;
use crate::error::{suggest_model_alias, ConfigError, KNOWN_MODEL_ALIASES};
use crate::models::{AvailableModel, ModelConfig, ModelId};

/// Configuration for a single agent loaded from TOML
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentFileConfig {
    /// Model to use for this agent (e.g., "opus", "sonnet4.5")
    pub model: Option<String>,
}

impl AgentFileConfig {
    /// Validates the configuration values.
    ///
    /// Checks that all specified values are valid, including:
    /// - Model alias is a known value (opus, sonnet, haiku, etc.)
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::InvalidModel` if the model alias is not recognized.
    #[allow(dead_code)]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(ref model) = self.model {
            self.validate_model(model, None)?;
        }
        Ok(())
    }

    /// Validates the configuration with a file path for better error messages.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::InvalidModel` if the model alias is not recognized,
    /// with the file path included in the error message.
    pub fn validate_with_path(&self, file_path: &Path) -> Result<(), ConfigError> {
        if let Some(ref model) = self.model {
            self.validate_model(model, Some(file_path))?;
        }
        Ok(())
    }

    /// Validates a model alias against known values.
    fn validate_model(&self, model: &str, file_path: Option<&Path>) -> Result<(), ConfigError> {
        let model_lower = model.trim().to_lowercase();

        // Check if it's a known alias
        if KNOWN_MODEL_ALIASES.contains(&model_lower.as_str()) {
            return Ok(());
        }

        // Check if it looks like a versioned model (family + version number)
        // by extracting the family part and checking if that's known
        let family = model_lower
            .chars()
            .take_while(|c| c.is_alphabetic())
            .collect::<String>();

        let known_families = ["opus", "sonnet", "haiku", "gpt"];
        if known_families.contains(&family.as_str()) {
            // It's a versioned model like "sonnet4" or "opus5" - allow it
            // The actual resolution will validate against available models later
            return Ok(());
        }

        // Invalid model - try to suggest a correction
        let suggestion = suggest_model_alias(&model_lower);

        if let Some(path) = file_path {
            Err(ConfigError::invalid_model_in_file(
                model,
                suggestion.as_deref(),
                path,
            ))
        } else {
            Err(ConfigError::invalid_model(model, suggestion.as_deref()))
        }
    }
}

/// Loaded configuration for all agents
#[derive(Debug, Clone, Default)]
pub struct LoadedAgentConfigs {
    pub orchestrator: AgentFileConfig,
    pub planner: AgentFileConfig,
    pub implementer: AgentFileConfig,
}

/// Returns the user-level config directory path (~/.paperboat/agents/)
fn user_config_dir() -> PathBuf {
    let home = shellexpand::tilde("~/.paperboat/agents").to_string();
    PathBuf::from(home)
}

/// Returns the project-level config directory path (.paperboat/agents/)
fn project_config_dir() -> PathBuf {
    PathBuf::from(".paperboat/agents")
}

/// Loads a single agent config file from the given path.
///
/// After loading, validates the configuration and returns an error if validation fails.
fn load_agent_config(path: &Path) -> Result<AgentFileConfig> {
    if !path.exists() {
        return Ok(AgentFileConfig::default());
    }

    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: AgentFileConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    // Validate the config immediately after loading, with file path for better errors
    config
        .validate_with_path(path)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(config)
}

/// Merges two agent configs, with `override_config` taking priority
fn merge_agent_config(base: AgentFileConfig, override_config: AgentFileConfig) -> AgentFileConfig {
    AgentFileConfig {
        model: override_config.model.or(base.model),
    }
}

/// Loads all agent configurations from both user and project directories.
/// Project-level settings override user-level settings.
pub fn load_agent_configs() -> Result<LoadedAgentConfigs> {
    let user_dir = user_config_dir();
    let project_dir = project_config_dir();

    // Load user-level configs
    let user_orchestrator = load_agent_config(&user_dir.join("orchestrator.toml"))?;
    let user_planner = load_agent_config(&user_dir.join("planner.toml"))?;
    let user_implementer = load_agent_config(&user_dir.join("implementer.toml"))?;

    // Load project-level configs
    let project_orchestrator = load_agent_config(&project_dir.join("orchestrator.toml"))?;
    let project_planner = load_agent_config(&project_dir.join("planner.toml"))?;
    let project_implementer = load_agent_config(&project_dir.join("implementer.toml"))?;

    // Merge with project taking priority
    Ok(LoadedAgentConfigs {
        orchestrator: merge_agent_config(user_orchestrator, project_orchestrator),
        planner: merge_agent_config(user_planner, project_planner),
        implementer: merge_agent_config(user_implementer, project_implementer),
    })
}

/// Builds a `ModelConfig` from loaded agent configurations and available models.
/// Resolves model strings (e.g., "opus") to concrete `ModelIds` (e.g., "opus4.5").
/// The special value "auto" is handled directly without resolution.
pub fn build_model_config(
    loaded: &LoadedAgentConfigs,
    available_models: &[AvailableModel],
) -> Result<ModelConfig> {
    let mut config = ModelConfig::new(available_models.to_vec());

    // Resolve orchestrator model
    if let Some(ref model_str) = loaded.orchestrator.model {
        config.orchestrator_model = resolve_model_string(model_str, available_models)
            .with_context(|| format!("Failed to resolve orchestrator model '{model_str}'"))?;
    }

    // Resolve planner model
    if let Some(ref model_str) = loaded.planner.model {
        config.planner_model = resolve_model_string(model_str, available_models)
            .with_context(|| format!("Failed to resolve planner model '{model_str}'"))?;
    }

    // Resolve implementer model
    if let Some(ref model_str) = loaded.implementer.model {
        config.implementer_model = resolve_model_string(model_str, available_models)
            .with_context(|| format!("Failed to resolve implementer model '{model_str}'"))?;
    }

    Ok(config)
}

/// Resolves a model string to a `ModelId`.
/// Handles "auto" directly, otherwise uses the resolver.
fn resolve_model_string(model_str: &str, available_models: &[AvailableModel]) -> Result<ModelId> {
    // Handle "auto" directly without going through resolve_model
    if model_str.trim().eq_ignore_ascii_case("auto") {
        return Ok(ModelId::Auto);
    }

    // Normal resolution for other model strings
    let resolved = resolve_model(model_str, available_models)?;
    resolved.parse::<ModelId>()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // AgentFileConfig Tests
    // ========================================================================

    #[test]
    fn test_agent_file_config_default() {
        let config = AgentFileConfig::default();
        assert!(config.model.is_none());
    }

    #[test]
    fn test_agent_file_config_deserialization() {
        let toml_content = r#"model = "opus""#;
        let config: AgentFileConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.model, Some("opus".to_string()));
    }

    #[test]
    fn test_agent_file_config_deserialization_empty() {
        let toml_content = "";
        let config: AgentFileConfig = toml::from_str(toml_content).unwrap();
        assert!(config.model.is_none());
    }

    // ========================================================================
    // AgentFileConfig Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_known_aliases() {
        // Test all known model aliases pass validation
        for alias in &[
            "opus",
            "sonnet",
            "haiku",
            "opus4.5",
            "sonnet4.5",
            "haiku4.5",
            "auto",
        ] {
            let config = AgentFileConfig {
                model: Some(alias.to_string()),
            };
            assert!(
                config.validate().is_ok(),
                "Expected '{}' to be a valid alias",
                alias
            );
        }
    }

    #[test]
    fn test_validate_case_insensitive() {
        // Test that validation is case-insensitive
        let config = AgentFileConfig {
            model: Some("OPUS".to_string()),
        };
        assert!(config.validate().is_ok());

        let config = AgentFileConfig {
            model: Some("Sonnet".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_with_whitespace() {
        let config = AgentFileConfig {
            model: Some("  opus  ".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_versioned_models() {
        // Versioned models should be allowed
        let config = AgentFileConfig {
            model: Some("sonnet4".to_string()),
        };
        assert!(config.validate().is_ok());

        let config = AgentFileConfig {
            model: Some("opus5".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_model() {
        let config = AgentFileConfig {
            model: Some("invalid_model".to_string()),
        };
        let result = config.validate();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let display = format!("{err}");
        // Uses format: "Invalid model '{model}': {reason}. Available models: ..."
        assert!(
            display.contains("Invalid model"),
            "Expected 'Invalid model' in: {display}"
        );
        assert!(
            display.contains("invalid_model"),
            "Expected model name in: {display}"
        );
    }

    #[test]
    fn test_validate_typo_with_suggestion() {
        // Use a typo that has a suggestion in suggest_model_alias
        let config = AgentFileConfig {
            model: Some("sonnett".to_string()),
        };
        let result = config.validate();
        assert!(result.is_err());

        let err = result.unwrap_err();
        let display = format!("{err}");
        assert!(
            display.contains("Did you mean 'sonnet'?"),
            "Expected suggestion in: {display}"
        );
    }

    #[test]
    fn test_validate_empty_model_is_ok() {
        // No model specified should be fine (uses default)
        let config = AgentFileConfig { model: None };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_with_path_includes_file() {
        let config = AgentFileConfig {
            model: Some("badmodel".to_string()),
        };
        let path = PathBuf::from(".paperboat/agents/orchestrator.toml");
        let result = config.validate_with_path(&path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let display = format!("{err}");
        assert!(display.contains(".paperboat/agents/orchestrator.toml"));
    }

    // ========================================================================
    // load_agent_config Tests (with temp files)
    // ========================================================================

    #[test]
    fn test_load_agent_config_nonexistent_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("nonexistent.toml");

        let config = load_agent_config(&path).unwrap();
        assert!(config.model.is_none());
    }

    #[test]
    fn test_load_agent_config_existing_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("orchestrator.toml");

        std::fs::write(&path, "model = \"sonnet\"").unwrap();

        let config = load_agent_config(&path).unwrap();
        assert_eq!(config.model, Some("sonnet".to_string()));
    }

    #[test]
    fn test_load_agent_config_with_comments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("planner.toml");

        let content = r#"
# This is a comment
model = "opus"  # inline comment
"#;
        std::fs::write(&path, content).unwrap();

        let config = load_agent_config(&path).unwrap();
        assert_eq!(config.model, Some("opus".to_string()));
    }

    #[test]
    fn test_load_agent_config_invalid_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("invalid.toml");

        std::fs::write(&path, "this is not valid toml {{{").unwrap();

        let result = load_agent_config(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_agent_config_invalid_model_alias() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("invalid_model.toml");

        std::fs::write(&path, r#"model = "badmodel""#).unwrap();

        let result = load_agent_config(&path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let display = format!("{err}");
        // Error format: "Invalid model 'badmodel': not recognized..."
        assert!(
            display.contains("Invalid model"),
            "Expected 'Invalid model' in: {display}"
        );
    }

    #[test]
    fn test_load_agent_config_typo_model_with_suggestion() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("typo_model.toml");

        std::fs::write(&path, r#"model = "sonnett""#).unwrap();

        let result = load_agent_config(&path);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let display = format!("{err}");
        assert!(
            display.contains("Did you mean 'sonnet'?"),
            "Expected suggestion for 'sonnett', got: {display}"
        );
    }

    // ========================================================================
    // merge_agent_config Tests
    // ========================================================================

    #[test]
    fn test_merge_agent_config_override_takes_priority() {
        let base = AgentFileConfig {
            model: Some("haiku".to_string()),
        };
        let override_config = AgentFileConfig {
            model: Some("opus".to_string()),
        };

        let merged = merge_agent_config(base, override_config);
        assert_eq!(merged.model, Some("opus".to_string()));
    }

    #[test]
    fn test_merge_agent_config_base_used_when_override_empty() {
        let base = AgentFileConfig {
            model: Some("sonnet".to_string()),
        };
        let override_config = AgentFileConfig { model: None };

        let merged = merge_agent_config(base, override_config);
        assert_eq!(merged.model, Some("sonnet".to_string()));
    }

    #[test]
    fn test_merge_agent_config_both_empty() {
        let base = AgentFileConfig { model: None };
        let override_config = AgentFileConfig { model: None };

        let merged = merge_agent_config(base, override_config);
        assert!(merged.model.is_none());
    }

    // ========================================================================
    // build_model_config Tests
    // ========================================================================

    #[test]
    fn test_build_model_config_default() {
        let loaded = LoadedAgentConfigs::default();
        let available_models = vec![AvailableModel {
            id: ModelId::Opus4_5,
            name: "Opus 4.5".to_string(),
            description: "Most capable".to_string(),
        }];

        let config = build_model_config(&loaded, &available_models).unwrap();

        // Should use ModelConfig::new() defaults when no model specified
        // ModelConfig defaults: orchestrator=Opus4_5, planner=Sonnet4_5, implementer=Sonnet4_5
        let default_config = crate::models::ModelConfig::default();
        assert_eq!(config.orchestrator_model, default_config.orchestrator_model);
        assert_eq!(config.planner_model, default_config.planner_model);
        assert_eq!(config.implementer_model, default_config.implementer_model);
    }

    #[test]
    fn test_build_model_config_with_model_strings() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("opus".to_string()),
            },
            planner: AgentFileConfig {
                model: Some("sonnet".to_string()),
            },
            implementer: AgentFileConfig {
                model: Some("haiku".to_string()),
            },
        };

        let available_models = vec![
            AvailableModel {
                id: ModelId::Opus4_5,
                name: "Opus 4.5".to_string(),
                description: "".to_string(),
            },
            AvailableModel {
                id: ModelId::Sonnet4_5,
                name: "Sonnet 4.5".to_string(),
                description: "".to_string(),
            },
            AvailableModel {
                id: ModelId::Haiku4_5,
                name: "Haiku 4.5".to_string(),
                description: "".to_string(),
            },
        ];

        let config = build_model_config(&loaded, &available_models).unwrap();

        assert_eq!(config.orchestrator_model, ModelId::Opus4_5);
        assert_eq!(config.planner_model, ModelId::Sonnet4_5);
        assert_eq!(config.implementer_model, ModelId::Haiku4_5);
    }

    #[test]
    fn test_build_model_config_partial_settings() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("opus".to_string()),
            },
            planner: AgentFileConfig { model: None },
            implementer: AgentFileConfig { model: None },
        };

        let available_models = vec![AvailableModel {
            id: ModelId::Opus4_5,
            name: "Opus 4.5".to_string(),
            description: "".to_string(),
        }];

        let config = build_model_config(&loaded, &available_models).unwrap();

        // Only orchestrator should be changed
        assert_eq!(config.orchestrator_model, ModelId::Opus4_5);
        assert_eq!(config.planner_model, ModelId::default());
        assert_eq!(config.implementer_model, ModelId::default());
    }

    #[test]
    fn test_build_model_config_invalid_model_string() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("invalid_model_name".to_string()),
            },
            planner: AgentFileConfig { model: None },
            implementer: AgentFileConfig { model: None },
        };

        let available_models = vec![AvailableModel {
            id: ModelId::Opus4_5,
            name: "Opus 4.5".to_string(),
            description: "".to_string(),
        }];

        // Should fail because "invalid_model_name" doesn't match any available model
        let result = build_model_config(&loaded, &available_models);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_model_config_auto() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("auto".to_string()),
            },
            planner: AgentFileConfig {
                model: Some("AUTO".to_string()), // Test case insensitivity
            },
            implementer: AgentFileConfig {
                model: Some(" Auto ".to_string()), // Test whitespace handling
            },
        };

        // Auto should work even with empty available_models
        let available_models = vec![];

        let config = build_model_config(&loaded, &available_models).unwrap();

        assert_eq!(config.orchestrator_model, ModelId::Auto);
        assert_eq!(config.planner_model, ModelId::Auto);
        assert_eq!(config.implementer_model, ModelId::Auto);
    }

    // ========================================================================
    // LoadedAgentConfigs Tests
    // ========================================================================

    #[test]
    fn test_loaded_agent_configs_default() {
        let configs = LoadedAgentConfigs::default();

        assert!(configs.orchestrator.model.is_none());
        assert!(configs.planner.model.is_none());
        assert!(configs.implementer.model.is_none());
    }
}
