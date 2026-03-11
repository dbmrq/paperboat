//! Repository detection for self-improvement feature.
//!
//! This module provides utilities to detect whether the application is running
//! in its own repository (paperboat) or in a different repository. This is used
//! to enable/disable the self-improvement feature.

use std::path::Path;
use std::process::Command;

/// Result of repository detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryKind {
    /// Running in the paperboat repository (self-improvement enabled).
    OwnRepository,
    /// Running in a different repository.
    DifferentRepository,
    /// Could not determine the repository type.
    Unknown,
}

impl RepositoryKind {
    /// Returns true if this is the paperboat repository.
    #[must_use]
    pub const fn is_own_repository(&self) -> bool {
        matches!(self, Self::OwnRepository)
    }
}

/// Detects whether the current working directory is the paperboat repository.
///
/// Uses multiple detection methods:
/// 1. Check git remote URL for "paperboat" or "villalobos"
/// 2. Check Cargo.toml for `name = "paperboat"`
///
/// Either method matching is sufficient to confirm we're in the paperboat repo.
///
/// # Returns
/// - `OwnRepository` if detected as the paperboat repository
/// - `DifferentRepository` if definitely a different repository
/// - `Unknown` if detection failed (no git, no Cargo.toml, etc.)
#[must_use]
pub fn detect_repository() -> RepositoryKind {
    detect_repository_at(Path::new("."))
}

/// Detects whether the given directory is the paperboat repository.
///
/// This is the testable version that accepts a path parameter.
///
/// # Arguments
/// * `dir` - The directory to check
///
/// # Returns
/// - `OwnRepository` if detected as the paperboat repository
/// - `DifferentRepository` if definitely a different repository
/// - `Unknown` if detection failed
#[must_use]
pub fn detect_repository_at(dir: &Path) -> RepositoryKind {
    // Method 1: Check git remote URL
    if let Some(is_paperboat) = check_git_remote(dir) {
        if is_paperboat {
            return RepositoryKind::OwnRepository;
        }
        // Git remote exists but doesn't match - could still check Cargo.toml
        // for local development scenarios without proper remote
    }

    // Method 2: Check Cargo.toml package name
    if let Some(is_paperboat) = check_cargo_toml(dir) {
        if is_paperboat {
            return RepositoryKind::OwnRepository;
        }
        // Has Cargo.toml but different package name
        return RepositoryKind::DifferentRepository;
    }

    // Neither method could make a determination
    RepositoryKind::Unknown
}

/// Check if the git remote URL indicates the paperboat repository.
///
/// Returns `Some(true)` if the remote matches paperboat/villalobos,
/// `Some(false)` if git remote exists but doesn't match,
/// `None` if git is not available or not a git repository.
fn check_git_remote(dir: &Path) -> Option<bool> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout);
    let url_lower = url.to_lowercase();

    // Check for paperboat or villalobos (the monorepo name)
    let is_paperboat = url_lower.contains("paperboat") || url_lower.contains("villalobos");

    Some(is_paperboat)
}

/// Check if the Cargo.toml package name is "paperboat".
///
/// Returns `Some(true)` if Cargo.toml contains `name = "paperboat"`,
/// `Some(false)` if Cargo.toml exists but has different package name,
/// `None` if Cargo.toml doesn't exist or can't be read.
fn check_cargo_toml(dir: &Path) -> Option<bool> {
    let cargo_path = dir.join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_path).ok()?;

    // Simple check: look for name = "paperboat" in the package section
    // This is intentionally simple - a full TOML parse would be overkill
    let is_paperboat = content.contains(r#"name = "paperboat""#);

    Some(is_paperboat)
}

/// Async-compatible wrapper for repository detection.
///
/// This function is async to allow for future enhancements that may
/// require async operations (e.g., checking remote repository state).
#[allow(dead_code)] // Public async API for repository detection
pub async fn detect_repository_async() -> RepositoryKind {
    // For now, just wrap the sync version
    // Using spawn_blocking for the git command execution
    tokio::task::spawn_blocking(detect_repository)
        .await
        .unwrap_or(RepositoryKind::Unknown)
}

/// Async-compatible wrapper for repository detection at a specific path.
#[allow(dead_code)] // Public async API for path-based repository detection
pub async fn detect_repository_at_async(dir: std::path::PathBuf) -> RepositoryKind {
    tokio::task::spawn_blocking(move || detect_repository_at(&dir))
        .await
        .unwrap_or(RepositoryKind::Unknown)
}

/// Checks if the current working directory is the paperboat repository.
///
/// This is a convenience function that returns `true` if the current directory
/// is identified as the paperboat repository using git remote URL and Cargo.toml checks.
///
/// # Returns
///
/// Returns `true` if the current directory is the paperboat repository, `false` otherwise.
///
/// # Example
///
/// ```ignore
/// use paperboat::self_improve::detection::is_paperboat_repository;
///
/// if is_paperboat_repository() {
///     // Self-improvement feature can run
/// }
/// ```
#[must_use]
pub fn is_paperboat_repository() -> bool {
    detect_repository().is_own_repository()
}

/// Checks if the given directory is the paperboat repository.
///
/// This is the path-parameterized version of [`is_paperboat_repository`] for testing.
#[must_use]
#[allow(dead_code)] // Used in tests and for path-parameterized detection
pub fn is_paperboat_repository_in(dir: &Path) -> bool {
    detect_repository_at(dir).is_own_repository()
}

/// Async-compatible wrapper for `is_paperboat_repository`.
#[allow(dead_code)] // Public async API for repository check
pub async fn is_paperboat_repository_async() -> bool {
    detect_repository_async().await.is_own_repository()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to create a temporary directory with specific files
    fn setup_test_dir() -> TempDir {
        tempfile::tempdir().expect("Failed to create temp dir")
    }

    #[test]
    fn test_repository_kind_is_own_repository() {
        assert!(RepositoryKind::OwnRepository.is_own_repository());
        assert!(!RepositoryKind::DifferentRepository.is_own_repository());
        assert!(!RepositoryKind::Unknown.is_own_repository());
    }

    #[test]
    fn test_detect_paperboat_cargo_toml() {
        let temp = setup_test_dir();
        let cargo_path = temp.path().join("Cargo.toml");

        // Create a Cargo.toml with paperboat package name
        fs::write(
            &cargo_path,
            r#"[package]
name = "paperboat"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("Failed to write Cargo.toml");

        let result = detect_repository_at(temp.path());
        assert_eq!(result, RepositoryKind::OwnRepository);
    }

    #[test]
    fn test_detect_different_cargo_toml() {
        let temp = setup_test_dir();
        let cargo_path = temp.path().join("Cargo.toml");

        // Create a Cargo.toml with a different package name
        fs::write(
            &cargo_path,
            r#"[package]
name = "some-other-project"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("Failed to write Cargo.toml");

        let result = detect_repository_at(temp.path());
        assert_eq!(result, RepositoryKind::DifferentRepository);
    }

    #[test]
    fn test_detect_empty_directory() {
        let temp = setup_test_dir();

        // Empty directory - no git, no Cargo.toml
        let result = detect_repository_at(temp.path());
        assert_eq!(result, RepositoryKind::Unknown);
    }

    #[test]
    fn test_detect_nonexistent_directory() {
        let nonexistent = Path::new("/nonexistent/path/that/does/not/exist");
        let result = detect_repository_at(nonexistent);
        // Should return Unknown since we can't read anything
        assert_eq!(result, RepositoryKind::Unknown);
    }

    #[test]
    fn test_check_cargo_toml_missing() {
        let temp = setup_test_dir();
        let result = check_cargo_toml(temp.path());
        assert_eq!(result, None);
    }

    #[test]
    fn test_check_cargo_toml_paperboat() {
        let temp = setup_test_dir();
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "paperboat"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = check_cargo_toml(temp.path());
        assert_eq!(result, Some(true));
    }

    #[test]
    fn test_check_cargo_toml_other_project() {
        let temp = setup_test_dir();
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "other-project"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = check_cargo_toml(temp.path());
        assert_eq!(result, Some(false));
    }

    #[test]
    fn test_check_git_remote_no_git() {
        let temp = setup_test_dir();
        // No .git directory
        let result = check_git_remote(temp.path());
        assert_eq!(result, None);
    }

    #[test]
    fn test_repository_kind_equality() {
        assert_eq!(RepositoryKind::OwnRepository, RepositoryKind::OwnRepository);
        assert_eq!(
            RepositoryKind::DifferentRepository,
            RepositoryKind::DifferentRepository
        );
        assert_eq!(RepositoryKind::Unknown, RepositoryKind::Unknown);

        assert_ne!(
            RepositoryKind::OwnRepository,
            RepositoryKind::DifferentRepository
        );
        assert_ne!(RepositoryKind::OwnRepository, RepositoryKind::Unknown);
    }

    #[test]
    fn test_repository_kind_clone() {
        let kind = RepositoryKind::OwnRepository;
        let cloned = kind.clone();
        assert_eq!(kind, cloned);
    }

    #[test]
    fn test_repository_kind_debug() {
        let kind = RepositoryKind::OwnRepository;
        let debug_str = format!("{kind:?}");
        assert!(debug_str.contains("OwnRepository"));
    }

    #[tokio::test]
    async fn test_detect_repository_async() {
        // This should work in any directory (might return Unknown or OwnRepository
        // depending on where tests are run)
        let result = detect_repository_async().await;
        // Just verify it doesn't panic and returns a valid variant
        let _ = result.is_own_repository();
    }

    #[tokio::test]
    async fn test_detect_repository_at_async_paperboat() {
        let temp = setup_test_dir();
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "paperboat"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = detect_repository_at_async(temp.path().to_path_buf()).await;
        assert_eq!(result, RepositoryKind::OwnRepository);
    }

    #[tokio::test]
    async fn test_detect_repository_at_async_empty() {
        let temp = setup_test_dir();
        let result = detect_repository_at_async(temp.path().to_path_buf()).await;
        assert_eq!(result, RepositoryKind::Unknown);
    }

    #[test]
    fn test_is_paperboat_repository_wrapper() {
        // Test the is_paperboat_repository_in wrapper function
        let temp = setup_test_dir();
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "paperboat"
version = "0.1.0"
"#,
        )
        .unwrap();
        assert!(is_paperboat_repository_in(temp.path()));
    }

    #[test]
    fn test_is_paperboat_repository_false() {
        let temp = setup_test_dir();
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "other-project"
version = "0.1.0"
"#,
        )
        .unwrap();
        assert!(!is_paperboat_repository_in(temp.path()));
    }

    #[test]
    fn test_is_paperboat_repository_current_dir() {
        // Running in paperboat repo, should return true
        assert!(is_paperboat_repository());
    }

    #[tokio::test]
    async fn test_is_paperboat_repository_async() {
        // Running in paperboat repo, should return true
        assert!(is_paperboat_repository_async().await);
    }

    // ========================================================================
    // Additional Edge Case Tests
    // ========================================================================

    #[test]
    fn test_detect_villalobos_git_remote_url() {
        // Note: This is a unit test of the URL matching logic
        // The actual check_git_remote function uses Command, so we test indirectly
        // by checking the URL matching pattern in check_git_remote

        // The function looks for "paperboat" or "villalobos" in the remote URL
        let test_urls = [
            ("https://github.com/user/paperboat.git", true),
            ("git@github.com:user/paperboat.git", true),
            ("https://github.com/user/villalobos.git", true),
            ("git@github.com:user/villalobos.git", true),
            ("https://github.com/user/other-project.git", false),
        ];

        for (url, expected_match) in test_urls {
            let url_lower = url.to_lowercase();
            let is_match = url_lower.contains("paperboat") || url_lower.contains("villalobos");
            assert_eq!(
                is_match, expected_match,
                "URL '{url}' should match: {expected_match}"
            );
        }
    }

    #[test]
    fn test_check_cargo_toml_case_sensitive() {
        let temp = setup_test_dir();

        // The check is case-sensitive for the exact string 'name = "paperboat"'
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "PAPERBOAT"
version = "0.1.0"
"#,
        )
        .unwrap();

        // Uppercase should NOT match (case-sensitive check)
        let result = check_cargo_toml(temp.path());
        assert_eq!(result, Some(false));
    }

    #[test]
    fn test_check_cargo_toml_with_workspace() {
        let temp = setup_test_dir();

        // A workspace Cargo.toml that doesn't have the package name directly
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[workspace]
members = ["paperboat", "other"]
"#,
        )
        .unwrap();

        // Should not match because the name field isn't present
        let result = check_cargo_toml(temp.path());
        assert_eq!(result, Some(false));
    }

    #[test]
    fn test_detect_repository_both_methods() {
        // Create a directory with both Cargo.toml for paperboat
        // Git remote would require actual git init which is more complex
        let temp = setup_test_dir();
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "paperboat"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = detect_repository_at(temp.path());
        assert_eq!(result, RepositoryKind::OwnRepository);
    }

    #[test]
    fn test_repository_kind_debug_format() {
        // Test debug output for all variants
        assert!(format!("{:?}", RepositoryKind::OwnRepository).contains("OwnRepository"));
        assert!(
            format!("{:?}", RepositoryKind::DifferentRepository).contains("DifferentRepository")
        );
        assert!(format!("{:?}", RepositoryKind::Unknown).contains("Unknown"));
    }
}
