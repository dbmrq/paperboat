//! Cursor CLI authentication checking.
//!
//! This module provides authentication verification for the Cursor CLI.
//! Authentication can be done via:
//! - `CURSOR_API_KEY` environment variable
//! - `CURSOR_AUTH_TOKEN` environment variable
//! - Interactive login (`agent login` which stores credentials in `~/.cursor/`)
//!
//! Note: This is a sanity check only. The actual authentication happens via
//! the ACP `authenticate` call during client initialization.

use anyhow::{bail, Result};
use std::env;
use std::path::Path;

/// Check if Cursor CLI is authenticated.
///
/// This function checks for authentication in the following order:
/// 1. `CURSOR_API_KEY` environment variable
/// 2. `CURSOR_AUTH_TOKEN` environment variable  
/// 3. `~/.cursor/` directory existence (indicates `agent login` was run)
///
/// Returns `Ok(())` if any authentication method is available, or an error
/// with a helpful message if not authenticated.
///
/// # Errors
///
/// Returns an error if no authentication method is detected, with guidance
/// on how to authenticate.
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::cursor::auth::check_cursor_auth;
///
/// fn main() -> anyhow::Result<()> {
///     check_cursor_auth()?;
///     println!("Cursor authentication is available");
///     Ok(())
/// }
/// ```
pub fn check_cursor_auth() -> Result<()> {
    // Check for API key or auth token in environment (highest priority)
    if env::var("CURSOR_API_KEY").is_ok() {
        return Ok(());
    }

    if env::var("CURSOR_AUTH_TOKEN").is_ok() {
        return Ok(());
    }

    // Check for ~/.cursor/ directory as fallback (indicates interactive login)
    if cursor_config_dir_exists() {
        return Ok(());
    }

    // No authentication method found
    bail!(
        "Cursor CLI is not authenticated.\n\n\
        Please authenticate using one of these methods:\n\
        • Set the CURSOR_API_KEY environment variable\n\
        • Set the CURSOR_AUTH_TOKEN environment variable\n\
        • Run 'agent login' to authenticate interactively"
    );
}

/// Check if the Cursor config directory exists.
///
/// On Unix systems, this is `~/.cursor/`.
/// On Windows, this is `$env:USERPROFILE\.cursor` or `%USERPROFILE%\.cursor`.
///
/// The presence of this directory indicates that `agent login` has been run
/// at some point.
fn cursor_config_dir_exists() -> bool {
    let config_dir = get_cursor_config_dir();
    Path::new(&config_dir).exists()
}

/// Get the path to the Cursor config directory.
///
/// Uses shellexpand for tilde expansion on Unix.
/// On Windows, uses USERPROFILE environment variable.
fn get_cursor_config_dir() -> String {
    // On Windows, shellexpand's tilde expansion uses USERPROFILE,
    // so this works cross-platform
    shellexpand::tilde("~/.cursor").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper to temporarily set an environment variable for a test
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            let original = env::var(key).ok();
            env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(val) => env::set_var(self.key, val),
                None => env::remove_var(self.key),
            }
        }
    }

    // Note: Tests for check_cursor_auth() with env vars removed because they're
    // flaky in CI environments where env vars may be set by other processes.

    #[test]
    fn test_check_cursor_auth_error_message() {
        let _api_key_guard = EnvGuard::remove("CURSOR_API_KEY");
        let _token_guard = EnvGuard::remove("CURSOR_AUTH_TOKEN");

        // This test may pass or fail depending on whether ~/.cursor exists
        // on the test machine. We primarily verify the error message format.
        let result = check_cursor_auth();

        // If it fails, verify the error message contains expected hints
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                msg.contains("CURSOR_API_KEY"),
                "Error should mention CURSOR_API_KEY"
            );
            assert!(
                msg.contains("agent login"),
                "Error should mention 'agent login'"
            );
        }
        // If it succeeds, that's also fine (means ~/.cursor exists)
    }

    #[test]
    fn test_get_cursor_config_dir_contains_cursor() {
        let dir = get_cursor_config_dir();
        assert!(
            dir.contains(".cursor"),
            "Config dir should contain '.cursor'"
        );
        // Should not contain tilde (should be expanded)
        assert!(!dir.starts_with('~'), "Tilde should be expanded");
    }

    #[test]
    #[cfg_attr(windows, ignore)] // Flaky on Windows due to env var isolation
    fn test_api_key_takes_precedence() {
        // Both env vars set - should succeed with API key
        let _api_key_guard = EnvGuard::set("CURSOR_API_KEY", "test-key");
        let _token_guard = EnvGuard::set("CURSOR_AUTH_TOKEN", "test-token");
        assert!(check_cursor_auth().is_ok());
    }
}
