//! Planner agent handling.

use super::types::PLANNER_PROMPT;
use super::App;
use anyhow::{Context, Result};
use serde_json::json;

impl App {
    /// Spawn a planner agent.
    /// Returns (`session_id`, prompt) so the prompt can be logged.
    pub(crate) async fn spawn_planner(&mut self, task: &str) -> Result<(String, String)> {
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

        // Configure MCP server with planner agent type (only gets write_plan and complete tools)
        // Use unique name "villalobos-planner" to prevent MCP server caching/reuse between agent types
        let mcp_servers = vec![json!({
            "name": "villalobos-planner",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server"],
            "env": [
                {"name": "VILLALOBOS_SOCKET", "value": socket_path},
                {"name": "VILLALOBOS_AGENT_TYPE", "value": "planner"}
            ]
        })];

        let response = self
            .acp_planner
            .session_new(self.model_config.planner_model.as_str(), mcp_servers, &cwd)
            .await?;

        let prompt = PLANNER_PROMPT.replace("{task}", task);
        tracing::debug!("📝 Planner prompt:\n{}", prompt);

        self.acp_planner
            .session_prompt(&response.session_id, &prompt)
            .await?;

        Ok((response.session_id, prompt))
    }
}
