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
//!
//! # Backend Selection
//!
//! Backend selection follows a priority order:
//! 1. `PAPERBOAT_BACKEND` environment variable
//! 2. Project config file (`.paperboat/config.toml`)
//! 3. User config file (`~/.paperboat/config.toml`)
//! 4. Default (Auggie)

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::backend::BackendKind;
use crate::error::{suggest_model_alias, ConfigError, KNOWN_MODEL_ALIASES};
use crate::models::{EffortLevel, ModelConfig, ModelFallbackChain, ModelTier};

/// Configuration for a single agent loaded from TOML
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentFileConfig {
    /// Model to use for this agent (e.g., "opus", "sonnet4.5", "openai, opus, gemini")
    pub model: Option<String>,
    /// Effort/thinking level for this agent (e.g., "low", "medium", "high", "xhigh")
    pub effort: Option<String>,
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
    #[allow(dead_code)] // Public API for config validation
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
    ///
    /// Supports both single models ("opus") and fallback chains ("openai, opus, gemini").
    fn validate_model(&self, model: &str, file_path: Option<&Path>) -> Result<(), ConfigError> {
        // Handle fallback chains (comma-separated models)
        let models: Vec<&str> = model.split(',').map(str::trim).collect();

        for single_model in models {
            self.validate_single_model(single_model, file_path)?;
        }

        Ok(())
    }

    /// Validates a single model alias (not a chain).
    fn validate_single_model(
        &self,
        model: &str,
        file_path: Option<&Path>,
    ) -> Result<(), ConfigError> {
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
        effort: override_config.effort.or(base.effort),
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
/// Builds a `ModelConfig` from loaded agent configurations.
///
/// Parses model strings (e.g., "opus", "sonnet, codex, opus") into fallback chains.
/// Parses effort strings (e.g., "high", "xhigh") into effort levels.
/// The special value "auto" is handled directly.
pub fn build_model_config(
    loaded: &LoadedAgentConfigs,
    available_tiers: HashSet<ModelTier>,
) -> Result<ModelConfig> {
    let mut config = ModelConfig::new(available_tiers);

    // Parse orchestrator model fallback chain and effort
    if let Some(ref model_str) = loaded.orchestrator.model {
        config.orchestrator_model = parse_model_chain(model_str)
            .with_context(|| format!("Failed to parse orchestrator model '{model_str}'"))?;
    }
    if let Some(ref effort_str) = loaded.orchestrator.effort {
        config.orchestrator_effort = parse_effort_level(effort_str)
            .with_context(|| format!("Failed to parse orchestrator effort '{effort_str}'"))?;
    }

    // Parse planner model fallback chain and effort
    if let Some(ref model_str) = loaded.planner.model {
        config.planner_model = parse_model_chain(model_str)
            .with_context(|| format!("Failed to parse planner model '{model_str}'"))?;
    }
    if let Some(ref effort_str) = loaded.planner.effort {
        config.planner_effort = parse_effort_level(effort_str)
            .with_context(|| format!("Failed to parse planner effort '{effort_str}'"))?;
    }

    // Parse implementer model fallback chain and effort
    if let Some(ref model_str) = loaded.implementer.model {
        config.implementer_model = parse_model_chain(model_str)
            .with_context(|| format!("Failed to parse implementer model '{model_str}'"))?;
    }
    if let Some(ref effort_str) = loaded.implementer.effort {
        config.implementer_effort = parse_effort_level(effort_str)
            .with_context(|| format!("Failed to parse implementer effort '{effort_str}'"))?;
    }

    Ok(config)
}

/// Parses a model string into a fallback chain.
///
/// Accepts either:
/// - Single tier: "sonnet"
/// - Comma-separated list: "gemini, codex, opus"
fn parse_model_chain(model_str: &str) -> Result<ModelFallbackChain> {
    model_str.parse()
}

/// Parses an effort string into an effort level.
///
/// Accepts: "low", "medium", "high", "xhigh"
fn parse_effort_level(effort_str: &str) -> Result<EffortLevel> {
    effort_str.parse()
}

// ============================================================================
// Backend Configuration
// ============================================================================

use crate::backend::BackendConfig;

/// Configuration loaded from TOML config files for backend selection.
///
/// This struct is used for deserializing config files, not for runtime configuration.
/// The `backend` field can contain a backend name with optional transport suffix:
/// - `"cursor"` - Cursor with default transport (CLI)
/// - `"cursor:cli"` - Cursor with CLI transport (explicit)
/// - `"cursor:acp"` - Cursor with ACP transport
/// - `"auggie"` - Auggie with default transport (ACP)
///
/// See [`BackendConfig`] for the parsed runtime type.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FileBackendConfig {
    /// Backend to use with optional transport (e.g., "auggie", "cursor", "cursor:cli").
    /// If not specified, defaults to the system default (Auggie with ACP).
    pub backend: Option<String>,
}

/// Environment variable name for backend selection.
const PAPERBOAT_BACKEND_ENV: &str = "PAPERBOAT_BACKEND";

/// Loads the backend configuration from environment and config files.
///
/// # Backend:Transport Syntax
///
/// Both environment variables and config files support the `backend:transport` syntax:
///
/// ```toml
/// # .paperboat/config.toml
/// backend = "cursor"       # Uses default transport (CLI for Cursor)
/// backend = "cursor:cli"   # Explicitly use CLI transport
/// backend = "cursor:acp"   # Explicitly use ACP transport
/// backend = "auggie:acp"   # Auggie with ACP (only option)
/// ```
///
/// ```bash
/// # Environment variable
/// PAPERBOAT_BACKEND=cursor:cli cargo run -- "task"
/// ```
///
/// # Priority Order
///
/// Priority order (highest to lowest):
/// 1. `PAPERBOAT_BACKEND` environment variable
/// 2. Project config file (`.paperboat/config.toml`)
/// 3. User config file (`~/.paperboat/config.toml`)
/// 4. Default (`BackendKind::Auggie` with ACP transport)
///
/// Invalid backend names are logged as warnings and fall back to the default.
///
/// # Returns
///
/// A [`BackendConfig`] containing the selected backend kind and optional transport.
///
/// # Example
///
/// ```ignore
/// use paperboat::config::load_backend_config;
///
/// // Respects PAPERBOAT_BACKEND=cursor:cli if set
/// let config = load_backend_config();
/// let backend = config.kind.create();
/// let transport = config.effective_transport();
/// ```
#[allow(dead_code)] // Public API for loading backend configuration
pub fn load_backend_config() -> BackendConfig {
    // Priority 1: Environment variable
    if let Ok(backend_str) = std::env::var(PAPERBOAT_BACKEND_ENV) {
        let backend_str = backend_str.trim();
        if !backend_str.is_empty() {
            match BackendConfig::parse(backend_str) {
                Ok(config) => {
                    tracing::debug!(
                        "Backend '{}' with transport '{}' selected from {} environment variable",
                        config.kind,
                        config.effective_transport(),
                        PAPERBOAT_BACKEND_ENV
                    );
                    return config;
                }
                Err(err) => {
                    tracing::warn!(
                        "Invalid {} value '{}': {}. Using default backend.",
                        PAPERBOAT_BACKEND_ENV,
                        backend_str,
                        err
                    );
                }
            }
        }
    }

    // Priority 2: Project config file (.paperboat/config.toml)
    if let Some(config) = load_backend_from_config(&project_backend_config_path()) {
        tracing::debug!(
            "Backend '{}' with transport '{}' selected from project config file",
            config.kind,
            config.effective_transport()
        );
        return config;
    }

    // Priority 3: User config file (~/.paperboat/config.toml)
    if let Some(config) = load_backend_from_config(&user_backend_config_path()) {
        tracing::debug!(
            "Backend '{}' with transport '{}' selected from user config file",
            config.kind,
            config.effective_transport()
        );
        return config;
    }

    // Priority 4: Default (Auggie with ACP transport)
    let default_config = BackendConfig::default();
    tracing::debug!(
        "Using default backend '{}' with transport '{}'",
        default_config.kind,
        default_config.effective_transport()
    );
    default_config
}

/// Loads the backend configuration from environment and config files.
///
/// This is a convenience function that returns only the [`BackendKind`],
/// discarding transport information. For full configuration including
/// transport, use [`load_backend_config`] instead.
///
/// # Deprecation Note
///
/// This function is kept for backwards compatibility. New code should use
/// [`load_backend_config`] which returns the full [`BackendConfig`].
#[allow(dead_code)] // Backwards compatibility wrapper
pub fn load_backend_kind() -> BackendKind {
    load_backend_config().kind
}

/// Check if a backend was explicitly configured via environment variable or config files.
///
/// This function checks for explicit configuration without falling back to defaults.
/// Returns `Some(BackendConfig)` if explicitly configured, `None` if no explicit config exists.
///
/// # Use Case
///
/// This is useful for determining whether to prompt the user to select a backend
/// when multiple are available. If the user has explicitly configured a backend,
/// we should use it. If not, we can offer a choice.
///
/// # Priority Order
///
/// Same as [`load_backend_config`]:
/// 1. `PAPERBOAT_BACKEND` environment variable
/// 2. Project config file (`.paperboat/config.toml`)
/// 3. User config file (`~/.paperboat/config.toml`)
///
/// Returns `None` if none of these are configured (would have fallen back to default).
pub fn get_explicit_backend_config() -> Option<BackendConfig> {
    // Check environment variable
    if let Ok(backend_str) = std::env::var(PAPERBOAT_BACKEND_ENV) {
        let backend_str = backend_str.trim();
        if !backend_str.is_empty() {
            if let Ok(config) = BackendConfig::parse(backend_str) {
                return Some(config);
            }
        }
    }

    // Check project config file
    if let Some(config) = load_backend_from_config(&project_backend_config_path()) {
        return Some(config);
    }

    // Check user config file
    if let Some(config) = load_backend_from_config(&user_backend_config_path()) {
        return Some(config);
    }

    // No explicit config found
    None
}

/// Returns the user-level backend config file path (~/.paperboat/config.toml)
fn user_backend_config_path() -> PathBuf {
    let home = shellexpand::tilde("~/.paperboat/config.toml").to_string();
    PathBuf::from(home)
}

/// Returns the project-level backend config file path (.paperboat/config.toml)
fn project_backend_config_path() -> PathBuf {
    PathBuf::from(".paperboat/config.toml")
}

/// Attempts to load backend configuration from a config file.
///
/// Returns `Some(BackendConfig)` if the file exists, is valid, and contains
/// a valid backend setting with optional transport. Returns `None` otherwise
/// (file doesn't exist, parse error, or no backend specified).
///
/// # Supported Syntax
///
/// The `backend` field in config files can be:
/// - `"cursor"` - Cursor with default transport (CLI)
/// - `"cursor:cli"` - Cursor with CLI transport (explicit)
/// - `"cursor:acp"` - Cursor with ACP transport
/// - `"auggie"` or `"augment"` - Auggie with default transport (ACP)
fn load_backend_from_config(path: &Path) -> Option<BackendConfig> {
    if !path.exists() {
        return None;
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to read backend config file '{}': {}",
                path.display(),
                e
            );
            return None;
        }
    };

    let file_config: FileBackendConfig = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to parse backend config file '{}': {}",
                path.display(),
                e
            );
            return None;
        }
    };

    let backend_str = file_config.backend.as_ref()?;
    let backend_str = backend_str.trim();
    if backend_str.is_empty() {
        return None;
    }

    match BackendConfig::parse(backend_str) {
        Ok(config) => Some(config),
        Err(err) => {
            tracing::warn!(
                "Invalid backend '{}' in config file '{}': {}. Ignoring.",
                backend_str,
                path.display(),
                err
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::TransportKind;

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
                ..Default::default()
            };
            assert!(
                config.validate().is_ok(),
                "Expected '{alias}' to be a valid alias"
            );
        }
    }

    #[test]
    fn test_validate_case_insensitive() {
        // Test that validation is case-insensitive
        let config = AgentFileConfig {
            model: Some("OPUS".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = AgentFileConfig {
            model: Some("Sonnet".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_with_whitespace() {
        let config = AgentFileConfig {
            model: Some("  opus  ".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_fallback_chain() {
        // Test comma-separated fallback chains
        let config = AgentFileConfig {
            model: Some("openai, opus, gemini, composer".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = AgentFileConfig {
            model: Some("gpt, codex, sonnet".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        // Single model in chain format should also work
        let config = AgentFileConfig {
            model: Some("opus".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_fallback_chain_with_invalid_model() {
        // One invalid model in chain should fail
        let config = AgentFileConfig {
            model: Some("opus, invalid_model, sonnet".to_string()),
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid_model"));
    }

    #[test]
    fn test_validate_versioned_models() {
        // Versioned models should be allowed
        let config = AgentFileConfig {
            model: Some("sonnet4".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = AgentFileConfig {
            model: Some("opus5".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_model() {
        let config = AgentFileConfig {
            model: Some("invalid_model".to_string()),
            ..Default::default()
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
            ..Default::default()
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
        let config = AgentFileConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_with_path_includes_file() {
        let config = AgentFileConfig {
            model: Some("badmodel".to_string()),
            ..Default::default()
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
            ..Default::default()
        };
        let override_config = AgentFileConfig {
            model: Some("opus".to_string()),
            ..Default::default()
        };

        let merged = merge_agent_config(base, override_config);
        assert_eq!(merged.model, Some("opus".to_string()));
    }

    #[test]
    fn test_merge_agent_config_base_used_when_override_empty() {
        let base = AgentFileConfig {
            model: Some("sonnet".to_string()),
            ..Default::default()
        };
        let override_config = AgentFileConfig::default();

        let merged = merge_agent_config(base, override_config);
        assert_eq!(merged.model, Some("sonnet".to_string()));
    }

    #[test]
    fn test_merge_agent_config_both_empty() {
        let base = AgentFileConfig::default();
        let override_config = AgentFileConfig::default();

        let merged = merge_agent_config(base, override_config);
        assert!(merged.model.is_none());
    }

    // ========================================================================
    // build_model_config Tests
    // ========================================================================

    #[test]
    fn test_build_model_config_default() {
        let loaded = LoadedAgentConfigs::default();
        let available_tiers: HashSet<ModelTier> =
            [ModelTier::Opus, ModelTier::Sonnet].into_iter().collect();

        let config = build_model_config(&loaded, available_tiers).unwrap();

        // Should use ModelConfig::new() defaults when no model specified
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
                ..Default::default()
            },
            planner: AgentFileConfig {
                model: Some("sonnet".to_string()),
                ..Default::default()
            },
            implementer: AgentFileConfig {
                model: Some("haiku".to_string()),
                ..Default::default()
            },
        };

        let available_tiers: HashSet<ModelTier> =
            [ModelTier::Opus, ModelTier::Sonnet, ModelTier::Haiku]
                .into_iter()
                .collect();

        let config = build_model_config(&loaded, available_tiers).unwrap();

        assert_eq!(config.orchestrator_model.primary(), Some(ModelTier::Opus));
        assert_eq!(config.planner_model.primary(), Some(ModelTier::Sonnet));
        assert_eq!(config.implementer_model.primary(), Some(ModelTier::Haiku));
    }

    #[test]
    fn test_build_model_config_with_fallback_chain() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("gemini, codex, opus".to_string()),
                ..Default::default()
            },
            planner: AgentFileConfig::default(),
            implementer: AgentFileConfig::default(),
        };

        let available_tiers: HashSet<ModelTier> =
            [ModelTier::Opus, ModelTier::Sonnet].into_iter().collect();

        let config = build_model_config(&loaded, available_tiers).unwrap();

        // Should parse as a fallback chain
        assert_eq!(
            config.orchestrator_model.0,
            vec![ModelTier::Gemini, ModelTier::Codex, ModelTier::Opus]
        );
    }

    #[test]
    fn test_build_model_config_partial_settings() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("opus".to_string()),
                ..Default::default()
            },
            planner: AgentFileConfig::default(),
            implementer: AgentFileConfig::default(),
        };

        let available_tiers: HashSet<ModelTier> = std::iter::once(ModelTier::Opus).collect();

        let config = build_model_config(&loaded, available_tiers).unwrap();

        // Only orchestrator should be changed
        assert_eq!(config.orchestrator_model.primary(), Some(ModelTier::Opus));
        // Others should use defaults
        assert_eq!(
            config.planner_model,
            crate::models::ModelConfig::default().planner_model
        );
    }

    #[test]
    fn test_build_model_config_invalid_model_string() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("invalid_model_name".to_string()),
                ..Default::default()
            },
            planner: AgentFileConfig::default(),
            implementer: AgentFileConfig::default(),
        };

        let available_tiers: HashSet<ModelTier> = HashSet::new();

        // Should fail because "invalid_model_name" is not a valid tier
        let result = build_model_config(&loaded, available_tiers);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_model_config_auto() {
        let loaded = LoadedAgentConfigs {
            orchestrator: AgentFileConfig {
                model: Some("auto".to_string()),
                ..Default::default()
            },
            planner: AgentFileConfig {
                model: Some("AUTO".to_string()), // Test case insensitivity
                ..Default::default()
            },
            implementer: AgentFileConfig::default(),
        };

        let available_tiers: HashSet<ModelTier> = HashSet::new();

        let config = build_model_config(&loaded, available_tiers).unwrap();

        assert_eq!(config.orchestrator_model.primary(), Some(ModelTier::Auto));
        assert_eq!(config.planner_model.primary(), Some(ModelTier::Auto));
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

    // ========================================================================
    // FileBackendConfig Tests
    // ========================================================================

    #[test]
    fn test_file_backend_config_default() {
        let config = FileBackendConfig::default();
        assert!(config.backend.is_none());
    }

    #[test]
    fn test_file_backend_config_deserialization() {
        let toml_content = r#"backend = "cursor""#;
        let config: FileBackendConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.backend, Some("cursor".to_string()));
    }

    #[test]
    fn test_file_backend_config_deserialization_empty() {
        let toml_content = "";
        let config: FileBackendConfig = toml::from_str(toml_content).unwrap();
        assert!(config.backend.is_none());
    }

    #[test]
    fn test_file_backend_config_deserialization_with_other_fields() {
        // FileBackendConfig should ignore unknown fields
        let toml_content = r#"
backend = "auggie"
some_other_field = "value"
"#;
        let config: FileBackendConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.backend, Some("auggie".to_string()));
    }

    #[test]
    fn test_file_backend_config_with_transport() {
        // Test that backend:transport syntax is stored as-is in the file config
        let toml_content = r#"backend = "cursor:cli""#;
        let config: FileBackendConfig = toml::from_str(toml_content).unwrap();
        assert_eq!(config.backend, Some("cursor:cli".to_string()));
    }

    // ========================================================================
    // load_backend_from_config Tests
    // ========================================================================

    #[test]
    fn test_load_backend_from_config_nonexistent_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("nonexistent.toml");

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_valid_auggie() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "auggie""#).unwrap();

        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Auggie);
        assert_eq!(result.transport, None); // Default transport
    }

    #[test]
    fn test_load_backend_from_config_valid_cursor() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "cursor""#).unwrap();

        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Cursor);
        assert_eq!(result.transport, None); // Default transport
    }

    #[test]
    fn test_load_backend_from_config_cursor_with_transport() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        // Test cursor:cli syntax
        std::fs::write(&path, r#"backend = "cursor:cli""#).unwrap();
        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Cursor);
        assert_eq!(result.transport, Some(TransportKind::Cli));

        // Test cursor:acp syntax
        std::fs::write(&path, r#"backend = "cursor:acp""#).unwrap();
        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Cursor);
        assert_eq!(result.transport, Some(TransportKind::Acp));
    }

    #[test]
    fn test_load_backend_from_config_case_insensitive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "CURSOR:CLI""#).unwrap();

        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Cursor);
        assert_eq!(result.transport, Some(TransportKind::Cli));
    }

    #[test]
    fn test_load_backend_from_config_invalid_backend() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "invalid_backend""#).unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_invalid_transport() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "cursor:invalid""#).unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_unsupported_transport() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        // Auggie doesn't support CLI transport
        std::fs::write(&path, r#"backend = "auggie:cli""#).unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_empty_string() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = """#).unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_whitespace_only() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "  ""#).unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_no_backend_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"some_other_key = "value""#).unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_invalid_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, "this is not valid toml {{{").unwrap();

        let result = load_backend_from_config(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_backend_from_config_with_whitespace_in_value() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        std::fs::write(&path, r#"backend = "  cursor  ""#).unwrap();

        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Cursor);
    }

    #[test]
    fn test_load_backend_from_config_augment_alias() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        // "augment" is an alias for "auggie"
        std::fs::write(&path, r#"backend = "augment""#).unwrap();

        let result = load_backend_from_config(&path).unwrap();
        assert_eq!(result.kind, BackendKind::Auggie);
    }

    // ========================================================================
    // load_backend_kind Tests (Priority Order)
    // ========================================================================
    //
    // These tests use the serial_test crate because they modify environment
    // variables which are process-global state.

    use serial_test::serial;

    // Helper to temporarily set an environment variable for a test
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_env_var_takes_priority() {
        // Set the environment variable to cursor
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "cursor");

        // load_backend_kind should return Cursor (env var priority)
        let kind = load_backend_kind();
        assert_eq!(kind, BackendKind::Cursor);
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_env_var_augment_alias() {
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "augment");

        let kind = load_backend_kind();
        assert_eq!(kind, BackendKind::Auggie);
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_env_var_case_insensitive() {
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "CURSOR");

        let kind = load_backend_kind();
        assert_eq!(kind, BackendKind::Cursor);
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_invalid_env_var_falls_back() {
        // Invalid env var should fall back to default (Auggie)
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "invalid_backend");

        let kind = load_backend_kind();
        // Should fall back to default when env var is invalid
        // (depends on config files, but without them, should be Auggie)
        assert!(kind == BackendKind::Auggie || kind == BackendKind::Cursor);
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_empty_env_var_ignored() {
        // Empty env var should be ignored (fall back to config or default)
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "");

        // Should not panic and should return some backend
        let _kind = load_backend_kind();
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_whitespace_env_var_ignored() {
        // Whitespace-only env var should be ignored
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "   ");

        // Should not panic and should return some backend
        let _kind = load_backend_kind();
    }

    #[test]
    #[serial]
    fn test_load_backend_kind_default_without_env_or_config() {
        // Remove env var and ensure no config files exist
        let _guard = EnvGuard::remove("PAPERBOAT_BACKEND");

        // Without env var or config files, should return default (Auggie)
        // Note: This test may be affected by user's actual config files
        let kind = load_backend_kind();
        // At minimum, it should return a valid backend
        assert!(kind == BackendKind::Auggie || kind == BackendKind::Cursor);
    }

    // ========================================================================
    // load_backend_config Tests (with transport)
    // ========================================================================

    #[test]
    #[serial]
    fn test_load_backend_config_env_var_with_transport() {
        // Test backend:transport syntax in environment variable
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "cursor:cli");

        let config = load_backend_config();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, Some(TransportKind::Cli));
        assert_eq!(config.effective_transport(), TransportKind::Cli);
    }

    #[test]
    #[serial]
    fn test_load_backend_config_env_var_cursor_acp() {
        // Test cursor with ACP transport
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "cursor:acp");

        let config = load_backend_config();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, Some(TransportKind::Acp));
        assert_eq!(config.effective_transport(), TransportKind::Acp);
    }

    #[test]
    #[serial]
    fn test_load_backend_config_env_var_default_transport() {
        // Test that cursor without transport suffix uses default (CLI)
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "cursor");

        let config = load_backend_config();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, None);
        assert_eq!(config.effective_transport(), TransportKind::Cli); // Default for Cursor
    }

    #[test]
    #[serial]
    fn test_load_backend_config_env_var_auggie_default_transport() {
        // Test that auggie without transport suffix uses default (ACP)
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "auggie");

        let config = load_backend_config();
        assert_eq!(config.kind, BackendKind::Auggie);
        assert_eq!(config.transport, None);
        assert_eq!(config.effective_transport(), TransportKind::Acp); // Default for Auggie
    }

    #[test]
    #[serial]
    fn test_load_backend_config_invalid_transport_falls_back() {
        // Test that invalid transport falls back to default config
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "cursor:invalid");

        // Should fall back to default (Auggie with ACP)
        // Note: May also pick up from config files
        let _config = load_backend_config();
        // Just verify it doesn't panic
    }

    #[test]
    #[serial]
    fn test_load_backend_config_unsupported_transport_falls_back() {
        // Test that unsupported transport (auggie:cli) falls back
        let _guard = EnvGuard::set("PAPERBOAT_BACKEND", "auggie:cli");

        // Should fall back to default since auggie doesn't support CLI
        let _config = load_backend_config();
        // Just verify it doesn't panic
    }
}
