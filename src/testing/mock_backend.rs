//! Mock backend for testing.
//!
//! Provides a mock implementation of the `Backend` trait that can be configured
//! for different test scenarios, enabling deterministic testing without requiring
//! real authentication or cache setup.
//!
//! # Configurable Behavior
//!
//! The `MockBackend` supports configuring:
//! - Authentication success/failure via `with_auth_failure()`
//! - Custom models via `with_models()`
//! - Custom backend name via `with_name()`
//!
//! # Example
//!
//! ```ignore
//! use paperboat::testing::MockBackend;
//!
//! // Default mock backend (always succeeds)
//! let backend = MockBackend::new();
//! backend.check_auth()?; // Always succeeds
//!
//! // Mock backend that fails auth
//! let failing_backend = MockBackend::builder()
//!     .auth_fails(true)
//!     .build();
//! assert!(failing_backend.check_auth().is_err());
//! ```

use anyhow::{bail, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::acp::AcpClientTrait;
use crate::backend::{AgentCacheType, Backend};
use crate::models::ModelTier;
use crate::testing::MockAcpClient;

/// A mock backend with configurable behavior for testing.
///
/// This backend is designed for testing scenarios where you need to control:
/// - Whether authentication checks pass or fail
/// - What models are returned from discovery
/// - The backend name for error messages
///
/// # Default Behavior
///
/// By default, `MockBackend`:
/// - Always passes authentication checks
/// - Returns a single mock model (Sonnet 4.5)
/// - Uses "mock" as the backend name
/// - Returns fake cache paths (no filesystem operations)
///
/// # Builder Pattern
///
/// Use `MockBackend::builder()` to create a backend with custom behavior:
///
/// ```ignore
/// let backend = MockBackend::builder()
///     .name("test-backend")
///     .auth_fails(true)
///     .models(vec![...])
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct MockBackend {
    /// Optional custom name for the backend (defaults to "mock")
    name: Option<&'static str>,
    /// Whether authentication should fail
    auth_fails: bool,
    /// Custom tiers to return from available_tiers()
    custom_tiers: Option<HashSet<ModelTier>>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            name: None,
            auth_fails: false,
            custom_tiers: None,
        }
    }
}

impl MockBackend {
    /// Create a new mock backend with default behavior.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for configuring a mock backend.
    #[must_use]
    pub fn builder() -> MockBackendBuilder {
        MockBackendBuilder::default()
    }

    /// Create a mock backend with a custom name.
    #[must_use]
    pub fn with_name(name: &'static str) -> Self {
        Self {
            name: Some(name),
            ..Self::default()
        }
    }

    /// Create a mock backend that fails authentication.
    #[must_use]
    pub fn with_auth_failure() -> Self {
        Self {
            auth_fails: true,
            ..Self::default()
        }
    }
}

/// Builder for creating `MockBackend` instances with custom configuration.
#[derive(Debug, Default)]
pub struct MockBackendBuilder {
    name: Option<&'static str>,
    auth_fails: bool,
    custom_tiers: Option<HashSet<ModelTier>>,
}

impl MockBackendBuilder {
    /// Set a custom name for the backend.
    #[must_use]
    pub fn name(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        self
    }

    /// Configure whether authentication should fail.
    #[must_use]
    pub fn auth_fails(mut self, fails: bool) -> Self {
        self.auth_fails = fails;
        self
    }

    /// Set custom tiers to return from `available_tiers()`.
    #[must_use]
    pub fn tiers(mut self, tiers: HashSet<ModelTier>) -> Self {
        self.custom_tiers = Some(tiers);
        self
    }

    /// Build the mock backend.
    #[must_use]
    pub fn build(self) -> MockBackend {
        MockBackend {
            name: self.name,
            auth_fails: self.auth_fails,
            custom_tiers: self.custom_tiers,
        }
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &'static str {
        self.name.unwrap_or("mock")
    }

    fn check_auth(&self) -> Result<()> {
        if self.auth_fails {
            bail!("Mock authentication failure (configured for testing)")
        }
        Ok(())
    }

    async fn available_tiers(&self) -> Result<HashSet<ModelTier>> {
        // Return custom tiers if configured, otherwise default tiers
        if let Some(ref tiers) = self.custom_tiers {
            return Ok(tiers.clone());
        }
        Ok([ModelTier::Sonnet, ModelTier::Opus, ModelTier::Haiku]
            .into_iter()
            .collect())
    }

    fn resolve_tier(&self, tier: ModelTier) -> Result<String> {
        // Mock: just return the tier name as the model string
        Ok(tier.as_str().to_string())
    }

    async fn setup_mcp(&self, _socket_path: &str) -> Result<()> {
        // Mock backend doesn't need MCP setup
        Ok(())
    }

    fn cleanup_mcp(&self) -> Result<()> {
        // Mock backend doesn't need MCP cleanup
        Ok(())
    }

    async fn create_client(
        &self,
        _agent_type: AgentCacheType,
        _cache_dir: Option<&str>,
        _request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>> {
        // Return an empty mock ACP client
        Ok(Box::new(MockAcpClient::empty()))
    }

    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        _removed_tools: &[&str],
    ) -> Result<PathBuf> {
        // Return a fake path based on agent type - no actual filesystem operations
        Ok(PathBuf::from(format!(
            "/tmp/mock-cache/{}",
            agent_type.as_str()
        )))
    }

    fn login_hint(&self) -> &'static str {
        "mock login"
    }

    fn auth_error_message(&self) -> String {
        "Mock backend authentication failed (this should not happen in tests)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_backend_name() {
        let backend = MockBackend::new();
        assert_eq!(backend.name(), "mock");
    }

    #[test]
    fn test_mock_backend_custom_name() {
        let backend = MockBackend::with_name("test-backend");
        assert_eq!(backend.name(), "test-backend");
    }

    #[test]
    fn test_mock_backend_check_auth() {
        let backend = MockBackend::new();
        assert!(backend.check_auth().is_ok());
    }

    #[test]
    fn test_mock_backend_setup_cache() {
        let backend = MockBackend::new();
        let path = backend
            .setup_agent_cache(AgentCacheType::Orchestrator, &[])
            .unwrap();
        assert!(path.to_str().unwrap().contains("orchestrator"));
    }

    #[test]
    fn test_mock_backend_login_hint() {
        let backend = MockBackend::new();
        assert_eq!(backend.login_hint(), "mock login");
    }

    #[tokio::test]
    async fn test_mock_backend_available_tiers() {
        let backend = MockBackend::new();
        let tiers = backend.available_tiers().await.unwrap();
        assert!(tiers.contains(&ModelTier::Sonnet));
        assert!(tiers.contains(&ModelTier::Opus));
        assert!(tiers.contains(&ModelTier::Haiku));
    }

    // ========================================================================
    // Configurable Behavior Tests
    // ========================================================================

    #[test]
    fn test_mock_backend_with_auth_failure() {
        let backend = MockBackend::with_auth_failure();
        let result = backend.check_auth();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Mock authentication failure"));
    }

    #[test]
    fn test_mock_backend_builder_default() {
        let backend = MockBackend::builder().build();
        assert_eq!(backend.name(), "mock");
        assert!(backend.check_auth().is_ok());
    }

    #[test]
    fn test_mock_backend_builder_with_name() {
        let backend = MockBackend::builder().name("custom").build();
        assert_eq!(backend.name(), "custom");
    }

    #[test]
    fn test_mock_backend_builder_auth_fails() {
        let backend = MockBackend::builder().auth_fails(true).build();
        assert!(backend.check_auth().is_err());

        let backend_ok = MockBackend::builder().auth_fails(false).build();
        assert!(backend_ok.check_auth().is_ok());
    }

    #[tokio::test]
    async fn test_mock_backend_builder_with_custom_tiers() {
        let custom_tiers: HashSet<ModelTier> =
            [ModelTier::Opus, ModelTier::Codex].into_iter().collect();

        let backend = MockBackend::builder().tiers(custom_tiers).build();
        let tiers = backend.available_tiers().await.unwrap();

        assert_eq!(tiers.len(), 2);
        assert!(tiers.contains(&ModelTier::Opus));
        assert!(tiers.contains(&ModelTier::Codex));
        assert!(!tiers.contains(&ModelTier::Haiku));
    }

    #[test]
    fn test_mock_backend_builder_chaining() {
        let backend = MockBackend::builder()
            .name("chained")
            .auth_fails(false)
            .build();

        assert_eq!(backend.name(), "chained");
        assert!(backend.check_auth().is_ok());
    }

    #[test]
    fn test_mock_backend_setup_cache_all_agent_types() {
        let backend = MockBackend::new();

        let orchestrator_path = backend
            .setup_agent_cache(AgentCacheType::Orchestrator, &[])
            .unwrap();
        assert!(orchestrator_path.to_str().unwrap().contains("orchestrator"));

        let planner_path = backend
            .setup_agent_cache(AgentCacheType::Planner, &[])
            .unwrap();
        assert!(planner_path.to_str().unwrap().contains("planner"));

        let worker_path = backend
            .setup_agent_cache(AgentCacheType::Worker, &[])
            .unwrap();
        assert!(worker_path.to_str().unwrap().contains("worker"));
    }

    #[tokio::test]
    async fn test_mock_backend_create_client() {
        let backend = MockBackend::new();
        let client = backend
            .create_client(
                AgentCacheType::Worker,
                Some("/tmp/test"),
                Duration::from_secs(60),
            )
            .await;
        assert!(client.is_ok());
    }

    #[test]
    fn test_mock_backend_auth_error_message() {
        let backend = MockBackend::new();
        let msg = backend.auth_error_message();
        assert!(msg.contains("Mock backend"));
    }

    #[test]
    fn test_mock_backend_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockBackend>();
    }

    #[test]
    fn test_mock_backend_clone() {
        let backend = MockBackend::builder()
            .name("cloneable")
            .auth_fails(true)
            .build();
        let cloned = backend.clone();

        assert_eq!(cloned.name(), "cloneable");
        assert!(cloned.check_auth().is_err());
    }
}
