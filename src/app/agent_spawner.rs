//! Generic agent spawning.
//!
//! This module provides a generalized interface for spawning worker agents
//! (implementers, verifiers, etc.) with support for concurrent execution.

use super::types::{format_duration_human, truncate_for_log};
use super::App;
use crate::agents::AgentRole;
use crate::mcp_server::{AgentSpec, ToolResponse, WaitMode};
use anyhow::{Context, Result};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Result of an agent's execution.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// The role of the agent (e.g., "implementer")
    pub role: String,
    /// The task that was assigned to the agent.
    /// Note: This is part of the public API but may not be read directly in all cases.
    #[allow(dead_code)]
    pub task: String,
    /// Whether the agent completed successfully
    pub success: bool,
    /// Optional message from the agent.
    /// Note: Part of the API, may not be read directly in all cases.
    #[allow(dead_code)]
    pub message: Option<String>,
}

impl App {
    /// Handle implement tool call, returning a `ToolResponse`.
    /// Used by sequential mode in tests where the `SessionRouter` is not active.
    pub(crate) async fn handle_implement_with_response(
        &mut self,
        task: &str,
        request_id: &str,
    ) -> ToolResponse {
        match self.handle_implement_inner(task).await {
            Ok(summary) => ToolResponse::success(request_id.to_string(), summary),
            Err(e) => ToolResponse::failure(request_id.to_string(), e.to_string()),
        }
    }

    /// Inner implement logic that can fail.
    async fn handle_implement_inner(&mut self, task: &str) -> Result<String> {
        let start_time = std::time::Instant::now();

        // Create implementer writer (this assigns the implementer number)
        let mut impl_writer = self
            .current_scope
            .implementer_writer()
            .await
            .context("Failed to create implementer writer")?;

        // Get the implementer name for logging
        let impl_name = impl_writer.agent_name();
        tracing::info!(
            "🔨 [{}] Starting implementation: {}",
            impl_name,
            truncate_for_log(task, 100)
        );

        // Spawn implementer
        let (impl_session, impl_prompt) = self.spawn_implementer(task).await?;
        impl_writer.set_session_id(impl_session.clone());
        if let Err(e) = impl_writer
            .write_header_with_prompt(task, &impl_prompt)
            .await
        {
            tracing::warn!("Failed to write implementer header: {}", e);
        }

        // Wait for implementer to finish (with timeout)
        let result = self
            .wait_for_session_output(&impl_session, &mut impl_writer)
            .await;

        let success = result.is_ok();
        let elapsed = start_time.elapsed();
        let elapsed_str = format_duration_human(elapsed);

        // Finalize implementer log
        if let Err(e) = impl_writer.finalize(success).await {
            tracing::warn!("Failed to finalize implementer log: {}", e);
        }

        if !success {
            tracing::error!(
                "❌ [{}] Implementation FAILED after {} - task: {}",
                impl_name,
                elapsed_str,
                truncate_for_log(task, 80)
            );
            return Err(anyhow::anyhow!("Implementation failed for task: {task}"));
        }

        tracing::info!(
            "✅ [{}] Implementation complete ({}) - {}",
            impl_name,
            elapsed_str,
            truncate_for_log(task, 60)
        );

        Ok(format!("Task completed: {task}"))
    }

    /// Spawn an implementer agent (convenience wrapper for spawn_agent_with_spec).
    /// Returns (`session_id`, prompt) so the prompt can be logged.
    pub(crate) async fn spawn_implementer(&mut self, task: &str) -> Result<(String, String)> {
        let spec = AgentSpec {
            role: "implementer".to_string(),
            task: task.to_string(),
            prompt: None,
            tools: None,
        };
        self.spawn_agent_with_spec(&spec).await
    }

    /// Spawn an agent based on the AgentSpec.
    ///
    /// Determines the prompt and removed_tools based on the role:
    /// - Custom: requires prompt and tools from spec
    /// - Template roles (implementer, verifier, explorer): uses registry templates
    /// - Unknown roles: falls back to implementer behavior with warning
    ///
    /// Returns (`session_id`, prompt) so the prompt can be logged.
    pub(crate) async fn spawn_agent_with_spec(&mut self, spec: &AgentSpec) -> Result<(String, String)> {
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

        // Determine prompt and removed_tools based on role
        let (prompt, removed_tools) = match AgentRole::from_str(&spec.role) {
            Some(AgentRole::Custom) => {
                // Custom: require prompt and tools from spec
                let custom_prompt = spec.prompt.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Custom agent requires 'prompt'"))?
                    .clone();
                let allowed_tools = spec.tools.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Custom agent requires 'tools' whitelist"))?;

                // Derive removed_tools from allowed_tools
                let all_tools = vec!["str-replace-editor", "save-file", "remove-files",
                                     "launch-process", "kill-process", "read-process",
                                     "write-process", "list-processes", "web-search", "web-fetch"];
                let removed: Vec<String> = all_tools.iter()
                    .filter(|t| !allowed_tools.contains(&t.to_string()))
                    .map(|s| s.to_string())
                    .collect();

                (custom_prompt, removed)
            }
            Some(role) => {
                // Template role: get from registry
                let template = self.agent_registry.get(&role)
                    .ok_or_else(|| anyhow::anyhow!("No template for role: {:?}", role))?;

                let prompt = template.prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal);
                let removed = template.removed_tools.iter().map(|s| s.to_string()).collect();

                (prompt, removed)
            }
            None => {
                // Unknown role - treat as implementer for backward compatibility
                tracing::warn!("Unknown agent role '{}', treating as implementer", spec.role);
                let template = self.agent_registry.get(&AgentRole::Implementer).unwrap();
                let prompt = template.prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal);
                (prompt, vec![])
            }
        };

        // Build environment variables for the MCP server
        let mut env_vars = vec![
            json!({"name": "VILLALOBOS_SOCKET", "value": socket_path}),
            json!({"name": "VILLALOBOS_AGENT_TYPE", "value": spec.role.clone()}),
        ];

        // Add removed tools to environment if any
        if !removed_tools.is_empty() {
            env_vars.push(json!({"name": "VILLALOBOS_REMOVED_TOOLS", "value": removed_tools.join(",")}));
        }

        // Configure MCP server with agent type
        // Use unique name based on role to prevent MCP server caching/reuse between agent types
        let mcp_servers = vec![json!({
            "name": format!("villalobos-{}", spec.role),
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server"],
            "env": env_vars
        })];

        let response = self
            .acp_worker
            .session_new(
                self.model_config.implementer_model.as_str(),
                mcp_servers,
                &cwd,
            )
            .await?;

        tracing::debug!("🔨 {} prompt:\n{}", spec.role, prompt);

        self.acp_worker
            .session_prompt(&response.session_id, &prompt)
            .await?;

        Ok((response.session_id, prompt))
    }

    /// Handle decompose tool call, returning a `ToolResponse`.
    pub(crate) async fn handle_decompose_with_response(
        &mut self,
        task: &str,
        request_id: &str,
    ) -> ToolResponse {
        match self.handle_decompose_inner(task).await {
            Ok(summary) => ToolResponse::success(request_id.to_string(), summary),
            Err(e) => ToolResponse::failure(request_id.to_string(), e.to_string()),
        }
    }

    /// Spawn an agent asynchronously based on an `AgentSpec`.
    ///
    /// Returns a tuple of (`session_id`, receiver) where the receiver will
    /// receive the `AgentResult` when the agent completes.
    ///
    /// Supports all agent roles:
    /// - Template roles (implementer, verifier, explorer): uses registry templates
    /// - Custom: requires prompt and tools from spec
    /// - Unknown roles: falls back to implementer behavior with warning
    ///
    /// # Arguments
    ///
    /// * `spec` - The agent specification describing the role and task
    ///
    /// # Returns
    ///
    /// A tuple containing the session ID and a oneshot receiver for the result.
    #[allow(dead_code)]
    pub(crate) async fn spawn_agent_async(
        &mut self,
        spec: &AgentSpec,
    ) -> Result<(String, oneshot::Receiver<AgentResult>)> {
        // Create implementer writer
        let mut impl_writer = self
            .current_scope
            .implementer_writer()
            .await
            .context("Failed to create implementer writer")?;

        let impl_name = impl_writer.agent_name();
        let task = spec.task.clone();
        let role = spec.role.clone();

        tracing::info!(
            "🔨 [{}] Starting async agent spawn (role={}): {}",
            impl_name,
            role,
            truncate_for_log(&task, 100)
        );

        // Spawn the agent session using the spec-based spawner
        let (session_id, agent_prompt) = self.spawn_agent_with_spec(spec).await?;
        impl_writer.set_session_id(session_id.clone());
        if let Err(e) = impl_writer
            .write_header_with_prompt(&task, &agent_prompt)
            .await
        {
            tracing::warn!("Failed to write agent header: {}", e);
        }

        // Create the completion channel
        let (result_tx, result_rx) = oneshot::channel();

        // Register session with router
        let session_rx = self.register_session(&session_id).await;

        // Clone what we need for the spawned task
        let session_router = Arc::clone(&self.session_router);
        let session_id_clone = session_id.clone();
        let timeout_duration = self.timeout_config.session_timeout;

        // Spawn a task to wait for the agent and send result
        tokio::spawn(async move {
            let start_time = std::time::Instant::now();

            // Wait for the session to complete
            // Note: This simplified version just waits for session_finished
            // In practice, we'd need to handle tool calls from the agent
            let success = Self::wait_for_agent_completion(session_rx, timeout_duration).await;

            let elapsed = start_time.elapsed();
            let elapsed_str = format_duration_human(elapsed);

            // Finalize the writer
            if let Err(e) = impl_writer.finalize(success).await {
                tracing::warn!("Failed to finalize implementer log: {}", e);
            }

            // Unregister from router
            {
                let mut router = session_router.write().await;
                router.unregister(&session_id_clone);
            }

            let message = if success {
                tracing::info!(
                    "✅ [async] Agent {} completed ({}) - {}",
                    role,
                    elapsed_str,
                    truncate_for_log(&task, 60)
                );
                Some(format!("Task completed: {task}"))
            } else {
                tracing::error!(
                    "❌ [async] Agent {} FAILED after {} - {}",
                    role,
                    elapsed_str,
                    truncate_for_log(&task, 80)
                );
                Some(format!("Task failed: {task}"))
            };

            let result = AgentResult {
                role,
                task,
                success,
                message,
            };

            // Send result (ignore if receiver dropped)
            let _ = result_tx.send(result);
        });

        Ok((session_id, result_rx))
    }

    /// Internal helper to wait for an agent to complete via session messages.
    ///
    /// This is a simplified version that just waits for the session to finish.
    /// For full functionality, tool calls would need to be handled externally.
    async fn wait_for_agent_completion(
        mut session_rx: tokio::sync::mpsc::Receiver<serde_json::Value>,
        timeout: std::time::Duration,
    ) -> bool {
        let result = tokio::time::timeout(timeout, async {
            while let Some(msg) = session_rx.recv().await {
                // Check for session_finished
                if let Some(params) = msg.get("params") {
                    if let Some(update) = params.get("update") {
                        if let Some(session_update) =
                            update.get("sessionUpdate").and_then(|v| v.as_str())
                        {
                            if session_update == "session_finished" {
                                return true;
                            }
                        }
                    }
                }
            }
            false
        })
        .await;

        result.unwrap_or(false)
    }

    /// Spawn multiple agents concurrently with the specified wait mode.
    ///
    /// # Arguments
    ///
    /// * `agents` - Vector of agent specifications to spawn
    /// * `wait` - How to wait for the agents:
    ///   - `WaitMode::All`: Wait for all agents to complete, return all results
    ///   - `WaitMode::Any`: Wait for the first agent to complete, return that result
    ///   - `WaitMode::None`: Return immediately with empty results (fire and forget)
    ///
    /// # Returns
    ///
    /// A vector of `AgentResult` for completed agents (may be empty for `WaitMode::None`).
    #[allow(dead_code)]
    pub(crate) async fn spawn_agents_concurrent(
        &mut self,
        agents: Vec<AgentSpec>,
        wait: WaitMode,
    ) -> Vec<AgentResult> {
        if agents.is_empty() {
            return Vec::new();
        }

        let agent_count = agents.len();
        tracing::info!(
            "🚀 Spawning {} agents concurrently (wait={:?})",
            agent_count,
            wait
        );

        // Spawn all agents and collect their receivers
        let mut receivers = Vec::with_capacity(agent_count);
        let mut spawn_errors = Vec::new();

        for spec in agents {
            match self.spawn_agent_async(&spec).await {
                Ok((_session_id, rx)) => {
                    receivers.push((spec.role.clone(), spec.task.clone(), rx));
                }
                Err(e) => {
                    tracing::error!("Failed to spawn agent [{}]: {}", spec.role, e);
                    spawn_errors.push(AgentResult {
                        role: spec.role,
                        task: spec.task,
                        success: false,
                        message: Some(format!("Failed to spawn: {e}")),
                    });
                }
            }
        }

        match wait {
            WaitMode::None => {
                // Fire and forget - return immediately with spawn errors only
                tracing::info!("🔥 Fire-and-forget mode: {} agents spawned", receivers.len());
                spawn_errors
            }
            WaitMode::Any => {
                // Wait for the first agent to complete
                if receivers.is_empty() {
                    return spawn_errors;
                }

                let (tx, mut rx) = tokio::sync::mpsc::channel(1);

                for (role, task, receiver) in receivers {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        match receiver.await {
                            Ok(result) => {
                                let _ = tx.send(result).await;
                            }
                            Err(_) => {
                                let _ = tx
                                    .send(AgentResult {
                                        role,
                                        task,
                                        success: false,
                                        message: Some("Agent channel closed".to_string()),
                                    })
                                    .await;
                            }
                        }
                    });
                }
                drop(tx); // Drop our copy so channel closes when all senders done

                if let Some(first_result) = rx.recv().await {
                    tracing::info!(
                        "⚡ First agent completed: [{}] success={}",
                        first_result.role,
                        first_result.success
                    );
                    let mut results = spawn_errors;
                    results.push(first_result);
                    results
                } else {
                    spawn_errors
                }
            }
            WaitMode::All => {
                // Wait for all agents to complete
                let mut results = spawn_errors;

                for (role, task, receiver) in receivers {
                    match receiver.await {
                        Ok(result) => {
                            results.push(result);
                        }
                        Err(_) => {
                            results.push(AgentResult {
                                role,
                                task,
                                success: false,
                                message: Some("Agent channel closed".to_string()),
                            });
                        }
                    }
                }

                let success_count = results.iter().filter(|r| r.success).count();
                tracing::info!(
                    "✅ All {} agents completed: {}/{} successful",
                    results.len(),
                    success_count,
                    results.len()
                );

                results
            }
        }
    }
}

