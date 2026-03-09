//! Backend trait definition for agent providers.
//!
//! This module defines the `Backend` trait that all backend providers must implement.
//! The trait abstracts authentication, model discovery, and client/transport creation
//! to support multiple agent backends (Auggie, Cursor, etc.).
//!
//! # Transport Support
//!
//! Backends can support multiple transport protocols (ACP, CLI, etc.):
//! - Use `supported_transports()` to check which protocols a backend supports
//! - Use `create_transport()` to create a transport instance
//! - The legacy `create_client()` method remains for backward compatibility
//!
//! See [`TransportConfig`] for configuration options and [`super::transport`] for
//! transport protocol details.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::acp::AcpClientTrait;
use crate::models::ModelTier;

use super::transport::{AgentTransport, AgentType, TransportKind};

/// Type of agent for cache configuration.
///
/// Different agent types may have different cache directory structures
/// and configuration requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentCacheType {
    /// The orchestrator agent that coordinates other agents
    Orchestrator,
    /// The planner agent that decomposes tasks
    Planner,
    /// The worker agent that implements tasks
    #[allow(dead_code)]
    Worker,
}

impl AgentCacheType {
    /// Returns a string identifier for this agent cache type.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Planner => "planner",
            Self::Worker => "worker",
        }
    }
}

impl std::fmt::Display for AgentCacheType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Transport Configuration
// ============================================================================

/// Configuration for creating an agent transport.
///
/// Contains all parameters needed to create and initialize a transport connection.
/// This struct is designed to be extensible - new fields can be added without
/// breaking existing code.
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::{TransportConfig, AgentType};
/// use std::time::Duration;
///
/// let config = TransportConfig::new("/path/to/workspace")
///     .with_model("sonnet-4.6")
///     .with_request_timeout(Duration::from_secs(300))
///     .with_mcp_servers(vec![/* ... */]);
///
/// let transport = backend.create_transport(
///     TransportKind::Acp,
///     AgentType::Implementer,
///     config,
/// ).await?;
/// ```
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Working directory for the agent session.
    ///
    /// The agent will operate relative to this directory.
    pub workspace: PathBuf,

    /// Model identifier (e.g., "sonnet-4.6", "opus-4.6").
    ///
    /// This should be the backend-specific model ID returned by
    /// `Backend::resolve_tier()`.
    pub model: Option<String>,

    /// Timeout duration for requests.
    ///
    /// How long to wait for responses from the agent.
    /// Different operations may have different timeout requirements.
    pub request_timeout: Duration,

    /// MCP server configurations to enable for this session.
    ///
    /// Each entry is a JSON object with server configuration
    /// (typically name, command, args, env).
    pub mcp_servers: Vec<Value>,
}

impl TransportConfig {
    /// Create a new transport configuration with required workspace.
    ///
    /// Uses sensible defaults for optional fields:
    /// - `model`: None (must be set before creating session)
    /// - `request_timeout`: 5 minutes
    /// - `mcp_servers`: empty
    #[must_use]
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            model: None,
            request_timeout: Duration::from_secs(300), // 5 minutes
            mcp_servers: Vec::new(),
        }
    }

    /// Set the model identifier.
    #[must_use]
    #[allow(dead_code)]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the request timeout.
    #[must_use]
    #[allow(dead_code)]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the MCP server configurations.
    #[must_use]
    #[allow(dead_code)]
    pub fn with_mcp_servers(mut self, servers: Vec<Value>) -> Self {
        self.mcp_servers = servers;
        self
    }

    /// Add a single MCP server configuration.
    #[must_use]
    #[allow(dead_code)]
    pub fn with_mcp_server(mut self, server: Value) -> Self {
        self.mcp_servers.push(server);
        self
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("."),
            model: None,
            request_timeout: Duration::from_secs(300),
            mcp_servers: Vec::new(),
        }
    }
}

/// Trait defining a backend provider for agent communication.
///
/// This trait abstracts the differences between various agent backends
/// (e.g., Auggie, Cursor) while maintaining a unified interface for
/// the rest of the application.
///
/// Backends are responsible for:
/// - Authentication management
/// - Model discovery
/// - ACP client creation and configuration
/// - Cache directory setup
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::{Backend, BackendKind, AgentCacheType};
///
/// async fn run_with_backend() -> anyhow::Result<()> {
///     let backend = BackendKind::default().create();
///     backend.check_auth()?;
///
///     let models = backend.discover_models().await?;
///     let client = backend.create_client(
///         AgentCacheType::Worker,
///         None,
///         Duration::from_secs(60)
///     ).await?;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait Backend: Send + Sync {
    /// Human-readable name of the backend (e.g., "auggie", "cursor").
    fn name(&self) -> &'static str;

    /// Check if the backend is authenticated and ready to use.
    ///
    /// Returns `Ok(())` if authentication is valid, or an error with
    /// a helpful message if not authenticated.
    fn check_auth(&self) -> Result<()>;

    /// Discover available model tiers from this backend.
    ///
    /// Returns a set of model tiers that can be used with this backend.
    /// Each tier (e.g., `Sonnet`, `Opus`) maps to the best available
    /// model version for that tier in this backend.
    async fn available_tiers(&self) -> Result<HashSet<ModelTier>>;

    /// Resolve a model tier to the actual model ID string for this backend.
    ///
    /// Each backend may use different model ID formats:
    /// - Auggie: "sonnet4.5", "opus4.5", "haiku4.5"
    /// - Cursor: "sonnet-4.6", "opus-4.6", "gpt-5.1-codex-mini"
    ///
    /// Returns the model ID string to pass to `session_new()`.
    fn resolve_tier(&self, tier: ModelTier) -> Result<String>;

    /// Set up MCP server configuration for this backend.
    ///
    /// Called before creating ACP clients to ensure the MCP server is
    /// properly configured and enabled for the backend.
    ///
    /// # Arguments
    ///
    /// * `socket_path` - Path to the Unix socket for MCP communication
    ///
    /// # Backend-specific behavior
    ///
    /// - **Auggie**: No-op (mcpServers are passed in session/new)
    /// - **Cursor**: Writes to ~/.cursor/mcp.json and runs `agent mcp enable`
    async fn setup_mcp(&self, socket_path: &str) -> Result<()>;

    /// Clean up MCP server configuration for this backend.
    ///
    /// Called after the run completes to remove any MCP configuration
    /// that was set up for this session.
    fn cleanup_mcp(&self) -> Result<()>;

    /// Create an ACP client for the given agent type and timeout.
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent (orchestrator, planner, worker)
    /// * `cache_dir` - Optional path to the cache directory for this session.
    /// * `request_timeout` - Timeout duration for ACP requests.
    ///
    /// # Returns
    ///
    /// A boxed trait object implementing `AcpClientTrait`.
    ///
    /// # Permission Handling
    ///
    /// For Cursor backend, the agent_type determines tool permissions:
    /// - `Orchestrator`/`Planner`: File editing tools are denied
    /// - `Worker`: All tools are allowed
    #[allow(dead_code)]
    async fn create_client(
        &self,
        agent_type: AgentCacheType,
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>>;

    /// Set up the cache directory for a specific agent type.
    ///
    /// Creates and configures the cache directory structure needed for
    /// the specified agent type, including any tool filtering configuration.
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent to set up the cache for.
    /// * `removed_tools` - List of tool names to remove/deny for this agent.
    ///
    /// # Returns
    ///
    /// The path to the configured cache directory.
    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        removed_tools: &[&str],
    ) -> Result<PathBuf>;

    // ========================================================================
    // Transport Methods (new transport abstraction layer)
    // ========================================================================

    /// Returns the list of transport protocols this backend supports.
    ///
    /// Each backend may support different communication protocols:
    /// - **ACP**: JSON-RPC over stdin/stdout, supported by both Auggie and Cursor
    /// - **CLI**: Non-interactive CLI mode (currently Cursor only)
    ///
    /// Use this method to check if a transport kind is available before calling
    /// `create_transport()`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let backend = BackendKind::Cursor.create();
    /// let transports = backend.supported_transports();
    ///
    /// if transports.contains(&TransportKind::Cli) {
    ///     // Use CLI transport for better MCP support
    ///     let transport = backend.create_transport(
    ///         TransportKind::Cli,
    ///         AgentType::Implementer,
    ///         config,
    ///     ).await?;
    /// }
    /// ```
    fn supported_transports(&self) -> Vec<TransportKind> {
        // Default: only ACP is supported (both Auggie and Cursor support this)
        vec![TransportKind::Acp]
    }

    /// Create a transport instance for the specified protocol.
    ///
    /// Creates and returns a transport configured for the given protocol,
    /// agent type, and configuration. The transport is ready to be initialized
    /// via `transport.initialize()`.
    ///
    /// # Arguments
    ///
    /// * `kind` - The transport protocol to use (ACP, CLI, etc.)
    /// * `agent_type` - The type of agent (Orchestrator, Planner, Implementer)
    /// * `config` - Transport configuration including workspace, model, timeout
    ///
    /// # Returns
    ///
    /// A boxed transport implementing [`AgentTransport`].
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The transport kind is not in `supported_transports()`
    /// - Transport creation fails (e.g., binary not found, configuration error)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let config = TransportConfig::new("/path/to/workspace")
    ///     .with_model("sonnet-4.6")
    ///     .with_request_timeout(Duration::from_secs(300));
    ///
    /// let mut transport = backend.create_transport(
    ///     TransportKind::Acp,
    ///     AgentType::Implementer,
    ///     config,
    /// ).await?;
    ///
    /// transport.initialize().await?;
    /// ```
    ///
    /// # Note
    ///
    /// This is the new transport-based API. The legacy `create_client()` method
    /// is still available for backward compatibility but will be deprecated.
    async fn create_transport(
        &self,
        kind: TransportKind,
        _agent_type: AgentType,
        _config: TransportConfig,
    ) -> Result<Box<dyn AgentTransport>> {
        // Default implementation: check if transport is supported and return error
        // Individual backends should override this to provide actual implementations
        let supported = self.supported_transports();
        if !supported.contains(&kind) {
            return Err(anyhow!(
                "Transport '{}' is not supported by {} backend. Supported transports: {}",
                kind,
                self.name(),
                supported
                    .iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // If we get here, the transport is "supported" but not implemented
        Err(anyhow!(
            "Transport '{}' is listed as supported by {} backend but not yet implemented. \
            This is a bug - please report it.",
            kind,
            self.name()
        ))
    }

    /// Get a hint for how to authenticate with this backend.
    ///
    /// Returns a command or instruction that users can follow to
    /// authenticate (e.g., "auggie login", "cursor login").
    fn login_hint(&self) -> &'static str;

    /// Get a detailed error message for authentication failures.
    ///
    /// This provides a more detailed, backend-specific message that
    /// can include multiple authentication methods or troubleshooting steps.
    fn auth_error_message(&self) -> String {
        format!(
            "{} is not authenticated.\n\n\
            Please run '{}' first to authenticate, then try again.",
            self.name(),
            self.login_hint()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_transport_config_new() {
        let config = TransportConfig::new("/path/to/workspace");
        assert_eq!(config.workspace, PathBuf::from("/path/to/workspace"));
        assert!(config.model.is_none());
        assert_eq!(config.request_timeout, Duration::from_secs(300));
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_transport_config_with_model() {
        let config = TransportConfig::new("/workspace").with_model("sonnet-4.6");
        assert_eq!(config.model, Some("sonnet-4.6".to_string()));
    }

    #[test]
    fn test_transport_config_with_timeout() {
        let config =
            TransportConfig::new("/workspace").with_request_timeout(Duration::from_secs(60));
        assert_eq!(config.request_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_transport_config_with_mcp_servers() {
        let servers = vec![serde_json::json!({"name": "test"})];
        let config = TransportConfig::new("/workspace").with_mcp_servers(servers.clone());
        assert_eq!(config.mcp_servers.len(), 1);
    }

    #[test]
    fn test_transport_config_with_mcp_server() {
        let config = TransportConfig::new("/workspace")
            .with_mcp_server(serde_json::json!({"name": "server1"}))
            .with_mcp_server(serde_json::json!({"name": "server2"}));
        assert_eq!(config.mcp_servers.len(), 2);
    }

    #[test]
    fn test_transport_config_default() {
        let config = TransportConfig::default();
        assert_eq!(config.workspace, PathBuf::from("."));
        assert!(config.model.is_none());
        assert_eq!(config.request_timeout, Duration::from_secs(300));
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_transport_config_builder_chaining() {
        let config = TransportConfig::new("/path")
            .with_model("opus-4.6")
            .with_request_timeout(Duration::from_secs(600))
            .with_mcp_server(serde_json::json!({"name": "paperboat"}));

        assert_eq!(config.workspace, PathBuf::from("/path"));
        assert_eq!(config.model, Some("opus-4.6".to_string()));
        assert_eq!(config.request_timeout, Duration::from_secs(600));
        assert_eq!(config.mcp_servers.len(), 1);
    }
}
