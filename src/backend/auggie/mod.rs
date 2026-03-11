// Allow some clippy lints for this new module - can be cleaned up later
#![allow(clippy::doc_markdown)]
#![allow(clippy::uninlined_format_args)]

//! Auggie backend implementation.
//!
//! This module implements the [`Backend`] trait for Augment's Auggie CLI.
//!
//! # Authentication
//!
//! Auggie requires authentication via `auggie login`, which stores credentials
//! in `~/.augment/session.json`.
//!
//! # Features
//!
//! - Model discovery via `auggie model list`
//! - Cache configuration uses `settings.json` with `removedTools` format
//! - No additional ACP calls required after `initialize` (unlike Cursor)
//!
//! # Example
//!
//! ```ignore
//! use paperboat::backend::auggie::AuggieBackend;
//! use paperboat::backend::Backend;
//!
//! let backend = AuggieBackend::new();
//! backend.check_auth()?;
//! let models = backend.discover_models().await?;
//! ```

pub mod acp;
pub mod auth;
pub mod cache;
mod models;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::acp::{AcpClient, AcpClientTrait};
use crate::backend::transport::{AgentTransport, AgentType, TransportKind};
use crate::backend::{AgentCacheType, Backend, TransportConfig};
use crate::models::ModelTier;

/// Auggie CLI backend implementation.
///
/// This is a zero-sized type that implements the [`Backend`] trait for
/// Augment's Auggie CLI. Auggie uses the `auggie` command-line tool
/// with ACP (Agent Communication Protocol) for AI interactions.
///
/// # Key Features
///
/// - Simple authentication: just requires `auggie login`
/// - Model discovery: supports `auggie model list` command
/// - Cache configuration: uses `settings.json` with `removedTools` array
///
/// # Example
///
/// ```ignore
/// use paperboat::backend::auggie::AuggieBackend;
/// use paperboat::backend::Backend;
///
/// let backend = AuggieBackend::new();
/// backend.check_auth()?;
/// let models = backend.discover_models().await?;
/// let client = backend.create_client(None, Duration::from_secs(60)).await?;
/// ```
pub struct AuggieBackend;

impl AuggieBackend {
    /// Create a new Auggie backend instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for AuggieBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for AuggieBackend {
    fn name(&self) -> &'static str {
        "auggie"
    }

    fn check_auth(&self) -> Result<()> {
        auth::check_auggie_auth()
    }

    async fn available_tiers(&self) -> Result<HashSet<ModelTier>> {
        models::discover_auggie_tiers().await
    }

    fn resolve_tier(
        &self,
        tier: ModelTier,
        _effort: Option<crate::models::EffortLevel>,
    ) -> Result<Vec<String>> {
        // Auggie uses format like "haiku4.5", "sonnet4.5", "opus4.5"
        // Returns a fallback chain of versions to try (newest first)
        // Note: Auggie doesn't support effort levels - the parameter is ignored
        match tier {
            ModelTier::Auto => Ok(vec!["auto".to_string()]),
            // Opus: try 4.5 (latest known)
            ModelTier::Opus => Ok(vec!["opus4.5".to_string()]),
            // Sonnet: try 4.5 (latest known)
            ModelTier::Sonnet => Ok(vec!["sonnet4.5".to_string()]),
            // Haiku: try 4.5 (latest known)
            ModelTier::Haiku => Ok(vec!["haiku4.5".to_string()]),
            // Auggie doesn't have these tiers
            ModelTier::Gpt
            | ModelTier::OpenAI
            | ModelTier::Codex
            | ModelTier::CodexMini
            | ModelTier::Gemini
            | ModelTier::GeminiFlash
            | ModelTier::Grok
            | ModelTier::Composer => Err(anyhow!(
                "Model tier '{}' is not available in Auggie backend",
                tier
            )),
        }
    }

    async fn setup_mcp(&self, _socket_path: &str) -> Result<()> {
        // Auggie passes mcpServers dynamically in session/new, no pre-setup needed
        Ok(())
    }

    fn cleanup_mcp(&self) -> Result<()> {
        // Auggie doesn't need cleanup - MCP config is per-session
        Ok(())
    }

    async fn create_client(
        &self,
        _agent_type: AgentCacheType,
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>> {
        // Auggie uses cache_dir with settings.json to control tool access
        // The permission policy is enforced via the cache directory configuration
        let client = AcpClient::spawn_with_timeout(cache_dir, request_timeout).await?;
        Ok(Box::new(client))
    }

    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        removed_tools: &[&str],
    ) -> Result<PathBuf> {
        cache::setup_auggie_cache(agent_type, removed_tools)
    }

    // ========================================================================
    // Transport Methods
    // ========================================================================

    /// Returns the list of transport protocols Auggie supports.
    ///
    /// Auggie only supports ACP transport. CLI transport is not available.
    fn supported_transports(&self) -> Vec<TransportKind> {
        vec![TransportKind::Acp]
    }

    /// Create a transport instance for the specified protocol.
    ///
    /// # Arguments
    ///
    /// * `kind` - The transport protocol (only `TransportKind::Acp` is supported)
    /// * `agent_type` - The type of agent (Orchestrator, Planner, Implementer)
    /// * `config` - Transport configuration including workspace, model, timeout
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `kind` is not `TransportKind::Acp` (CLI not supported)
    /// - The transport cannot be created (e.g., auggie binary not found)
    async fn create_transport(
        &self,
        kind: TransportKind,
        agent_type: AgentType,
        config: TransportConfig,
    ) -> Result<Box<dyn AgentTransport>> {
        // Check if the requested transport is supported
        if kind != TransportKind::Acp {
            return Err(anyhow!(
                "Transport '{}' is not supported by Auggie backend. \
                Only ACP transport is available. \
                Use TransportKind::Acp instead.",
                kind
            ));
        }

        // Determine cache directory based on agent type
        // This controls tool filtering via settings.json
        let cache_dir = match agent_type {
            AgentType::Orchestrator => {
                let path = self.setup_agent_cache(AgentCacheType::Orchestrator, &[])?;
                if path.as_os_str().is_empty() {
                    None
                } else {
                    Some(path.to_string_lossy().to_string())
                }
            }
            AgentType::Planner => {
                let path = self.setup_agent_cache(AgentCacheType::Planner, &[])?;
                if path.as_os_str().is_empty() {
                    None
                } else {
                    Some(path.to_string_lossy().to_string())
                }
            }
            AgentType::Implementer => {
                // Implementers use default cache (full tool access)
                None
            }
        };

        let transport =
            acp::AuggieAcpTransport::new(cache_dir.as_deref(), config.request_timeout).await?;

        Ok(Box::new(transport))
    }

    fn login_hint(&self) -> &'static str {
        "auggie login"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auggie_backend_name() {
        let backend = AuggieBackend::new();
        assert_eq!(backend.name(), "auggie");
    }

    #[test]
    fn test_auggie_backend_login_hint() {
        let backend = AuggieBackend::new();
        assert_eq!(backend.login_hint(), "auggie login");
    }

    #[test]
    fn test_auggie_backend_auth_error_message() {
        let backend = AuggieBackend::new();
        let msg = backend.auth_error_message();
        assert!(msg.contains("auggie"));
        assert!(msg.contains("auggie login"));
    }

    #[test]
    fn test_auggie_backend_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AuggieBackend>();
    }

    #[test]
    fn test_auggie_backend_default() {
        let _ = AuggieBackend;
    }

    // ========================================================================
    // Transport Tests
    // ========================================================================

    #[test]
    fn test_auggie_supported_transports_only_acp() {
        let backend = AuggieBackend::new();
        let transports = backend.supported_transports();
        assert_eq!(transports.len(), 1);
        assert_eq!(transports[0], TransportKind::Acp);
    }

    #[test]
    fn test_auggie_supported_transports_no_cli() {
        let backend = AuggieBackend::new();
        let transports = backend.supported_transports();
        assert!(!transports.contains(&TransportKind::Cli));
    }

    #[tokio::test]
    async fn test_auggie_create_transport_rejects_cli() {
        let backend = AuggieBackend::new();
        let config = TransportConfig::new("/tmp/test");

        let result = backend
            .create_transport(TransportKind::Cli, AgentType::Implementer, config)
            .await;

        assert!(result.is_err());
        let err = result.err().expect("expected error");
        assert!(err.to_string().contains("not supported"));
        assert!(err.to_string().contains("Auggie"));
    }
}
