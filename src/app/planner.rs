//! Planner agent handling.

use super::retry::{is_transient_error, RetryConfig};
use super::socket::{setup_agent_socket, AgentSocketHandle};
use super::types::PLANNER_PROMPT;
use super::App;
use crate::acp::SessionMode;
use crate::backend::transport::{SessionConfig, SessionInfo, TransportKind};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

/// Result of spawning a planner agent.
pub struct PlannerSession {
    /// The session ID for the planner session
    pub session_id: String,
    /// The model used for this session
    pub model: String,
    /// The prompt sent to the planner
    pub prompt: String,
    /// Socket handle for CLI transport (must be kept alive during the session)
    socket_handle: Option<AgentSocketHandle>,
    /// Tool receiver extracted from the socket handle (for passing to `wait_for_session_output`)
    tool_rx: Option<super::types::ToolReceiver>,
}

impl PlannerSession {
    /// Take the tool receiver for use in `wait_for_session_output`.
    /// Returns None if there's no CLI socket handle (e.g., ACP transport).
    pub const fn take_tool_rx(&mut self) -> Option<super::types::ToolReceiver> {
        self.tool_rx.take()
    }

    /// Clean up the socket handle when done.
    #[allow(dead_code)] // Public API for explicit resource cleanup
    pub fn cleanup(self) {
        if let Some(handle) = self.socket_handle {
            handle.cleanup();
        }
    }
}

impl App {
    /// Spawn a planner agent.
    /// Returns a `PlannerSession` containing session info and socket handle.
    /// The socket handle must be kept alive until the planner session completes.
    #[tracing::instrument(skip(self), fields(agent_type = "planner", session_id))]
    pub(crate) async fn spawn_planner(&mut self, task: &str) -> Result<PlannerSession> {
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();

        // Get the path to the current binary
        let binary_path =
            std::env::current_exe().context("Failed to get current executable path")?;

        // For CLI transport, create a unique socket to prevent MCP server caching.
        // This ensures each planner session gets its own MCP server process.
        let (socket_address, cli_socket_handle) = if self.acp_planner.kind() == TransportKind::Cli {
            let agent_id = format!("cli-plan-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            let socket_handle = setup_agent_socket(&agent_id).await.with_context(|| {
                format!("Failed to create unique socket for CLI planner: {agent_id}")
            })?;
            let addr = socket_handle.socket_address.as_str().to_string();
            tracing::debug!(
                "🔌 Created unique socket for CLI planner: {} -> {}",
                agent_id,
                addr
            );
            (addr, Some(socket_handle))
        } else {
            let addr = self
                .socket_address
                .as_ref()
                .context("Socket not set up")?
                .as_str()
                .to_string();
            (addr, None)
        };

        // Configure MCP server with planner agent type (only gets create_task and complete tools)
        // Use unique name "paperboat-planner" to prevent MCP server caching/reuse between agent types
        // Pass --socket directly to avoid env var caching issues across auggie sessions
        let mcp_servers = vec![json!({
            "name": "paperboat-planner",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server", "--socket", &socket_address],
            "env": [{
                "name": "PAPERBOAT_AGENT_TYPE",
                "value": "planner"
            }]
        })];

        // Create session with retry logic for transient MCP server startup errors
        let (response, model) = self
            .create_planner_session_with_retry(mcp_servers, &cwd)
            .await?;

        let prompt = PLANNER_PROMPT.replace("{task}", task);

        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &response.session_id);
        tracing::debug!("📝 Planner prompt:\n{}", prompt);

        self.acp_planner
            .send_prompt(&response.session_id, &prompt)
            .await?;

        // Extract tool_rx from the socket handle if present (for CLI transport)
        let (socket_handle, tool_rx) = match cli_socket_handle {
            Some(mut handle) => {
                // Take the tool_rx out of the handle so we can pass it to wait_for_session_output
                // We need to keep the handle alive (it has the listener task), but use its tool_rx
                let rx = std::mem::replace(
                    &mut handle.tool_rx,
                    tokio::sync::mpsc::channel(1).1, // Replace with dummy receiver
                );
                (Some(handle), Some(rx))
            }
            None => (None, None),
        };

        Ok(PlannerSession {
            session_id: response.session_id,
            model,
            prompt,
            socket_handle,
            tool_rx,
        })
    }

    /// Create a planner session with model fallback chain and retry logic.
    ///
    /// This tries each model in the chain, and for each model:
    /// - Handles transient MCP server startup errors with exponential backoff
    /// - Falls back to the next model if "model not available" error occurs
    ///
    /// Returns (`SessionInfo`, `actual_model`) so the model can be recorded in logs.
    async fn create_planner_session_with_retry(
        &mut self,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<(SessionInfo, String)> {
        use super::retry::is_model_not_available_error;

        let retry_config = RetryConfig::from_env();
        let model_chain = self.resolve_planner_model()?;

        // Track the last error for reporting if all models fail
        let mut last_error: Option<anyhow::Error> = None;

        for model in &model_chain {
            let mut attempt = 0;
            let mut delay = retry_config.initial_delay;

            loop {
                attempt += 1;

                // Create session config with Agent mode - planner needs to call MCP tools
                // Note: Cursor's "plan" mode is read-only and can't call tools
                let config = SessionConfig::new(model, cwd)
                    .with_mcp_servers(mcp_servers.clone())
                    .with_mode(SessionMode::Agent);

                match self.acp_planner.create_session(config).await {
                    Ok(response) => {
                        if attempt > 1 {
                            tracing::info!(
                                "🔄 Planner create_session succeeded on attempt {}/{} with model {}",
                                attempt,
                                retry_config.max_retries + 1,
                                model
                            );
                        }
                        return Ok((response, model.clone()));
                    }
                    Err(e) => {
                        // Check if model is not available - try next model
                        if is_model_not_available_error(&e) {
                            tracing::debug!(
                                "Model '{}' not available for planner, trying next in chain...",
                                model
                            );
                            last_error = Some(e);
                            break; // Move to next model in chain
                        }

                        let is_transient = is_transient_error(&e);
                        let can_retry = attempt <= retry_config.max_retries && is_transient;

                        if can_retry {
                            tracing::warn!(
                                "⚠️ Planner create_session failed (attempt {}/{}, model {}): {}. Retrying in {:?}...",
                                attempt,
                                retry_config.max_retries + 1,
                                model,
                                e,
                                delay
                            );
                            tokio::time::sleep(delay).await;

                            // Exponential backoff with cap
                            delay = Duration::from_secs_f64(
                                (delay.as_secs_f64() * retry_config.backoff_multiplier)
                                    .min(retry_config.max_delay.as_secs_f64()),
                            );
                        } else {
                            // Non-transient, non-model error - fail immediately
                            let reason = if is_transient {
                                "exhausted retries"
                            } else {
                                "non-transient error"
                            };
                            tracing::error!(
                                "❌ Planner create_session failed after {attempt} attempt(s) ({reason}): {e:#}",
                            );
                            return Err(e).context(format!(
                                "Planner create_session failed after {attempt} attempt(s)"
                            ));
                        }
                    }
                }
            }
        }

        // All models in the chain failed
        Err(last_error
            .unwrap_or_else(|| anyhow::anyhow!("No models in fallback chain"))
            .context(format!(
                "All models in fallback chain failed for planner: {model_chain:?}"
            )))
    }

    /// Resolve the planner model configuration to a fallback chain of model strings.
    ///
    /// Returns a vector of model strings to try in order (first is preferred).
    fn resolve_planner_model(&self) -> anyhow::Result<Vec<String>> {
        // Resolve fallback chain to a tier
        let tier = self
            .model_config
            .planner_model
            .resolve(&self.model_config.available_tiers)?;

        // Planner doesn't use auto-resolution
        // Convert tier to backend-specific model fallback chain with effort level
        self.backend
            .resolve_tier(tier, Some(self.model_config.planner_effort))
    }
}
