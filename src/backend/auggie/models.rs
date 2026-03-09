//! Auggie model discovery.
//!
//! This module provides model tier discovery functionality for the Auggie backend
//! by running the `auggie model list` command.

use anyhow::{anyhow, Result};
use std::collections::HashSet;
use tokio::process::Command;

use crate::models::{parse_auggie_model_list, ModelTier};

/// Discovers available model tiers by running `auggie model list`.
///
/// This function executes the `auggie model list` command and parses
/// the output to extract available model tiers (Opus, Sonnet, Haiku, etc.).
///
/// # Errors
///
/// Returns an error if:
/// - The `auggie` command is not found (not installed or not in PATH)
/// - The command execution fails
/// - The command returns a non-zero exit status
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::auggie::discover_auggie_tiers;
///
/// let tiers = discover_auggie_tiers().await?;
/// if tiers.contains(&ModelTier::Sonnet) {
///     println!("Sonnet is available!");
/// }
/// ```
pub async fn discover_auggie_tiers() -> Result<HashSet<ModelTier>> {
    let output = Command::new("auggie")
        .args(["model", "list"])
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "The 'auggie' command was not found. \
                    Please ensure the Augment CLI is installed and in your PATH.\n\n\
                    Installation instructions: https://docs.augmentcode.com/cli"
                )
            } else {
                anyhow!("Failed to run 'auggie model list': {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_msg = stderr.trim();
        return Err(anyhow!(
            "auggie model list failed with status {}{}",
            output.status,
            if stderr_msg.is_empty() {
                String::new()
            } else {
                format!(": {stderr_msg}")
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_auggie_model_list(&stdout)
}

#[cfg(test)]
mod tests {
    // Note: Integration tests for discover_auggie_tiers() are not included here
    // as they require the auggie CLI to be installed and authenticated.
    // Those tests should be in integration test files.
    //
    // The parsing logic is tested in src/models.rs test suite.
}
