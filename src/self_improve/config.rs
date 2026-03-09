//! Configuration for self-improvement feature.
//!
//! This module handles loading configuration for the self-improvement feature.
//! Configuration is read from (in priority order):
//!
//! 1. Environment variable `PAPERBOAT_SELF_IMPROVE` (`1`, `true`, `0`, `false`)
//! 2. Project-level config: `.paperboat/self-improve.toml`
//! 3. User-level config: `~/.paperboat/self-improve.toml`
//! 4. Default: `true` (opt-out feature - enabled by default)

use serde::Deserialize;
use std::path::PathBuf;

/// Configuration for self-improvement feature loaded from TOML
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SelfImproveConfig {
    /// Whether self-improvement is enabled
    #[serde(default)]
    pub enabled: bool,
}

/// Returns the user-level self-improve config path (~/.paperboat/self-improve.toml)
fn user_config_path() -> PathBuf {
    let home = shellexpand::tilde("~/.paperboat/self-improve.toml").to_string();
    PathBuf::from(home)
}

/// Returns the project-level self-improve config path (.paperboat/self-improve.toml)
fn project_config_path() -> PathBuf {
    PathBuf::from(".paperboat/self-improve.toml")
}

/// Loads self-improvement configuration from TOML file.
///
/// Returns None if the file doesn't exist or can't be parsed.
fn load_config_file(path: &PathBuf) -> Option<SelfImproveConfig> {
    if !path.exists() {
        return None;
    }

    let contents = std::fs::read_to_string(path).ok()?;
    toml::from_str(&contents).ok()
}

/// Loads self-improvement configuration from config files.
///
/// Checks project-level config first, then user-level config.
/// Returns None if no config file exists.
fn load_self_improve_config() -> Option<SelfImproveConfig> {
    // Check project-level config first (higher priority)
    if let Some(config) = load_config_file(&project_config_path()) {
        return Some(config);
    }

    // Fall back to user-level config
    load_config_file(&user_config_path())
}

/// Checks if self-improvement is enabled.
///
/// Priority (highest to lowest):
/// 1. `PAPERBOAT_SELF_IMPROVE` environment variable (`1`, `true` to enable; `0`, `false` to disable)
/// 2. `.paperboat/self-improve.toml` (project-level)
/// 3. `~/.paperboat/self-improve.toml` (user-level)
/// 4. Default: `true` (opt-out feature - enabled by default)
///
/// # Examples
///
/// ```ignore
/// // Check if self-improvement should run
/// if is_self_improvement_enabled() {
///     // Run self-improvement agent
/// }
/// ```
pub fn is_self_improvement_enabled() -> bool {
    // Check env var first (highest priority)
    if let Ok(val) = std::env::var("PAPERBOAT_SELF_IMPROVE") {
        let val_lower = val.to_lowercase();
        // Explicit disable values return false
        if matches!(val_lower.as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
        // Enable values return true
        return matches!(val_lower.as_str(), "1" | "true" | "yes" | "on");
    }

    // Check config files, default to true if no config exists
    load_self_improve_config().is_none_or(|c| c.enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Use a mutex to serialize tests that modify PAPERBOAT_SELF_IMPROVE env var.
    // This prevents race conditions when tests run in parallel.
    static ENV_VAR_MUTEX: Mutex<()> = Mutex::new(());

    // Test the parsing logic directly without relying on environment variables
    fn parse_env_value(val: &str) -> bool {
        let val_lower = val.to_lowercase();
        matches!(val_lower.as_str(), "1" | "true" | "yes" | "on")
    }

    #[test]
    fn test_env_parsing_true_values() {
        assert!(parse_env_value("true"));
        assert!(parse_env_value("TRUE"));
        assert!(parse_env_value("True"));
        assert!(parse_env_value("1"));
        assert!(parse_env_value("yes"));
        assert!(parse_env_value("YES"));
        assert!(parse_env_value("on"));
        assert!(parse_env_value("ON"));
    }

    #[test]
    fn test_env_parsing_false_values() {
        assert!(!parse_env_value("false"));
        assert!(!parse_env_value("FALSE"));
        assert!(!parse_env_value("0"));
        assert!(!parse_env_value("no"));
        assert!(!parse_env_value("off"));
        assert!(!parse_env_value(""));
        assert!(!parse_env_value("invalid"));
    }

    #[test]
    fn test_config_struct_default() {
        let config = SelfImproveConfig::default();
        assert!(!config.enabled);
    }

    #[test]
    fn test_config_parsing() {
        let toml_enabled = "enabled = true";
        let config: SelfImproveConfig = toml::from_str(toml_enabled).unwrap();
        assert!(config.enabled);

        let toml_disabled = "enabled = false";
        let config: SelfImproveConfig = toml::from_str(toml_disabled).unwrap();
        assert!(!config.enabled);

        // Empty config defaults to disabled
        let toml_empty = "";
        let config: SelfImproveConfig = toml::from_str(toml_empty).unwrap();
        assert!(!config.enabled);
    }

    #[test]
    fn test_user_config_path() {
        let path = user_config_path();
        assert!(path.to_string_lossy().contains(".paperboat"));
        assert!(path.to_string_lossy().contains("self-improve.toml"));
    }

    #[test]
    fn test_project_config_path() {
        let path = project_config_path();
        assert_eq!(path.to_string_lossy(), ".paperboat/self-improve.toml");
    }

    #[test]
    fn test_load_config_file_nonexistent() {
        let path = PathBuf::from("/nonexistent/path/self-improve.toml");
        let result = load_config_file(&path);
        assert!(result.is_none());
    }

    // ========================================================================
    // Environment Variable Tests (serialized with ENV_VAR_MUTEX)
    // ========================================================================

    #[test]
    fn test_is_self_improvement_enabled_env_true() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set env var to enable
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "true");

        assert!(is_self_improvement_enabled());

        // Clean up
        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");
    }

    #[test]
    fn test_is_self_improvement_enabled_env_one() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set env var to "1"
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "1");

        assert!(is_self_improvement_enabled());

        // Clean up
        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");
    }

    #[test]
    fn test_is_self_improvement_enabled_env_false() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set env var to disable
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "false");

        assert!(!is_self_improvement_enabled());

        // Clean up
        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");
    }

    #[test]
    fn test_is_self_improvement_enabled_env_zero() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Set env var to "0"
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "0");

        assert!(!is_self_improvement_enabled());

        // Clean up
        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");
    }

    #[test]
    fn test_is_self_improvement_enabled_no_env_var() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Ensure env var is not set
        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");

        // Without config files, defaults to true (opt-out feature)
        assert!(is_self_improvement_enabled());
    }

    // ========================================================================
    // Config File Loading Tests
    // ========================================================================

    #[test]
    fn test_load_config_file_valid_enabled() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("self-improve.toml");
        std::fs::write(&config_path, "enabled = true").unwrap();

        let result = load_config_file(&config_path);
        assert!(result.is_some());
        assert!(result.unwrap().enabled);
    }

    #[test]
    fn test_load_config_file_valid_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("self-improve.toml");
        std::fs::write(&config_path, "enabled = false").unwrap();

        let result = load_config_file(&config_path);
        assert!(result.is_some());
        assert!(!result.unwrap().enabled);
    }

    #[test]
    fn test_load_config_file_invalid_toml() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("self-improve.toml");
        std::fs::write(&config_path, "this is not valid toml {{{").unwrap();

        let result = load_config_file(&config_path);
        // Invalid TOML returns None (graceful handling)
        assert!(result.is_none());
    }

    #[test]
    fn test_load_config_file_empty() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("self-improve.toml");
        std::fs::write(&config_path, "").unwrap();

        let result = load_config_file(&config_path);
        assert!(result.is_some());
        // Empty config should default to disabled
        assert!(!result.unwrap().enabled);
    }

    #[test]
    fn test_load_config_file_with_extra_fields() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("self-improve.toml");
        std::fs::write(
            &config_path,
            r#"
enabled = true
# Future fields should be ignored
some_other_field = "value"
"#,
        )
        .unwrap();

        // Should parse successfully, ignoring unknown fields
        let result = load_config_file(&config_path);
        assert!(result.is_some());
        assert!(result.unwrap().enabled);
    }
}
