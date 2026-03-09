//! Planner agent handling.

use super::retry::{is_transient_error, RetryConfig};
use super::types::PLANNER_PROMPT;
use super::App;
use crate::acp::SessionMode;
use crate::backend::transport::{SessionConfig, SessionInfo};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

impl App {
    /// Spawn a planner agent.
    /// Returns (`session_id`, `model`, `prompt`) so the model and prompt can be logged.
    #[tracing::instrument(skip(self), fields(agent_type = "planner", session_id))]
    pub(crate) async fn spawn_planner(&mut self, task: &str) -> Result<(String, String, String)> {
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();

        // Get the path to the current binary and socket
        let binary_path =
            std::env::current_exe().context("Failed to get current executable path")?;
        let socket_path = self
            .socket_path
            .as_ref()
            .context("Socket not set up")?
            .to_string_lossy()
            .to_string();

        // Configure MCP server with planner agent type (only gets create_task and complete tools)
        // Use unique name "paperboat-planner" to prevent MCP server caching/reuse between agent types
        // Pass --socket directly to avoid env var caching issues across auggie sessions
        let mcp_servers = vec![json!({
            "name": "paperboat-planner",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server", "--socket", &socket_path],
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

        Ok((response.session_id, model, prompt))
    }

    /// Create a planner session with retry logic.
    ///
    /// This handles transient MCP server startup errors by retrying the session
    /// creation with exponential backoff.
    /// Returns (`SessionInfo`, `model`) so the model can be recorded in logs.
    async fn create_planner_session_with_retry(
        &mut self,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<(SessionInfo, String)> {
        let retry_config = RetryConfig::from_env();
        let model = self.resolve_planner_model()?;
        let mut attempt = 0;
        let mut delay = retry_config.initial_delay;

        loop {
            attempt += 1;

            // Create session config with Agent mode - planner needs to call MCP tools
            // Note: Cursor's "plan" mode is read-only and can't call tools
            let config = SessionConfig::new(&model, cwd)
                .with_mcp_servers(mcp_servers.clone())
                .with_mode(SessionMode::Agent);

            match self.acp_planner.create_session(config).await {
                Ok(response) => {
                    if attempt > 1 {
                        tracing::info!(
                            "🔄 Planner create_session succeeded on attempt {}/{}",
                            attempt,
                            retry_config.max_retries + 1
                        );
                    }
                    return Ok((response, model));
                }
                Err(e) => {
                    let is_transient = is_transient_error(&e);
                    let can_retry = attempt <= retry_config.max_retries && is_transient;

                    if can_retry {
                        tracing::warn!(
                            "⚠️ Planner create_session failed (attempt {}/{}): {}. Retrying in {:?}...",
                            attempt,
                            retry_config.max_retries + 1,
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

    /// Resolve the planner model configuration to an actual model string.
    fn resolve_planner_model(&self) -> anyhow::Result<String> {
        // Resolve fallback chain to a tier
        let tier = self
            .model_config
            .planner_model
            .resolve(&self.model_config.available_tiers)?;

        // Planner doesn't use auto-resolution
        // Convert tier to backend-specific model string
        self.backend.resolve_tier(tier)
    }
}
