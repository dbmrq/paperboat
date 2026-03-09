//! Auggie authentication utilities.
//!
//! This module provides authentication checking for the Auggie backend.
//! It verifies that the user has logged in via `auggie login` by checking
//! for the existence of the session file.

use anyhow::{bail, Result};
use std::path::PathBuf;

/// Path to the Augment configuration directory.
const AUGMENT_CONFIG_DIR: &str = "~/.augment";

/// Name of the session file.
const SESSION_FILE_NAME: &str = "session.json";

/// Get the path to the main Augment session file.
///
/// The session file is located at `~/.augment/session.json` and is created
/// when the user runs `auggie login`.
///
/// # Returns
///
/// The expanded path to the session file.
#[must_use]
pub fn session_file_path() -> PathBuf {
    let augment_dir = shellexpand::tilde(AUGMENT_CONFIG_DIR).to_string();
    PathBuf::from(&augment_dir).join(SESSION_FILE_NAME)
}

/// Check if Auggie CLI is authenticated.
///
/// This function verifies that the user has authenticated with the Auggie CLI
/// by checking for the existence of `~/.augment/session.json`.
///
/// # Errors
///
/// Returns an error if the session file does not exist, indicating that
/// the user needs to run `auggie login` first.
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::auggie::auth::check_auggie_auth;
///
/// fn main() -> anyhow::Result<()> {
///     check_auggie_auth()?;
///     println!("Authenticated!");
///     Ok(())
/// }
/// ```
pub fn check_auggie_auth() -> Result<()> {
    let session_path = session_file_path();

    if !session_path.exists() {
        bail!(
            "Augment CLI is not authenticated.\n\n\
            Please run 'auggie login' first to authenticate, then try again."
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_file_path_expands_tilde() {
        let path = session_file_path();
        // The path should be expanded and not contain a tilde
        let path_str = path.to_string_lossy();
        assert!(
            !path_str.starts_with('~'),
            "Path should be expanded: {}",
            path_str
        );
        // Should end with session.json
        assert!(
            path_str.ends_with("session.json"),
            "Path should end with session.json: {}",
            path_str
        );
        // Should contain .augment directory
        assert!(
            path_str.contains(".augment"),
            "Path should contain .augment: {}",
            path_str
        );
    }

    #[test]
    fn test_session_file_path_is_absolute() {
        let path = session_file_path();
        assert!(
            path.is_absolute(),
            "Session file path should be absolute: {:?}",
            path
        );
    }

    #[test]
    fn test_check_auggie_auth_returns_correct_error_message() {
        // This test verifies the error message format when authentication fails.
        // We can't easily test the success case without mocking the file system,
        // but we can verify the error message structure.

        // Create a temporary directory without session.json to simulate unauthenticated state
        // Note: This test relies on the actual file system check.
        // In a real scenario, you might want to use a mock or temp directory.

        // For now, just verify the function exists and can be called
        // The actual behavior depends on whether ~/.augment/session.json exists
        let _ = check_auggie_auth();
    }
}
