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

    fn resolve_tier(
        &self,
        tier: ModelTier,
        effort: Option<crate::models::EffortLevel>,
    ) -> Result<Vec<String>> {
        use crate::models::EffortLevel;

        // Cursor uses model IDs like "sonnet-4.6", "opus-4.6", "gpt-5.1-codex-mini"
        //
        // This returns a fallback chain of models to try in order.
        // The chain includes multiple versions and effort variants to maximize
        // the chance of finding an available model.

        let effort = effort.unwrap_or_default();

        // Helper to build Claude fallback chain (only base and -thinking exist)
        let claude_chain = |versions: &[&str]| -> Vec<String> {
            let mut chain = Vec::new();
            for v in versions {
                match effort {
                    EffortLevel::High | EffortLevel::XHigh => {
                        chain.push(format!("{v}-thinking"));
                        chain.push(v.to_string());
                    }
                    EffortLevel::Low | EffortLevel::Medium => {
                        chain.push(v.to_string());
                        chain.push(format!("{v}-thinking"));
                    }
                }
            }
            chain
        };

        // Helper to build GPT/Codex fallback chain (full effort spectrum)
        let gpt_chain = |versions: &[&str]| -> Vec<String> {
            let mut chain = Vec::new();
            for v in versions {
                match effort {
                    EffortLevel::XHigh => {
                        chain.push(format!("{v}-xhigh"));
                        chain.push(format!("{v}-high"));
                        chain.push(v.to_string());
                        chain.push(format!("{v}-low"));
                    }
                    EffortLevel::High => {
                        chain.push(format!("{v}-high"));
                        chain.push(v.to_string());
                        chain.push(format!("{v}-xhigh"));
                        chain.push(format!("{v}-low"));
                    }
                    EffortLevel::Medium => {
                        chain.push(v.to_string());
                        chain.push(format!("{v}-high"));
                        chain.push(format!("{v}-low"));
                    }
                    EffortLevel::Low => {
                        chain.push(format!("{v}-low"));
                        chain.push(v.to_string());
                        chain.push(format!("{v}-high"));
                    }
                }
            }
            chain
        };

        match tier {
            ModelTier::Auto => Ok(vec!["auto".to_string()]),

            // Opus: try 4.6, then 4.5
            ModelTier::Opus => Ok(claude_chain(&["opus-4.6", "opus-4.5"])),

            // Sonnet: try 4.6, then 4.5
            ModelTier::Sonnet => Ok(claude_chain(&["sonnet-4.6", "sonnet-4.5"])),

            // GPT: try 5.4, 5.3, 5.2, 5.1
            ModelTier::Gpt => Ok(gpt_chain(&["gpt-5.4", "gpt-5.3", "gpt-5.2", "gpt-5.1"])),

            // OpenAI: comprehensive chain of all OpenAI models
            // Includes GPT, Codex variants - try to find ANY working OpenAI model
            ModelTier::OpenAI => {
                let mut chain = gpt_chain(&["gpt-5.4", "gpt-5.3", "gpt-5.2", "gpt-5.1"]);
                // Also include codex variants as fallback
                chain.extend(gpt_chain(&[
                    "gpt-5.3-codex",
                    "gpt-5.2-codex",
                    "gpt-5.1-codex-max",
                ]));
                Ok(chain)
            }

            // Codex: specialized coding models
            ModelTier::Codex => Ok(gpt_chain(&[
                "gpt-5.3-codex",
                "gpt-5.2-codex",
                "gpt-5.1-codex-max",
            ])),

            // CodexMini: cheap/fast coding model
            ModelTier::CodexMini => Ok(vec!["gpt-5.1-codex-mini".to_string()]),

            // Gemini: try 3.1, then 3
            ModelTier::Gemini => Ok(vec![
                "gemini-3.1-pro".to_string(),
                "gemini-3-pro".to_string(),
            ]),

            ModelTier::GeminiFlash => Ok(vec!["gemini-3-flash".to_string()]),

            ModelTier::Grok => Ok(vec!["grok".to_string()]),

            // Composer: try 1.5, then 1
            ModelTier::Composer => Ok(vec!["composer-1.5".to_string(), "composer-1".to_string()]),

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
    use crate::backend::transport::SessionConfig;
    use serde_json::json;
    use serial_test::serial;
    use std::env;

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
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

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

    #[tokio::test]
    async fn test_create_transport_cli_returns_cli_transport() {
        let backend = CursorBackend::new();
        let config = TransportConfig::new("/tmp/workspace")
            .with_model("sonnet-4.6")
            .with_mcp_server(json!({
                "name": "paperboat",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }));

        let mut transport = backend
            .create_transport(TransportKind::Cli, AgentType::Implementer, config)
            .await
            .unwrap();

        assert_eq!(transport.kind(), TransportKind::Cli);

        let session = transport
            .create_session(
                SessionConfig::new("sonnet-4.6", "/tmp/workspace").with_mcp_servers(vec![json!({
                    "name": "paperboat",
                    "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
                })]),
            )
            .await
            .unwrap();

        assert!(!session.session_id.is_empty());
    }

    #[tokio::test]
    async fn test_create_transport_acp_returns_uninitialized_acp_transport() {
        let backend = CursorBackend::new();
        let config = TransportConfig::new("/tmp/workspace")
            .with_model("sonnet-4.6")
            .with_request_timeout(Duration::from_secs(1))
            .with_mcp_server(json!({
                "name": "paperboat",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }));

        let mut transport = backend
            .create_transport(TransportKind::Acp, AgentType::Planner, config)
            .await
            .unwrap();

        assert_eq!(transport.kind(), TransportKind::Acp);

        let err = transport
            .create_session(SessionConfig::new("sonnet-4.6", "/tmp/workspace"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not initialized"));
    }

    #[tokio::test]
    #[serial]
    async fn test_setup_mcp_is_noop_for_cursor_backend() {
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let backend = CursorBackend::new();

        backend.setup_mcp("/tmp/paperboat.sock").await.unwrap();

        assert!(!temp.path().join(".cursor/mcp.json").exists());
    }

    #[test]
    #[serial]
    fn test_cleanup_mcp_removes_paperboat_servers_and_preserves_unrelated_entries() {
        let temp = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            serde_json::json!({
                "mcpServers": {
                    "github": {
                        "command": "gh",
                        "args": ["mcp", "serve"]
                    },
                    "paperboat-planner": {
                        "command": "/old/paperboat",
                        "args": ["--stale"]
                    },
                    "paperboat-implementer-session123": {
                        "command": "/old/paperboat",
                        "args": ["--stale"]
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        CursorBackend::new().cleanup_mcp().unwrap();

        let config: mcp_config::CursorMcpConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert!(config.mcp_servers.contains_key("github"));
        assert!(!config.mcp_servers.keys().any(|name| name.starts_with("paperboat-")));
    }

    // ========================================================================
    // Transport Selection Branch Tests
    // ========================================================================

    #[tokio::test]
    async fn test_create_transport_cli_for_orchestrator() {
        let backend = CursorBackend::new();
        let config = TransportConfig::new("/tmp/workspace")
            .with_model("sonnet-4.6")
            .with_mcp_server(json!({
                "name": "paperboat-orchestrator",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }));

        let transport = backend
            .create_transport(TransportKind::Cli, AgentType::Orchestrator, config)
            .await
            .unwrap();

        assert_eq!(transport.kind(), TransportKind::Cli);
    }

    #[tokio::test]
    async fn test_create_transport_cli_for_planner() {
        let backend = CursorBackend::new();
        let config = TransportConfig::new("/tmp/workspace")
            .with_model("sonnet-4.6")
            .with_mcp_server(json!({
                "name": "paperboat-planner",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }));

        let transport = backend
            .create_transport(TransportKind::Cli, AgentType::Planner, config)
            .await
            .unwrap();

        assert_eq!(transport.kind(), TransportKind::Cli);
    }

    #[tokio::test]
    async fn test_create_transport_acp_for_orchestrator() {
        let backend = CursorBackend::new();
        let config = TransportConfig::new("/tmp/workspace")
            .with_model("sonnet-4.6")
            .with_request_timeout(Duration::from_secs(1))
            .with_mcp_server(json!({
                "name": "paperboat-orchestrator",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }));

        let transport = backend
            .create_transport(TransportKind::Acp, AgentType::Orchestrator, config)
            .await
            .unwrap();

        assert_eq!(transport.kind(), TransportKind::Acp);
    }

    #[tokio::test]
    async fn test_create_transport_acp_for_planner() {
        let backend = CursorBackend::new();
        let config = TransportConfig::new("/tmp/workspace")
            .with_model("sonnet-4.6")
            .with_request_timeout(Duration::from_secs(1))
            .with_mcp_server(json!({
                "name": "paperboat-planner",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }));

        let transport = backend
            .create_transport(TransportKind::Acp, AgentType::Planner, config)
            .await
            .unwrap();

        assert_eq!(transport.kind(), TransportKind::Acp);
    }

    #[tokio::test]
    async fn test_create_transport_cli_session_creates_unique_session_id() {
        let backend = CursorBackend::new();
        let config1 = TransportConfig::new("/tmp/workspace").with_model("sonnet-4.6");
        let config2 = TransportConfig::new("/tmp/workspace").with_model("sonnet-4.6");

        let mut transport1 = backend
            .create_transport(TransportKind::Cli, AgentType::Implementer, config1)
            .await
            .unwrap();

        let mut transport2 = backend
            .create_transport(TransportKind::Cli, AgentType::Implementer, config2)
            .await
            .unwrap();

        // Each transport session should have unique IDs
        let session1 = transport1
            .create_session(SessionConfig::new("sonnet-4.6", "/tmp/workspace"))
            .await
            .unwrap();
        let session2 = transport2
            .create_session(SessionConfig::new("sonnet-4.6", "/tmp/workspace"))
            .await
            .unwrap();

        assert_ne!(session1.session_id, session2.session_id);
    }

    #[tokio::test]
    async fn test_resolve_tier_returns_fallback_chain() {
        let backend = CursorBackend::new();

        // Auto tier
        let auto_models = backend.resolve_tier(ModelTier::Auto, None).unwrap();
        assert_eq!(auto_models, vec!["auto"]);

        // Sonnet tier should return multiple options
        let sonnet_models = backend.resolve_tier(ModelTier::Sonnet, None).unwrap();
        assert!(sonnet_models.len() >= 2, "Sonnet should have fallback chain");
        assert!(sonnet_models.iter().any(|m| m.contains("sonnet")));

        // Opus tier
        let opus_models = backend.resolve_tier(ModelTier::Opus, None).unwrap();
        assert!(opus_models.len() >= 2, "Opus should have fallback chain");
        assert!(opus_models.iter().any(|m| m.contains("opus")));
    }

    #[tokio::test]
    async fn test_resolve_tier_haiku_returns_error() {
        let backend = CursorBackend::new();

        let result = backend.resolve_tier(ModelTier::Haiku, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not available"));
    }

    // Note: discover_models() tests are in the models module since they
    // require mocking the cursor-agent command or running against real CLI
}
