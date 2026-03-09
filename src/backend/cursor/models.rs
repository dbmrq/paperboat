//! Model tier discovery for Cursor backend.
//!
//! This module discovers available model tiers by running `cursor-agent --list-models`.
//! The output format is:
//! ```text
//! sonnet-4.6 - Claude 4.6 Sonnet
//! opus-4.6 - Claude 4.6 Opus  (current, default)
//! gpt-5.1-codex-mini - GPT-5.1 Codex Mini
//! ```

use anyhow::{anyhow, Result};
use regex::Regex;
use std::collections::HashSet;
use tokio::process::Command;

use crate::models::ModelTier;

/// Discover available model tiers by running `cursor-agent --list-models`.
///
/// Parses the output and maps Cursor model IDs to ModelTier values.
pub async fn discover_cursor_tiers() -> Result<HashSet<ModelTier>> {
    let output = Command::new("cursor-agent")
        .arg("--list-models")
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run 'cursor-agent --list-models': {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "cursor-agent --list-models failed with status {}: {}",
            output.status,
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_cursor_tiers(&stdout)
}

/// Parse the output of `cursor-agent --list-models` into a set of ModelTiers.
///
/// The format is:
/// ```text
/// model-id - Display Name
/// model-id - Display Name  (current, default)
/// ```
///
/// Maps cursor model IDs to tiers:
/// - `sonnet-*` → Sonnet
/// - `opus-*` → Opus
/// - `gpt-*-codex-mini` → CodexMini
/// - `gpt-*-codex` → Codex
/// - `gemini-*-pro` → Gemini
/// - `gemini-*-flash` → GeminiFlash
/// - `grok*` → Grok
/// - `composer-*` → Composer
pub fn parse_cursor_tiers(output: &str) -> Result<HashSet<ModelTier>> {
    let mut tiers = HashSet::new();

    // Pattern: "model-id - Display Name"
    let line_re = Regex::new(r"^([a-z0-9.-]+)\s+-\s+")?;

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Loading") || line.starts_with("Available") {
            continue;
        }

        if let Some(caps) = line_re.captures(line) {
            let cursor_id = caps.get(1).map_or("", |m| m.as_str());
            if let Some(tier) = extract_tier_from_cursor_id(cursor_id) {
                tiers.insert(tier);
            }
        }
    }

    Ok(tiers)
}

/// Extract a ModelTier from a Cursor model ID string.
///
/// Examples:
/// - "sonnet-4.6" → Sonnet
/// - "opus-4.5-thinking" → Opus
/// - "gpt-5.1-codex-mini" → CodexMini
/// - "gpt-5.3-codex" → Codex
/// - "gemini-3.1-pro" → Gemini
/// - "gemini-3-flash" → GeminiFlash
/// - "grok" → Grok
/// - "composer-1.5" → Composer
fn extract_tier_from_cursor_id(id: &str) -> Option<ModelTier> {
    let lower = id.to_lowercase();

    // Claude models
    if lower.starts_with("sonnet") {
        return Some(ModelTier::Sonnet);
    }
    if lower.starts_with("opus") {
        return Some(ModelTier::Opus);
    }

    // GPT models - check for specific tiers
    if lower.contains("codex-mini") {
        return Some(ModelTier::CodexMini);
    }
    if lower.contains("codex") {
        return Some(ModelTier::Codex);
    }

    // Gemini models
    if lower.contains("gemini") && lower.contains("flash") {
        return Some(ModelTier::GeminiFlash);
    }
    if lower.starts_with("gemini") {
        return Some(ModelTier::Gemini);
    }

    // Other models
    if lower.starts_with("grok") {
        return Some(ModelTier::Grok);
    }
    if lower.starts_with("composer") {
        return Some(ModelTier::Composer);
    }

    if lower == "auto" {
        return Some(ModelTier::Auto);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cursor_tiers_basic() {
        let output = r#"
sonnet-4.6 - Claude 4.6 Sonnet
opus-4.6 - Claude 4.6 Opus
gpt-5.1-codex-mini - GPT-5.1 Codex Mini
"#;
        let tiers = parse_cursor_tiers(output).unwrap();

        assert!(tiers.contains(&ModelTier::Sonnet));
        assert!(tiers.contains(&ModelTier::Opus));
        assert!(tiers.contains(&ModelTier::CodexMini));
    }

    #[test]
    fn test_parse_cursor_tiers_all_types() {
        let output = r#"
sonnet-4.6 - Claude 4.6 Sonnet
opus-4.6 - Claude 4.6 Opus
gpt-5.3-codex - GPT-5.3 Codex
gpt-5.1-codex-mini - GPT-5.1 Codex Mini
gemini-3.1-pro - Gemini 3.1 Pro
gemini-3-flash - Gemini 3 Flash
grok - Grok
composer-1.5 - Composer 1.5
"#;
        let tiers = parse_cursor_tiers(output).unwrap();

        assert!(tiers.contains(&ModelTier::Sonnet));
        assert!(tiers.contains(&ModelTier::Opus));
        assert!(tiers.contains(&ModelTier::Codex));
        assert!(tiers.contains(&ModelTier::CodexMini));
        assert!(tiers.contains(&ModelTier::Gemini));
        assert!(tiers.contains(&ModelTier::GeminiFlash));
        assert!(tiers.contains(&ModelTier::Grok));
        assert!(tiers.contains(&ModelTier::Composer));
        // Haiku should NOT be present
        assert!(!tiers.contains(&ModelTier::Haiku));
    }

    #[test]
    fn test_extract_tier_from_cursor_id() {
        assert_eq!(
            extract_tier_from_cursor_id("sonnet-4.6"),
            Some(ModelTier::Sonnet)
        );
        assert_eq!(
            extract_tier_from_cursor_id("opus-4.5-thinking"),
            Some(ModelTier::Opus)
        );
        assert_eq!(
            extract_tier_from_cursor_id("gpt-5.1-codex-mini"),
            Some(ModelTier::CodexMini)
        );
        assert_eq!(
            extract_tier_from_cursor_id("gpt-5.3-codex"),
            Some(ModelTier::Codex)
        );
        assert_eq!(
            extract_tier_from_cursor_id("gemini-3.1-pro"),
            Some(ModelTier::Gemini)
        );
        assert_eq!(
            extract_tier_from_cursor_id("gemini-3-flash"),
            Some(ModelTier::GeminiFlash)
        );
        assert_eq!(extract_tier_from_cursor_id("grok"), Some(ModelTier::Grok));
        assert_eq!(
            extract_tier_from_cursor_id("composer-1.5"),
            Some(ModelTier::Composer)
        );
        assert_eq!(extract_tier_from_cursor_id("unknown-model"), None);
    }
}
