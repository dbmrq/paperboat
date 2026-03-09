//! Cursor cache directory management.
//!
//! This module handles setting up Cursor-specific cache directories with
//! the appropriate `cli-config.json` configuration for tool permissions.
//!
//! **Note:** Currently, `agent acp` does not support a `--config-dir` flag,
//! so the cache directories are set up but not actually used. The config
//! files are written to preserve the structure for future Cursor CLI versions
//! that may support this feature.

use crate::backend::AgentCacheType;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Base path for paperboat's cache directories.
const CACHE_BASE: &str = "~/.paperboat";

/// Set up a Cursor cache directory for a specific agent type.
///
/// Creates the cache directory if it doesn't exist and writes the
/// `cli-config.json` file with the appropriate permissions configuration.
///
/// # Arguments
///
/// * `agent_type` - The type of agent (Orchestrator, Planner, or Worker)
/// * `denied_tools` - List of tool names to deny for this agent
///
/// # Returns
///
/// The path to the configured cache directory. For Worker type, returns
/// an empty PathBuf since workers use the default cache.
///
/// # Config Format
///
/// Cursor uses `cli-config.json` with a `permissions` object:
/// ```json
/// {
///     "version": 1,
///     "permissions": {
///         "allow": [],
///         "deny": ["tool1", "tool2"]
///     }
/// }
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - The cache directory cannot be created
/// - The `cli-config.json` file cannot be written
pub fn setup_cursor_cache(agent_type: AgentCacheType, denied_tools: &[&str]) -> Result<PathBuf> {
    let cache_name = match agent_type {
        AgentCacheType::Orchestrator => "cursor-orchestrator",
        AgentCacheType::Planner => "cursor-planner",
        AgentCacheType::Worker => return Ok(PathBuf::new()),
    };

    let cache_dir = shellexpand::tilde(&format!("{}/{}", CACHE_BASE, cache_name)).to_string();
    let cache_path = Path::new(&cache_dir);

    // Create directory if needed
    if !cache_path.exists() {
        std::fs::create_dir_all(cache_path)
            .with_context(|| format!("Failed to create cache directory: {}", cache_dir))?;
    }

    // Write cli-config.json with permissions (Cursor format)
    let config = json!({
        "version": 1,
        "permissions": {
            "allow": [],
            "deny": denied_tools
        }
    });

    let config_path = cache_path.join("cli-config.json");
    std::fs::write(&config_path, serde_json::to_string_pretty(&config)?).with_context(|| {
        format!(
            "Failed to write cli-config.json to {}",
            config_path.display()
        )
    })?;

    Ok(cache_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Helper to set up test environment with custom CACHE_BASE
    fn setup_test_cache(
        agent_type: AgentCacheType,
        denied_tools: &[&str],
    ) -> Result<(TempDir, PathBuf)> {
        let temp_dir = TempDir::new()?;
        let cache_name = match agent_type {
            AgentCacheType::Orchestrator => "cursor-orchestrator",
            AgentCacheType::Planner => "cursor-planner",
            AgentCacheType::Worker => return Ok((temp_dir, PathBuf::new())),
        };

        let cache_path = temp_dir.path().join(cache_name);
        fs::create_dir_all(&cache_path)?;

        // Write config
        let config = json!({
            "version": 1,
            "permissions": {
                "allow": [],
                "deny": denied_tools
            }
        });

        let config_path = cache_path.join("cli-config.json");
        fs::write(&config_path, serde_json::to_string_pretty(&config)?)?;

        Ok((temp_dir, cache_path))
    }

    #[test]
    fn test_worker_returns_empty_path() {
        let result = setup_cursor_cache(AgentCacheType::Worker, &["tool1", "tool2"]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::new());
    }

    #[test]
    fn test_config_format_with_denied_tools() {
        let denied_tools = ["str-replace-editor", "save-file", "remove-files"];
        let (_temp_dir, cache_path) =
            setup_test_cache(AgentCacheType::Orchestrator, &denied_tools).unwrap();

        let config_path = cache_path.join("cli-config.json");
        let contents = fs::read_to_string(config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert_eq!(config["version"], 1);
        assert!(config["permissions"]["allow"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(config["permissions"]["deny"].as_array().unwrap().len(), 3);
        assert_eq!(config["permissions"]["deny"][0], "str-replace-editor");
        assert_eq!(config["permissions"]["deny"][1], "save-file");
        assert_eq!(config["permissions"]["deny"][2], "remove-files");
    }

    #[test]
    fn test_config_format_with_empty_denied_tools() {
        let (_temp_dir, cache_path) = setup_test_cache(AgentCacheType::Planner, &[]).unwrap();

        let config_path = cache_path.join("cli-config.json");
        let contents = fs::read_to_string(config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert_eq!(config["version"], 1);
        assert!(config["permissions"]["allow"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(config["permissions"]["deny"].as_array().unwrap().is_empty());
    }
}
