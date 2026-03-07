//! Generic agent spawning.
//!
//! This module provides a generalized interface for spawning worker agents
//! (implementers, verifiers, etc.) with support for concurrent execution.
//!
//! # Retry Logic
//!
//! Agent spawning includes automatic retry logic for transient errors like
//! MCP server startup failures. The retry configuration can be customized
//! via environment variables:
//! - `PAPERBOAT_SPAWN_RETRIES`: Max retry attempts (default: 3)
//! - `PAPERBOAT_SPAWN_RETRY_DELAY_MS`: Initial delay in ms (default: 500)

use super::agent_session_handler::{
    spawn_agent_handler_task, spawn_notification_router, AgentHandlerParams,
};
use super::concurrent_spawner::{
    create_spawn_error, extract_role_and_task, handle_fire_and_forget, wait_for_all, wait_for_any,
};
use super::retry::{retry_async, RetryConfig};
use super::socket::{setup_agent_socket, AgentSocketHandle};
use super::spawn_config::{build_custom_prompt, validate_no_unreplaced_placeholders, AgentResult};
use super::tool_filtering::{build_mcp_server_config, compute_removed_tools};
use super::types::truncate_for_log;
use super::App;
use crate::acp::AcpClientTrait;
use crate::mcp_server::{AgentSpec, ResolvedAgentSpec, WaitMode};
use crate::tasks::TaskStatus;
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

/// Spawn and initialize an ACP client with retry logic.
///
/// This function handles the common pattern of spawning auggie, initializing
/// the connection, and creating a session - all with automatic retry for
/// transient errors like MCP server startup failures.
async fn spawn_acp_with_retry(
    request_timeout: Duration,
    model: &str,
    mcp_servers: Vec<Value>,
    cwd: &str,
    agent_id: &str,
) -> Result<(crate::acp::AcpClient, crate::acp::SessionNewResponse)> {
    let retry_config = RetryConfig::from_env();
    let operation_name = format!("spawn ACP session for {}", agent_id);

    retry_async(&retry_config, &operation_name, || {
        let mcp_servers = mcp_servers.clone();
        let model = model.to_string();
        let cwd = cwd.to_string();
        let agent_id = agent_id.to_string();

        async move {
            // Create a fresh auggie instance
            let mut agent_acp = crate::acp::AcpClient::spawn_with_timeout(
                None, // Use default cache
                request_timeout,
            )
            .await
            .with_context(|| format!("Failed to spawn auggie for agent_id={}", agent_id))?;

            // Initialize the ACP connection
            agent_acp.initialize().await.with_context(|| {
                format!("Failed to initialize auggie for agent_id={}", agent_id)
            })?;

            // Create the session with MCP servers
            let response = agent_acp
                .session_new(&model, mcp_servers, &cwd)
                .await
                .with_context(|| {
                    format!("Failed to create ACP session for agent_id={}", agent_id)
                })?;

            Ok((agent_acp, response))
        }
    })
    .await
}

impl App {
    /// Spawn an implementer agent (convenience wrapper for `spawn_agent_with_resolved_spec`).
    /// Returns (`session_id`, prompt) so the prompt can be logged.
    #[tracing::instrument(skip(self), fields(agent_type = "implementer"))]
    pub(crate) async fn spawn_implementer(&mut self, task: &str) -> Result<(String, String)> {
        let spec = ResolvedAgentSpec {
            role: "implementer".to_string(),
            task: task.to_string(),
            task_id: None,
            prompt: None,
            tools: None,
            model_complexity: None,
        };
        self.spawn_agent_with_resolved_spec(&spec).await
    }

    /// Spawn an agent based on a `ResolvedAgentSpec`.
    ///
    /// Determines the prompt and `removed_tools` based on the role:
    /// - Custom: requires prompt and tools from spec
    /// - Template roles (implementer, verifier, explorer): uses registry templates
    /// - Unknown roles: falls back to implementer behavior with warning
    ///
    /// Returns (`session_id`, prompt) so the prompt can be logged.
    #[tracing::instrument(skip(self, spec), fields(agent_type = %spec.role, task_id = ?spec.task_id, session_id))]
    pub(crate) async fn spawn_agent_with_resolved_spec(
        &mut self,
        spec: &ResolvedAgentSpec,
    ) -> Result<(String, String)> {
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
        // Note: context is empty string in sequential path (only used in concurrent spawning)
        let (prompt, removed_tools) = self.resolve_prompt_and_tools(spec, "")?;

        // Validate that all placeholders have been replaced
        validate_no_unreplaced_placeholders(&prompt, &spec.role)?;

        // Configure MCP server with agent type
        let mcp_server =
            build_mcp_server_config(&binary_path, &socket_path, &spec.role, &removed_tools, None);
        let mcp_servers = vec![mcp_server];

        // Resolve auto model to concrete model based on complexity
        let resolved_model = self
            .model_config
            .implementer_model
            .resolve_auto(spec.model_complexity);

        if self.model_config.implementer_model.is_auto() {
            tracing::info!(
                "🤖 Auto model resolved: {:?} → {} for {}",
                spec.model_complexity,
                resolved_model,
                spec.role
            );
        }

        // Create session with retry logic for transient MCP server startup errors
        let response = self
            .create_worker_session_with_retry(resolved_model.as_str(), mcp_servers, &cwd)
            .await?;

        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &response.session_id);
        tracing::debug!("🔨 {} prompt:\n{}", spec.role, prompt);

        self.acp_worker
            .session_prompt(&response.session_id, &prompt)
            .await?;

        Ok((response.session_id, prompt))
    }

    /// Create a worker session with retry logic.
    ///
    /// This handles transient MCP server startup errors by retrying the session
    /// creation with exponential backoff.
    async fn create_worker_session_with_retry(
        &mut self,
        model: &str,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<crate::acp::SessionNewResponse> {
        use super::retry::is_transient_error;

        let retry_config = RetryConfig::from_env();
        let mut attempt = 0;
        let mut delay = retry_config.initial_delay;

        loop {
            attempt += 1;

            match self.acp_worker.session_new(model, mcp_servers.clone(), cwd).await {
                Ok(response) => {
                    if attempt > 1 {
                        tracing::info!(
                            "🔄 Worker session_new succeeded on attempt {}/{}",
                            attempt,
                            retry_config.max_retries + 1
                        );
                    }
                    return Ok(response);
                }
                Err(e) => {
                    let is_transient = is_transient_error(&e);
                    let can_retry = attempt <= retry_config.max_retries && is_transient;

                    if can_retry {
                        tracing::warn!(
                            "⚠️ Worker session_new failed (attempt {}/{}): {}. Retrying in {:?}...",
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
                        let reason = if !is_transient {
                            "non-transient error"
                        } else {
                            "exhausted retries"
                        };
                        tracing::error!(
                            "❌ Worker session_new failed after {} attempt(s) ({}): {:#}",
                            attempt,
                            reason,
                            e
                        );
                        return Err(e).context(format!(
                            "Worker session_new failed after {} attempt(s)",
                            attempt
                        ));
                    }
                }
            }
        }
    }

    /// Spawn an agent with its own dedicated Unix socket.
    ///
    /// This is used for concurrent agent execution where each agent needs
    /// its own socket to receive tool call responses.
    ///
    /// Returns (`session_id`, prompt, `AgentSocketHandle`, `AcpClient`) so the caller can:
    /// - Log the prompt
    /// - Receive tool calls on the agent's dedicated socket
    /// - Clean up the socket when done
    /// - Keep the `AcpClient` alive until the agent completes
    #[tracing::instrument(skip(self, spec, context), fields(agent_type = %spec.role, task_id = ?spec.task_id, session_id))]
    pub(crate) async fn spawn_agent_with_own_socket(
        &mut self,
        spec: &ResolvedAgentSpec,
        context: &str,
    ) -> Result<(String, String, AgentSocketHandle, crate::acp::AcpClient)> {
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();

        // Get the path to the current binary
        let binary_path =
            std::env::current_exe().context("Failed to get current executable path")?;

        // Create a unique socket for this agent
        let agent_id = uuid::Uuid::new_v4().to_string();
        tracing::debug!("Creating socket for agent_id={}", &agent_id[..8]);
        let socket_handle = setup_agent_socket(&agent_id).await.with_context(|| {
            format!(
                "Failed to create agent socket for agent_id={}",
                &agent_id[..8]
            )
        })?;
        tracing::debug!(
            "Socket created successfully for agent_id={}",
            &agent_id[..8]
        );

        let socket_path_str = socket_handle.socket_path.to_string_lossy().to_string();

        // Determine prompt and removed_tools based on role
        let (prompt, removed_tools) = self.resolve_prompt_and_tools(spec, context)?;

        // Validate that all placeholders have been replaced
        validate_no_unreplaced_placeholders(&prompt, &spec.role)?;

        // Verify socket file still exists before configuring MCP server
        if !socket_handle.socket_path.exists() {
            anyhow::bail!(
                "Socket file {:?} does not exist before MCP server config (agent_id={}). \
                This may indicate a race condition or premature cleanup.",
                socket_handle.socket_path,
                &agent_id[..8]
            );
        }

        // Configure MCP server with unique suffix for concurrent agents
        let mcp_server = build_mcp_server_config(
            &binary_path,
            &socket_path_str,
            &spec.role,
            &removed_tools,
            Some(&agent_id[..8]),
        );
        let mcp_servers = vec![mcp_server.clone()];

        tracing::info!(
            "🔧 MCP server config for agent_id={}: name={}, socket={}, socket_exists=true",
            &agent_id[..8],
            mcp_server["name"],
            socket_path_str
        );

        // Resolve auto model to concrete model based on complexity
        let resolved_model = self
            .model_config
            .implementer_model
            .resolve_auto(spec.model_complexity);

        if self.model_config.implementer_model.is_auto() {
            tracing::info!(
                "🤖 Auto model resolved: {:?} → {} for {} (agent_id={})",
                spec.model_complexity,
                resolved_model,
                spec.role,
                &agent_id[..8]
            );
        }

        // Spawn ACP client with retry logic for transient MCP server startup errors
        let (mut agent_acp, response) = spawn_acp_with_retry(
            self.timeout_config.request_timeout,
            resolved_model.as_str(),
            mcp_servers,
            &cwd,
            &agent_id[..8],
        )
        .await?;

        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &response.session_id);
        tracing::debug!(
            "🔨 {} prompt (with own socket, agent_id={}):\n{}",
            spec.role,
            &agent_id[..8],
            prompt
        );

        agent_acp
            .session_prompt(&response.session_id, &prompt)
            .await
            .with_context(|| {
                format!(
                    "Failed to send prompt for agent_id={}, session_id={}",
                    &agent_id[..8],
                    response.session_id
                )
            })?;

        // Return the auggie instance so it stays alive while the agent runs
        Ok((
            response.session_id,
            prompt.clone(),
            socket_handle,
            agent_acp,
        ))
    }

    /// Resolves the prompt and removed tools for an agent based on its role.
    ///
    /// Handles three cases:
    /// - **Custom role**: Uses the custom prompt from spec with completion instructions appended.
    ///   Removed tools are computed from the optional tools whitelist.
    /// - **Template roles** (implementer, verifier, explorer): Uses registry templates with
    ///   placeholder substitution for `{task}`, `{user_goal}`, and `{context}`.
    /// - **Unknown roles**: Falls back to implementer behavior with a warning.
    ///
    /// # Arguments
    /// * `spec` - The resolved agent specification containing role, task, and optional prompt/tools
    /// * `context` - Context string to substitute for `{context}` placeholder (empty for sequential)
    ///
    /// # Returns
    /// A tuple of (prompt, removed tools) ready for use in agent spawning.
    fn resolve_prompt_and_tools(
        &self,
        spec: &ResolvedAgentSpec,
        context: &str,
    ) -> Result<(String, Vec<String>)> {
        use crate::agents::AgentRole;

        match AgentRole::from_str(&spec.role) {
            Some(AgentRole::Custom) => {
                // Custom agents require an explicit prompt
                let custom_prompt = spec
                    .prompt
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Custom agent requires 'prompt'"))?;

                let full_prompt = build_custom_prompt(custom_prompt);
                let removed = compute_removed_tools(spec.tools.as_ref());

                Ok((full_prompt, removed))
            }
            Some(role) => {
                // Template role: get from registry and substitute placeholders
                let template = self
                    .agent_registry
                    .get(&role)
                    .ok_or_else(|| anyhow::anyhow!("No template for role: {role:?}"))?;

                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal)
                    .replace("{context}", context);

                let removed = template
                    .removed_tools
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect();

                Ok((prompt, removed))
            }
            None => {
                // Unknown role - treat as implementer for backward compatibility
                tracing::warn!(
                    "Unknown agent role '{}', treating as implementer",
                    spec.role
                );
                let template = self.agent_registry.get(&AgentRole::Implementer).expect(
                    "implementer template must exist - it's a core role with a compile-time prompt",
                );
                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal)
                    .replace("{context}", context);

                Ok((prompt, vec![]))
            }
        }
    }

    /// Resolve an `AgentSpec` using the `TaskManager` for `task_id` lookups.
    ///
    /// The lookup supports both exact task IDs (e.g., "task001") and task names
    /// (e.g., "Setup database") as a fallback.
    ///
    /// If `task_id` is not provided but `task` description matches an existing
    /// task in the `TaskManager`, this method automatically infers the `task_id`
    /// to enable proper task status synchronization.
    async fn resolve_agent_spec(&self, spec: &AgentSpec) -> Result<ResolvedAgentSpec> {
        let task_manager = self.task_manager.read().await;
        let available_ids = task_manager.list_task_ids();

        // Try to infer task_id from task description if not explicitly provided
        let inferred_task_id = if spec.task_id.is_none() {
            if let Some(ref task_desc) = spec.task {
                // Try to find a matching task by name or description
                if let Some(found_id) = task_manager.find_by_name_or_description(task_desc) {
                    tracing::info!(
                        "📋 Inferred task_id '{}' from task description: {}",
                        found_id,
                        crate::app::types::truncate_for_log(task_desc, 60)
                    );
                    Some(found_id)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut resolved = spec
            .resolve(|tid| {
                // Use get_by_id_or_name for flexible lookup
                let result = task_manager
                    .get_by_id_or_name(tid)
                    .map(|t| t.description.clone());

                // Log helpful debug info if lookup fails
                if result.is_none() {
                    tracing::warn!(
                        "Task lookup failed for '{}'. Available task IDs: {:?}",
                        tid,
                        available_ids
                    );
                }
                result
            })
            .map_err(|e| anyhow::anyhow!(e))?;

        // Apply inferred task_id if we found one
        if resolved.task_id.is_none() {
            if let Some(inferred_id) = inferred_task_id {
                resolved.task_id = Some(inferred_id);
            }
        }

        Ok(resolved)
    }

    /// Spawn an agent asynchronously with its own dedicated socket and context.
    ///
    /// Each agent gets its own Unix socket and tool handler task, enabling
    /// concurrent execution of multiple agents without routing conflicts.
    ///
    /// # Arguments
    ///
    /// * `spec` - The agent specification describing the role and task
    /// * `context` - Additional context about neighboring tasks
    ///
    /// # Returns
    ///
    /// A tuple containing the session ID and a oneshot receiver for the result.
    #[tracing::instrument(
        skip(self, spec, context),
        fields(
            agent_type = spec.role.as_deref().unwrap_or("implementer"),
            task_id = ?spec.task_id,
            agent_name,
            session_id
        )
    )]
    pub(crate) async fn spawn_agent_async_with_context(
        &mut self,
        spec: &AgentSpec,
        context: &str,
    ) -> Result<(String, oneshot::Receiver<AgentResult>)> {
        // Resolve the spec (look up task_id if needed, apply defaults)
        let resolved = self.resolve_agent_spec(spec).await?;

        // Create implementer writer
        let mut impl_writer = self
            .current_scope
            .implementer_writer()
            .await
            .context("Failed to create implementer writer")?;

        let impl_name = impl_writer.agent_name().clone();
        let task = resolved.task.clone();
        let role = resolved.role.clone();
        let task_id = resolved.task_id.clone();

        // Record agent_name in the current span for tracing correlation
        tracing::Span::current().record("agent_name", &impl_name);
        tracing::info!(
            "🔨 [{}] Starting concurrent agent spawn (role={}, task_id={:?}): {}",
            impl_name,
            role,
            task_id,
            truncate_for_log(&task, 100)
        );

        // Record agent spawn metric
        crate::metrics::record_agent_spawned(&role);

        // If this agent is linked to a tracked task, mark it as InProgress
        if let Some(ref tid) = task_id {
            let mut tm = self.task_manager.write().await;
            tm.update_status(
                tid,
                &TaskStatus::InProgress {
                    agent_session: Some(impl_name.clone()),
                },
            );
            tracing::info!(
                "📋 Task {} marked as InProgress (agent: {})",
                tid,
                impl_name
            );
        }

        // Write a preliminary header so we have context in the log even if spawn fails
        if let Err(e) = impl_writer.write_header(&task).await {
            tracing::warn!("Failed to write preliminary header: {}", e);
        }

        // Spawn the agent with its own dedicated socket and auggie instance
        let spawn_result = self.spawn_agent_with_own_socket(&resolved, context).await;

        // If spawn failed, log the error to the implementer log file before propagating
        let (session_id, agent_prompt, socket_handle, mut agent_acp) = match spawn_result {
            Ok(result) => result,
            Err(e) => {
                // Mark task as failed if spawn failed
                if let Some(ref tid) = task_id {
                    let mut tm = self.task_manager.write().await;
                    tm.update_status(
                        tid,
                        &TaskStatus::Failed {
                            error: format!("Agent spawn failed: {e}"),
                        },
                    );
                }
                // Write spawn error to the log file for debugging
                let _ = impl_writer.write_spawn_error(&e).await;
                let _ = impl_writer.finalize(false).await;
                return Err(e);
            }
        };
        impl_writer.set_session_id(session_id.clone());
        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &session_id);
        if let Err(e) = impl_writer
            .write_header_with_prompt(&task, &agent_prompt)
            .await
        {
            tracing::warn!("Failed to write agent header: {}", e);
        }
        // Emit AgentStarted event for TUI
        impl_writer.emit_agent_started(&task);

        // Create the completion channel
        let (result_tx, result_rx) = oneshot::channel();

        // Register session with router for ACP messages
        let session_rx = self.register_session(&session_id).await;

        // Route agent ACP notifications to the session router
        spawn_notification_router(&mut agent_acp, Arc::clone(&self.session_router), &impl_name);

        // Spawn handler task for this agent
        let handler_params = AgentHandlerParams {
            agent_acp,
            socket_handle,
            session_rx,
            timeout_duration: self.timeout_config.session_timeout,
            role,
            task,
            impl_name,
            task_id,
            session_id: session_id.clone(),
            session_router: Arc::clone(&self.session_router),
            task_manager: Arc::clone(&self.task_manager),
            result_tx,
        };

        spawn_agent_handler_task(handler_params, impl_writer);

        Ok((session_id, result_rx))
    }

    /// Spawn multiple agents concurrently with the specified wait mode.
    ///
    /// Each agent gets its own dedicated Unix socket for tool call handling,
    /// enabling true concurrent execution without routing conflicts.
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
    #[tracing::instrument(skip(self, agents), fields(agent_count = agents.len(), wait_mode = ?wait))]
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

        for (index, spec) in agents.iter().enumerate() {
            let context = self.generate_task_context(&agents, index, spec).await;
            match self.spawn_agent_async_with_context(spec, &context).await {
                Ok((_session_id, rx)) => {
                    let (role, task) = extract_role_and_task(spec);
                    receivers.push((role, task, rx));
                }
                Err(e) => {
                    spawn_errors.push(create_spawn_error(spec, &e));
                }
            }
        }

        match wait {
            WaitMode::None => handle_fire_and_forget(receivers.len(), spawn_errors),
            WaitMode::Any => wait_for_any(receivers, spawn_errors).await,
            WaitMode::All => wait_for_all(receivers, spawn_errors).await,
        }
    }
}
