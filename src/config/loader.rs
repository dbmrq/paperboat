//! Configuration file loading for Villalobos
//!
//! This module handles loading agent configuration from TOML files.
//! Configuration is loaded from two locations, with project-level
//! settings taking priority over user-level settings:
//!
//! 1. User-level: `~/.villalobos/agents/`
//! 2. Project-level: `.villalobos/agents/`
//!
//! Each agent type has its own configuration file:
//! - `orchestrator.toml`
//! - `planner.toml`
//! - `implementer.toml`

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::config::resolver::resolve_model;
use crate::models::{AvailableModel, ModelConfig, ModelId};

/// Configuration for a single agent loaded from TOML
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AgentFileConfig {
    /// Model to use for this agent (e.g., "opus", "sonnet4.5")
    pub model: Option<String>,
}

/// Loaded configuration for all agents
#[derive(Debug, Clone, Default)]
pub struct LoadedAgentConfigs {
    pub orchestrator: AgentFileConfig,
    pub planner: AgentFileConfig,
    pub implementer: AgentFileConfig,
}

/// Returns the user-level config directory path (~/.villalobos/agents/)
fn user_config_dir() -> PathBuf {
    let home = shellexpand::tilde("~/.villalobos/agents").to_string();
    PathBuf::from(home)
}

/// Returns the project-level config directory path (.villalobos/agents/)
fn project_config_dir() -> PathBuf {
    PathBuf::from(".villalobos/agents")
}

/// Loads a single agent config file from the given path
fn load_agent_config(path: &Path) -> Result<AgentFileConfig> {
    if !path.exists() {
        return Ok(AgentFileConfig::default());
    }

    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))
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
pub fn build_model_config(
    loaded: &LoadedAgentConfigs,
    available_models: &[AvailableModel],
) -> Result<ModelConfig> {
    let mut config = ModelConfig::new(available_models.to_vec());

    // Resolve orchestrator model
    if let Some(ref model_str) = loaded.orchestrator.model {
        let resolved = resolve_model(model_str, available_models)
            .with_context(|| format!("Failed to resolve orchestrator model '{model_str}'"))?;
        config.orchestrator_model = resolved
            .parse::<ModelId>()
            .with_context(|| format!("Invalid orchestrator model ID: {resolved}"))?;
    }

    // Resolve planner model
    if let Some(ref model_str) = loaded.planner.model {
        let resolved = resolve_model(model_str, available_models)
            .with_context(|| format!("Failed to resolve planner model '{model_str}'"))?;
        config.planner_model = resolved
            .parse::<ModelId>()
            .with_context(|| format!("Invalid planner model ID: {resolved}"))?;
    }

    // Resolve implementer model
    if let Some(ref model_str) = loaded.implementer.model {
        let resolved = resolve_model(model_str, available_models)
            .with_context(|| format!("Failed to resolve implementer model '{model_str}'"))?;
        config.implementer_model = resolved
            .parse::<ModelId>()
            .with_context(|| format!("Invalid implementer model ID: {resolved}"))?;
    }

    Ok(config)
}
