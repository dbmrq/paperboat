//! Configuration error types.
//!
//! Errors related to loading, parsing, and validating configuration files.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Known model aliases that can be used in configuration files.
/// These are case-insensitive and will be resolved to actual model IDs.
pub const KNOWN_MODEL_ALIASES: &[&str] = &[
    "auto",   // Let the system choose
    "opus",   // Highest capability
    "sonnet", // Balanced
    "haiku",  // Fastest/cheapest
    // Version-specific aliases
    "opus4",
    "opus4.5",
    "sonnet4",
    "sonnet4.5",
    "haiku4",
    "haiku4.5",
];

/// Suggests a similar model alias for typos.
///
/// Uses simple edit distance heuristics to find the closest match.
pub fn suggest_model_alias(typo: &str) -> Option<String> {
    let typo_lower = typo.to_lowercase();

    // Check for common typos and near-matches
    let suggestions: &[(&str, &str)] = &[
        ("opis", "opus"),
        ("opuse", "opus"),
        ("opus5", "opus4.5"),
        ("sonet", "sonnet"),
        ("sonnett", "sonnet"),
        ("sonett", "sonnet"),
        ("sonnet5", "sonnet4.5"),
        ("hakai", "haiku"),
        ("hiku", "haiku"),
        ("haiku5", "haiku4.5"),
        ("automatic", "auto"),
    ];

    for (mistake, correction) in suggestions {
        if typo_lower == *mistake {
            return Some((*correction).to_string());
        }
    }

    // Check if it's a prefix match
    for alias in KNOWN_MODEL_ALIASES {
        if alias.starts_with(&typo_lower) && alias.len() > typo_lower.len() {
            return Some((*alias).to_string());
        }
    }

    // Check for partial matches with known families
    let families = ["opus", "sonnet", "haiku"];
    for family in families {
        if typo_lower.contains(family) || family.contains(&typo_lower) {
            return Some(family.to_string());
        }
    }

    None
}

/// Errors that can occur during configuration operations.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ConfigError {
    /// Configuration file was not found.
    #[error("Configuration file not found: {path}")]
    FileNotFound {
        /// Path to the missing file.
        path: PathBuf,
    },

    /// Failed to read configuration file.
    #[error("Failed to read configuration file '{path}': {message}")]
    ReadError {
        /// Path to the file that couldn't be read.
        path: PathBuf,
        /// Description of the error.
        message: String,
    },

    /// Failed to parse configuration file.
    #[error("Failed to parse configuration file '{path}': {message}")]
    ParseError {
        /// Path to the file that couldn't be parsed.
        path: PathBuf,
        /// Description of the parse error.
        message: String,
        /// Line number where the error occurred, if known.
        line: Option<usize>,
    },

    /// Invalid model specified in configuration.
    #[error("Invalid model '{model}': {reason}. Available models: {}", .available.join(", "))]
    InvalidModel {
        /// The invalid model string.
        model: String,
        /// Why the model is invalid.
        reason: String,
        /// List of available models.
        available: Vec<String>,
    },

    /// Model is not available (not authenticated or no access).
    #[error("Model '{model}' is not available: {reason}")]
    ModelNotAvailable {
        /// The unavailable model.
        model: String,
        /// Why the model is not available.
        reason: String,
    },

    /// Conflict during configuration merge.
    #[error("Configuration merge conflict for '{key}': {message}")]
    MergeConflict {
        /// The configuration key that has a conflict.
        key: String,
        /// Description of the conflict.
        message: String,
    },

    /// Required field is missing from configuration.
    #[error("Missing required field '{field}' in configuration")]
    MissingField {
        /// The missing field name.
        field: String,
        /// Path to the config file, if applicable.
        path: Option<PathBuf>,
    },

    /// Configuration validation failed.
    #[error("Configuration validation failed: {message}")]
    ValidationFailed {
        /// What validation check failed.
        message: String,
    },

    /// Failed to write configuration file.
    #[error("Failed to write configuration file '{path}': {message}")]
    WriteError {
        /// Path to the file that couldn't be written.
        path: PathBuf,
        /// Description of the error.
        message: String,
    },
}

impl ConfigError {
    /// Create an invalid model error with an optional suggestion.
    ///
    /// The suggestion can come from `suggest_model_alias()`.
    pub fn invalid_model(model: &str, suggestion: Option<&str>) -> Self {
        let reason = match suggestion {
            Some(sug) => format!("not recognized. Did you mean '{sug}'?"),
            None => "not recognized".to_string(),
        };
        Self::InvalidModel {
            model: model.to_string(),
            reason,
            available: KNOWN_MODEL_ALIASES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    /// Create an invalid model error with file path context.
    ///
    /// Provides a more helpful error message that includes the file location.
    pub fn invalid_model_in_file(model: &str, suggestion: Option<&str>, path: &Path) -> Self {
        let reason = match suggestion {
            Some(sug) => format!(
                "not recognized in '{}'. Did you mean '{sug}'?",
                path.display()
            ),
            None => format!("not recognized in '{}'", path.display()),
        };
        Self::InvalidModel {
            model: model.to_string(),
            reason,
            available: KNOWN_MODEL_ALIASES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_not_found_display() {
        let err = ConfigError::FileNotFound {
            path: PathBuf::from("~/.paperboat/agents/orchestrator.toml"),
        };
        let display = format!("{err}");
        assert!(display.contains("orchestrator.toml"));
    }

    #[test]
    fn test_parse_error_display() {
        let err = ConfigError::ParseError {
            path: PathBuf::from("config.toml"),
            message: "unexpected character".to_string(),
            line: Some(5),
        };
        let display = format!("{err}");
        assert!(display.contains("config.toml"));
        assert!(display.contains("unexpected character"));
    }

    #[test]
    fn test_invalid_model_display() {
        let err = ConfigError::InvalidModel {
            model: "gpt-5".to_string(),
            reason: "model not recognized".to_string(),
            available: vec![
                "opus".to_string(),
                "sonnet".to_string(),
                "haiku".to_string(),
            ],
        };
        let display = format!("{err}");
        assert!(display.contains("gpt-5"));
        assert!(display.contains("opus"));
        assert!(display.contains("sonnet"));
    }

    #[test]
    fn test_merge_conflict_display() {
        let err = ConfigError::MergeConflict {
            key: "orchestrator.model".to_string(),
            message: "conflicting values from user and project configs".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("orchestrator.model"));
    }

    #[test]
    fn test_validation_failed_display() {
        let err = ConfigError::ValidationFailed {
            message: "planner model must support extended thinking".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("planner model"));
    }
}
