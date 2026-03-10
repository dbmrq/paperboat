//! Configuration file writing for Paperboat
//!
//! This module handles saving agent configuration to TOML files.
//! Configuration is written to the project-level directory:
//!
//! - Project-level: `.paperboat/agents/`
//!
//! Each agent type has its own configuration file:
//! - `orchestrator.toml`
//! - `planner.toml`
//! - `implementer.toml`

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::models::ModelTier;

/// Returns the project-level config directory path (.paperboat/agents/)
fn project_config_dir() -> PathBuf {
    PathBuf::from(".paperboat/agents")
}

/// Ensures the project config directory exists, creating it if necessary.
///
/// # Returns
///
/// `Ok(PathBuf)` with the path to the directory, or `Err` if creation fails.
#[allow(dead_code)] // Public API for external consumers
pub fn ensure_config_dir() -> Result<PathBuf> {
    ensure_config_dir_at(&project_config_dir())
}

/// Ensures the given config directory exists, creating it if necessary.
///
/// This is a testable version that accepts a custom directory path.
fn ensure_config_dir_at(dir: &Path) -> Result<PathBuf> {
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create config directory: {}", dir.display()))?;
    }
    Ok(dir.to_path_buf())
}

/// Returns the tier name for saving to config files.
///
/// With the tier-based system, this is just the tier's string representation.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
const fn tier_name(tier: ModelTier) -> &'static str {
    tier.as_str()
}

/// Saves an agent configuration to its TOML file.
///
/// This function:
/// 1. Creates the config directory if it doesn't exist
/// 2. Writes the model configuration to a TOML file
/// 3. Uses atomic writes (write to temp file, then rename) to avoid corruption
/// 4. Preserves existing comments if the file already exists
///
/// # Arguments
///
/// * `agent_type` - The type of agent ("orchestrator", "planner", or "implementer")
/// * `model_id` - The model ID to save
///
/// # Returns
///
/// `Ok(())` on success, or `Err` if the operation fails.
///
/// # File Format
///
/// The file is written in TOML format:
/// ```toml
/// # Orchestrator agent configuration
/// model = "opus"
/// ```
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn save_agent_config(agent_type: &str, tier: ModelTier) -> Result<()> {
    save_agent_config_to_dir(&project_config_dir(), agent_type, tier)
}

/// Saves an agent configuration to a specified directory (for testing).
///
/// This is the testable version that accepts a custom directory path.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
fn save_agent_config_to_dir(config_dir: &Path, agent_type: &str, tier: ModelTier) -> Result<()> {
    let config_dir = ensure_config_dir_at(config_dir)?;
    let file_path = config_dir.join(format!("{agent_type}.toml"));
    let temp_path = config_dir.join(format!("{agent_type}.toml.tmp"));

    // Get the tier name for user-friendly output
    let family = tier_name(tier);

    // Generate the new content
    let content = generate_config_content_for_dir(&config_dir, agent_type, family)?;

    // Write to temp file first (atomic write)
    let mut temp_file = fs::File::create(&temp_path)
        .with_context(|| format!("Failed to create temp file: {}", temp_path.display()))?;

    temp_file
        .write_all(content.as_bytes())
        .with_context(|| format!("Failed to write to temp file: {}", temp_path.display()))?;

    temp_file
        .sync_all()
        .with_context(|| format!("Failed to sync temp file: {}", temp_path.display()))?;

    // Rename temp file to target (atomic on most filesystems)
    fs::rename(&temp_path, &file_path).with_context(|| {
        format!(
            "Failed to rename temp file {} to {}",
            temp_path.display(),
            file_path.display()
        )
    })?;

    Ok(())
}

/// Generates the TOML config file content.
///
/// If the file exists and has additional content beyond the model line,
/// preserves that content while updating only the model line.
#[allow(dead_code)] // Used by save_agent_config in production path
fn generate_config_content(agent_type: &str, model_family: &str) -> Result<String> {
    generate_config_content_for_dir(&project_config_dir(), agent_type, model_family)
}

/// Generates the TOML config file content for a specified directory.
///
/// This is the testable version that accepts a custom directory path.
fn generate_config_content_for_dir(
    config_dir: &Path,
    agent_type: &str,
    model_family: &str,
) -> Result<String> {
    let file_path = config_dir.join(format!("{agent_type}.toml"));

    if file_path.exists() {
        // Try to preserve existing content while updating the model line
        let existing = fs::read_to_string(&file_path)
            .with_context(|| format!("Failed to read existing config: {}", file_path.display()))?;
        return Ok(update_model_in_content(&existing, model_family, agent_type));
    }

    // Create new file with comment and model line
    let agent_title = capitalize_first(agent_type);
    Ok(format!(
        "# {agent_title} agent configuration\nmodel = \"{model_family}\"\n"
    ))
}

/// Updates the model line in existing content, preserving comments and other lines.
fn update_model_in_content(existing: &str, model_family: &str, agent_type: &str) -> String {
    let mut lines: Vec<String> = existing.lines().map(ToString::to_string).collect();
    let mut found_model_line = false;

    for line in &mut lines {
        let trimmed = line.trim();
        // Match lines like: model = "xxx" or model = 'xxx' or model = xxx
        if trimmed.starts_with("model") {
            if let Some(_eq_pos) = trimmed.find('=') {
                *line = format!("model = \"{model_family}\"");
                found_model_line = true;
                break;
            }
        }
    }

    if !found_model_line {
        // If no model line found, add it at the end
        // But first check if there's a header comment, if not add one
        let has_header = lines.iter().any(|l| l.trim().starts_with('#'));
        if !has_header {
            let agent_title = capitalize_first(agent_type);
            lines.insert(0, format!("# {agent_title} agent configuration"));
        }
        lines.push(format!("model = \"{model_family}\""));
    }

    // Ensure file ends with newline
    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Capitalizes the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // tier_name Tests
    // ========================================================================

    #[test]
    fn test_tier_name_opus() {
        let name = tier_name(ModelTier::Opus);
        assert_eq!(name, "opus");
    }

    #[test]
    fn test_tier_name_sonnet() {
        let name = tier_name(ModelTier::Sonnet);
        assert_eq!(name, "sonnet");
    }

    #[test]
    fn test_tier_name_codex() {
        let name = tier_name(ModelTier::Codex);
        assert_eq!(name, "codex");
    }

    #[test]
    fn test_tier_name_haiku() {
        let name = tier_name(ModelTier::Haiku);
        assert_eq!(name, "haiku");
    }

    // ========================================================================
    // capitalize_first Tests
    // ========================================================================

    #[test]
    fn test_capitalize_first_lowercase() {
        assert_eq!(capitalize_first("orchestrator"), "Orchestrator");
    }

    #[test]
    fn test_capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_capitalize_first_already_upper() {
        assert_eq!(capitalize_first("Planner"), "Planner");
    }

    // ========================================================================
    // update_model_in_content Tests
    // ========================================================================

    #[test]
    fn test_update_model_replaces_existing() {
        let existing = "# Orchestrator agent configuration\nmodel = \"sonnet\"\n";
        let result = update_model_in_content(existing, "opus", "orchestrator");
        assert!(result.contains("model = \"opus\""));
        assert!(!result.contains("sonnet"));
    }

    #[test]
    fn test_update_model_preserves_comments() {
        let existing = "# My custom comment\n# Another comment\nmodel = \"old\"\n";
        let result = update_model_in_content(existing, "new", "test");
        assert!(result.contains("# My custom comment"));
        assert!(result.contains("# Another comment"));
        assert!(result.contains("model = \"new\""));
    }

    #[test]
    fn test_update_model_adds_if_missing() {
        let existing = "# Just a comment\n";
        let result = update_model_in_content(existing, "opus", "orchestrator");
        assert!(result.contains("model = \"opus\""));
        assert!(result.contains("# Just a comment"));
    }

    #[test]
    fn test_update_model_adds_header_if_no_comment() {
        let existing = "";
        let result = update_model_in_content(existing, "opus", "orchestrator");
        assert!(result.contains("# Orchestrator agent configuration"));
        assert!(result.contains("model = \"opus\""));
    }

    #[test]
    fn test_update_model_ends_with_newline() {
        let existing = "model = \"old\"";
        let result = update_model_in_content(existing, "new", "test");
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_update_model_handles_different_quote_styles() {
        // Single quotes
        let existing = "model = 'old'\n";
        let result = update_model_in_content(existing, "new", "test");
        assert!(result.contains("model = \"new\""));
    }

    #[test]
    fn test_update_model_preserves_additional_config() {
        let existing = "# Config\nmodel = \"old\"\nsome_other_setting = true\n";
        let result = update_model_in_content(existing, "new", "test");
        assert!(result.contains("model = \"new\""));
        assert!(result.contains("some_other_setting = true"));
    }

    // ========================================================================
    // File I/O Tests (using temp directories)
    // ========================================================================

    #[test]
    fn test_save_agent_config_creates_file_if_not_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");

        // File should not exist yet
        let file_path = config_dir.join("orchestrator.toml");
        assert!(!file_path.exists());

        // Save config
        save_agent_config_to_dir(&config_dir, "orchestrator", ModelTier::Opus).unwrap();

        // File should now exist
        assert!(file_path.exists());

        // Check content
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("model = \"opus\""));
        assert!(content.contains("# Orchestrator agent configuration"));
    }

    #[test]
    fn test_save_agent_config_updates_existing_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");
        std::fs::create_dir_all(&config_dir).unwrap();

        let file_path = config_dir.join("planner.toml");

        // Create existing file with different model
        std::fs::write(&file_path, "# Planner config\nmodel = \"haiku\"\n").unwrap();

        // Save new config
        save_agent_config_to_dir(&config_dir, "planner", ModelTier::Sonnet).unwrap();

        // Check updated content
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("model = \"sonnet\""));
        assert!(!content.contains("haiku"));
        assert!(content.contains("# Planner config")); // Preserved comment
    }

    #[test]
    fn test_save_agent_config_creates_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("nested").join("path").join("agents");

        // Directory should not exist yet
        assert!(!config_dir.exists());

        // Save config - should create directory
        save_agent_config_to_dir(&config_dir, "implementer", ModelTier::Haiku).unwrap();

        // Directory and file should exist
        assert!(config_dir.exists());
        let file_path = config_dir.join("implementer.toml");
        assert!(file_path.exists());
    }

    #[test]
    fn test_save_agent_config_correct_format() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");

        save_agent_config_to_dir(&config_dir, "orchestrator", ModelTier::Opus).unwrap();

        let file_path = config_dir.join("orchestrator.toml");
        let content = std::fs::read_to_string(&file_path).unwrap();

        // Verify it's valid TOML
        let parsed: std::collections::HashMap<String, String> = toml::from_str(&content).unwrap();
        assert_eq!(parsed.get("model"), Some(&"opus".to_string()));

        // Verify format details
        assert!(content.starts_with('#'), "Should start with comment");
        assert!(content.ends_with('\n'), "Should end with newline");
        assert!(
            content.contains("model = \"opus\""),
            "Should have model line"
        );
    }

    #[test]
    fn test_save_agent_config_preserves_extra_settings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");
        std::fs::create_dir_all(&config_dir).unwrap();

        let file_path = config_dir.join("orchestrator.toml");

        // Create existing file with extra settings
        let original = "# Orchestrator\nmodel = \"haiku\"\nmax_tokens = 4096\n";
        std::fs::write(&file_path, original).unwrap();

        // Update model
        save_agent_config_to_dir(&config_dir, "orchestrator", ModelTier::Opus).unwrap();

        // Check that extra settings are preserved
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("model = \"opus\""));
        assert!(content.contains("max_tokens = 4096"));
    }

    #[test]
    fn test_ensure_config_dir_at_creates_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("new").join("agents");

        assert!(!config_dir.exists());

        let result = ensure_config_dir_at(&config_dir).unwrap();

        assert!(config_dir.exists());
        assert_eq!(result, config_dir);
    }

    #[test]
    fn test_ensure_config_dir_at_existing_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Should succeed even if directory exists
        let result = ensure_config_dir_at(&config_dir).unwrap();
        assert_eq!(result, config_dir);
    }

    #[test]
    fn test_generate_config_content_for_new_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");

        // File doesn't exist, should generate default content
        let content = generate_config_content_for_dir(&config_dir, "planner", "sonnet").unwrap();

        assert!(content.contains("# Planner agent configuration"));
        assert!(content.contains("model = \"sonnet\""));
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn test_generate_config_content_preserves_existing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_dir = temp_dir.path().join("agents");
        std::fs::create_dir_all(&config_dir).unwrap();

        let file_path = config_dir.join("implementer.toml");
        std::fs::write(
            &file_path,
            "# My custom header\nmodel = \"old\"\nextra = \"value\"\n",
        )
        .unwrap();

        let content = generate_config_content_for_dir(&config_dir, "implementer", "haiku").unwrap();

        assert!(content.contains("# My custom header"));
        assert!(content.contains("model = \"haiku\""));
        assert!(content.contains("extra = \"value\""));
        assert!(!content.contains("old"));
    }

    #[test]
    fn test_all_model_tiers_produce_valid_names() {
        // Test that all ModelTier variants produce valid tier names
        let test_cases = [
            (ModelTier::Auto, "auto"),
            (ModelTier::Opus, "opus"),
            (ModelTier::Sonnet, "sonnet"),
            (ModelTier::Haiku, "haiku"),
            (ModelTier::Codex, "codex"),
            (ModelTier::CodexMini, "codex-mini"),
        ];

        for (tier, expected_name) in test_cases {
            let name = tier_name(tier);
            assert_eq!(
                name, expected_name,
                "ModelTier::{tier:?} should produce name '{expected_name}'"
            );
        }
    }
}
