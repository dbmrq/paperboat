// Allow some clippy lints for this new module - can be cleaned up later
#![allow(clippy::doc_markdown)]
#![allow(clippy::len_zero)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::use_self)]

//! Cursor CLI backend implementation.
//!
//! This module implements the [`Backend`] trait for Cursor's agent CLI.
//!
//! # Authentication
//!
//! Cursor supports multiple authentication methods:
//! - `CURSOR_API_KEY` environment variable
//! - `CURSOR_AUTH_TOKEN` environment variable
//! - Interactive login via `agent login`
//!
//! # Differences from Auggie
//!
//! - Cursor requires an `authenticate` ACP call after `initialize`
//! - Model discovery via `cursor-agent --list-models` with different ID format
//! - Cache configuration uses `cli-config.json` with `permissions.deny` format

pub mod acp;
pub mod acp_transport;
pub mod auth;
pub mod cache;
pub mod cli;
pub mod mcp_config;
mod models;
pub mod permission;

// Re-export commonly used items for convenience
pub use acp_transport::CursorAcpTransport;
pub use cli::CursorCliTransport;
pub use permission::PermissionPolicy;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::acp::AcpClientTrait;
use crate::backend::transport::{AgentTransport, AgentType, TransportKind};
use crate::backend::{AgentCacheType, Backend, TransportConfig};
use crate::models::ModelTier;

/// Cursor CLI backend implementation.
///
/// This is a zero-sized type that implements the [`Backend`] trait for
/// Cursor's agent infrastructure. Cursor uses the `agent` command-line tool
/// with ACP (Agent Communication Protocol) for AI interactions.
///
/// # Key Differences from Auggie
///
/// - Requires an `authenticate` ACP call after `initialize` (handled by `CursorAcpClient`)
/// - Does not support `agent model list` - uses hardcoded model defaults
/// - Cache configuration uses `cli-config.json` with `permissions.deny` format
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::cursor::CursorBackend;
/// use paperboat::backend::Backend;
///
/// let backend = CursorBackend::new();
/// backend.check_auth()?;
/// let models = backend.discover_models().await?;
/// ```
pub struct CursorBackend;

impl CursorBackend {
    /// Create a new Cursor backend instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for CursorBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for CursorBackend {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn check_auth(&self) -> Result<()> {
        auth::check_cursor_auth()
    }

    /// Discover available model tiers for Cursor.
    ///
    /// Runs `cursor-agent --list-models` and parses the output to discover
    /// available tiers. Maps Cursor model IDs to ModelTier values.
    async fn available_tiers(&self) -> Result<HashSet<ModelTier>> {
        models::discover_cursor_tiers().await
    }

    fn resolve_tier(&self, tier: ModelTier) -> Result<String> {
        // Cursor uses model IDs like "sonnet-4.6", "opus-4.6", "gpt-5.1-codex-mini"
        // We map each tier to the best available model ID
        match tier {
            ModelTier::Auto => Ok("auto".to_string()),
            ModelTier::Opus => Ok("opus-4.6".to_string()),
            ModelTier::Sonnet => Ok("sonnet-4.6".to_string()),
            ModelTier::Codex => Ok("gpt-5.3-codex".to_string()),
            ModelTier::CodexMini => Ok("gpt-5.1-codex-mini".to_string()),
            ModelTier::Gemini => Ok("gemini-3.1-pro".to_string()),
            ModelTier::GeminiFlash => Ok("gemini-3-flash".to_string()),
            ModelTier::Grok => Ok("grok".to_string()),
            ModelTier::Composer => Ok("composer-1.5".to_string()),
            // Cursor doesn't have Haiku
            ModelTier::Haiku => Err(anyhow!(
                "Model tier 'haiku' is not available in Cursor backend. \
                Use 'codex-mini' or 'grok' for cheap models."
            )),
        }
    }

    async fn setup_mcp(&self, socket_path: &str) -> Result<()> {
        // For CLI transport, we don't pre-register MCP servers.
        // Instead, enable_mcp_for_agent() is called just-in-time before spawning
        // each agent, which registers ONLY that agent type's MCP server.
        // This ensures each agent sees only its own tools.
        tracing::debug!(
            "CLI transport will configure MCP per-agent (socket: {})",
            socket_path
        );
        Ok(())
    }

    fn cleanup_mcp(&self) -> Result<()> {
        mcp_config::unregister_paperboat_mcp()
    }

    async fn create_client(
        &self,
        agent_type: AgentCacheType,
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>> {
        // Determine permission policy based on agent type
        // This controls which tools each agent can use via permission requests
        let permission_policy = match agent_type {
            AgentCacheType::Orchestrator => acp::PermissionPolicy::for_orchestrator(),
            AgentCacheType::Planner => acp::PermissionPolicy::for_planner(),
            AgentCacheType::Worker => acp::PermissionPolicy::for_implementer(),
        };

        let client =
            acp::CursorAcpClient::spawn_with_policy(cache_dir, request_timeout, permission_policy)
                .await?;
        Ok(Box::new(client))
    }

    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        removed_tools: &[&str],
    ) -> Result<PathBuf> {
        cache::setup_cursor_cache(agent_type, removed_tools)
    }

    fn login_hint(&self) -> &'static str {
        "agent login"
    }

    fn auth_error_message(&self) -> String {
        "Cursor CLI is not authenticated.\n\n\
        Please run 'agent login' first to authenticate, then try again.\n\
        Alternatively, set CURSOR_API_KEY or CURSOR_AUTH_TOKEN environment variable."
            .to_string()
    }

    // ========================================================================
    // Transport Methods
    // ========================================================================

    /// Returns the list of transport protocols Cursor supports.
    ///
    /// Cursor supports both ACP and CLI transports:
    /// - **CLI** (recommended): Properly loads MCP servers from `~/.cursor/mcp.json`
    /// - **ACP**: Has broken MCP support (Cursor bug, no ETA for fix)
    ///
    /// Use CLI transport for production workloads until Cursor fixes ACP MCP support.
    fn supported_transports(&self) -> Vec<TransportKind> {
        vec![TransportKind::Acp, TransportKind::Cli]
    }

    /// Create a transport instance for the specified protocol.
    ///
    /// Creates either a CLI or ACP transport based on the requested kind,
    /// configured with appropriate permission policy for the agent type.
    ///
    /// # Arguments
    ///
    /// * `kind` - `TransportKind::Cli` (recommended) or `TransportKind::Acp`
    /// * `agent_type` - Controls which tools are allowed (Orchestrator, Planner, Implementer)
    /// * `config` - Transport configuration including workspace, model, timeout
    ///
    /// # Recommended Usage
    ///
    /// ```ignore
    /// let backend = CursorBackend::new();
    /// let config = TransportConfig::new("/workspace")
    ///     .with_model("sonnet-4.6")
    ///     .with_request_timeout(Duration::from_secs(300));
    ///
    /// // Use CLI transport for proper MCP support
    /// let transport = backend.create_transport(
    ///     TransportKind::Cli,  // Recommended over Acp
    ///     AgentType::Implementer,
    ///     config,
    /// ).await?;
    /// ```
    async fn create_transport(
        &self,
        kind: TransportKind,
        agent_type: AgentType,
        config: TransportConfig,
    ) -> Result<Box<dyn AgentTransport>> {
        // Validate transport kind is supported
        if !self.supported_transports().contains(&kind) {
            return Err(anyhow!(
                "Transport '{}' is not supported by Cursor backend. Supported: acp, cli",
                kind
            ));
        }

        // Determine permission policy based on agent type
        let permission_policy = match agent_type {
            AgentType::Orchestrator => PermissionPolicy::for_orchestrator(),
            AgentType::Planner => PermissionPolicy::for_planner(),
            AgentType::Implementer => PermissionPolicy::for_implementer(),
        };

        match kind {
            TransportKind::Cli => {
                tracing::debug!(
                    "Creating Cursor CLI transport for {} agent",
                    agent_type.as_str()
                );

                // Create CLI transport with agent type for MCP server selection
                let transport = CursorCliTransport::new(agent_type.as_str(), permission_policy);

                // Configure the transport with workspace and model
                // Note: CLI transport gets these from create_session, but we can
                // pre-configure them here for consistency
                if let Some(model) = &config.model {
                    // The CLI transport will use the model from SessionConfig
                    tracing::debug!("CLI transport will use model: {}", model);
                }

                Ok(Box::new(transport))
            }
            TransportKind::Acp => {
                tracing::debug!(
                    "Creating Cursor ACP transport for {} agent",
                    agent_type.as_str()
                );
                tracing::warn!(
                    "⚠️ Using ACP transport - note that Cursor ACP has broken MCP support. \
                    Consider using CLI transport instead."
                );

                // Create ACP transport with permission policy and timeout
                let mut transport =
                    CursorAcpTransport::new(permission_policy, config.request_timeout);

                // Pre-configure workspace, model, and MCP servers
                transport.set_workspace(config.workspace);
                if let Some(model) = config.model {
                    transport.set_model(model);
                }
                transport.set_mcp_servers(config.mcp_servers);

                Ok(Box::new(transport))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_backend_name() {
        let backend = CursorBackend::new();
        assert_eq!(backend.name(), "cursor");
    }

    #[test]
    fn test_cursor_backend_login_hint() {
        let backend = CursorBackend::new();
        assert_eq!(backend.login_hint(), "agent login");
    }

    #[test]
    fn test_cursor_backend_auth_error_message() {
        let backend = CursorBackend::new();
        let msg = backend.auth_error_message();
        assert!(msg.contains("agent login"));
        assert!(msg.contains("CURSOR_API_KEY"));
    }

    #[test]
    fn test_cursor_backend_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CursorBackend>();
    }

    #[test]
    fn test_cursor_backend_default() {
        let _backend: CursorBackend = Default::default();
    }

    #[test]
    fn test_supported_transports() {
        let backend = CursorBackend::new();
        let transports = backend.supported_transports();

        assert!(transports.contains(&TransportKind::Acp));
        assert!(transports.contains(&TransportKind::Cli));
        assert_eq!(transports.len(), 2);
    }

    #[test]
    fn test_supported_transports_includes_both() {
        let backend = CursorBackend::new();
        let transports = backend.supported_transports();

        // Both ACP and CLI should be supported
        assert!(
            transports.contains(&TransportKind::Acp),
            "ACP should be supported"
        );
        assert!(
            transports.contains(&TransportKind::Cli),
            "CLI should be supported"
        );
    }

    // Note: create_transport() tests require async runtime and would spawn
    // actual processes. Integration tests are in tests/integration/.

    // Note: discover_models() tests are in the models module since they
    // require mocking the cursor-agent command or running against real CLI
}
