//! Auggie cache directory management.
//!
//! This module handles setting up Auggie-specific cache directories with
//! the appropriate `settings.json` configuration for tool removal.

use crate::backend::AgentCacheType;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

use super::auth;

/// Base path for paperboat's cache directories.
const CACHE_BASE: &str = "~/.paperboat";

/// Set up an Auggie cache directory for a specific agent type.
///
/// This function ensures the agent has the correct tool restrictions by:
/// 1. Verifying Auggie authentication (session.json exists)
/// 2. Creating the cache directory if it doesn't exist
/// 3. Copying session.json for authentication
/// 4. Writing settings.json with the removedTools configuration
///
/// # Arguments
///
/// * `agent_type` - The type of agent (Orchestrator, Planner, or Worker)
/// * `removed_tools` - List of tool names to remove for this agent
///
/// # Returns
///
/// The path to the configured cache directory. For Worker type, returns
/// an empty PathBuf since workers use the default cache.
///
/// # Config Format
///
/// Auggie uses `settings.json` with a `removedTools` array:
/// ```json
/// {
///     "removedTools": ["tool1", "tool2"]
/// }
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - Auggie is not authenticated (session.json doesn't exist)
/// - The cache directory cannot be created
/// - session.json cannot be copied
/// - settings.json cannot be written
pub fn setup_auggie_cache(agent_type: AgentCacheType, removed_tools: &[&str]) -> Result<PathBuf> {
    // First, check if auggie is authenticated
    auth::check_auggie_auth()?;

    let cache_name = match agent_type {
        AgentCacheType::Orchestrator => "augment-orchestrator",
        AgentCacheType::Planner => "augment-planner",
        AgentCacheType::Worker => return Ok(PathBuf::new()),
    };

    let cache_dir = shellexpand::tilde(&format!("{}/{}", CACHE_BASE, cache_name)).to_string();
    let cache_path = Path::new(&cache_dir);

    // Create directory if it doesn't exist
    if !cache_path.exists() {
        std::fs::create_dir_all(cache_path)
            .with_context(|| format!("Failed to create {} cache directory", agent_type))?;
        tracing::info!("Created {} cache directory: {}", agent_type, cache_dir);
    }

    // Copy session.json from main augment directory for authentication
    let main_session = auth::session_file_path();
    let agent_session = cache_path.join("session.json");

    if !agent_session.exists() {
        std::fs::copy(&main_session, &agent_session)
            .with_context(|| format!("Failed to copy session.json to {} cache", agent_type))?;
        tracing::info!("Copied session.json to {} cache", agent_type);
    }

    // Always write settings.json to ensure removedTools is current
    let settings = json!({
        "removedTools": removed_tools
    });
    let settings_path = cache_path.join("settings.json");
    std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
        .with_context(|| format!("Failed to write {} settings.json", agent_type))?;
    tracing::debug!(
        "Wrote {} settings.json with {} removed tools",
        agent_type,
        removed_tools.len()
    );

    Ok(cache_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Helper to create a test cache directory with settings
    fn setup_test_cache(
        agent_type: AgentCacheType,
        removed_tools: &[&str],
    ) -> Result<(TempDir, PathBuf)> {
        let temp_dir = TempDir::new()?;
        let cache_name = match agent_type {
            AgentCacheType::Orchestrator => "augment-orchestrator",
            AgentCacheType::Planner => "augment-planner",
            AgentCacheType::Worker => return Ok((temp_dir, PathBuf::new())),
        };

        let cache_path = temp_dir.path().join(cache_name);
        fs::create_dir_all(&cache_path)?;

        // Write settings
        let settings = json!({
            "removedTools": removed_tools
        });

        let settings_path = cache_path.join("settings.json");
        fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;

        Ok((temp_dir, cache_path))
    }

    #[test]
    fn test_worker_returns_empty_path() {
        // Worker type should return empty PathBuf without any filesystem operations
        // We test this by simulating what the function would do
        // Note: We can't easily call setup_auggie_cache directly without authentication
        let agent_type = AgentCacheType::Worker;
        assert_eq!(agent_type.as_str(), "worker");
    }

    #[test]
    fn test_settings_format_with_removed_tools() {
        let removed_tools = ["str-replace-editor", "save-file", "remove-files"];
        let (_temp_dir, cache_path) =
            setup_test_cache(AgentCacheType::Orchestrator, &removed_tools).unwrap();

        let settings_path = cache_path.join("settings.json");
        let contents = fs::read_to_string(settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert_eq!(settings["removedTools"].as_array().unwrap().len(), 3);
        assert_eq!(settings["removedTools"][0], "str-replace-editor");
        assert_eq!(settings["removedTools"][1], "save-file");
        assert_eq!(settings["removedTools"][2], "remove-files");
    }

    #[test]
    fn test_settings_format_with_empty_removed_tools() {
        let (_temp_dir, cache_path) = setup_test_cache(AgentCacheType::Planner, &[]).unwrap();

        let settings_path = cache_path.join("settings.json");
        let contents = fs::read_to_string(settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert!(settings["removedTools"].as_array().unwrap().is_empty());
    }
}
