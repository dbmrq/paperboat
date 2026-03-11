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
use crate::acp::SessionMode;
use crate::backend::transport::{
    AgentTransport, AgentType, SessionConfig, SessionInfo, TransportKind,
};
use crate::backend::{Backend, TransportConfig};
use crate::mcp_server::{AgentSpec, ResolvedAgentSpec, WaitMode};
use crate::tasks::TaskStatus;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

/// Result of spawning an agent with its own socket.
/// Contains (`session_id`, model, prompt, `socket_handle`, transport).
pub type SpawnWithSocketResult = (
    String,
    String,
    String,
    AgentSocketHandle,
    Box<dyn AgentTransport>,
);

/// Result of spawning an agent via sequential mode.
pub struct AgentSession {
    /// The session ID for the agent session
    pub session_id: String,
    /// The model used for this session
    pub model: String,
    /// The prompt sent to the agent
    pub prompt: String,
    /// Socket handle for CLI transport (must be kept alive during the session).
    /// This field is intentionally not read directly - its presence keeps the socket listener
    /// alive until the `AgentSession` is dropped.
    #[allow(dead_code)]
    // Kept alive for RAII cleanup - socket listener drops when session drops
    socket_handle: Option<AgentSocketHandle>,
    /// Tool receiver extracted from the socket handle (for passing to wait functions)
    tool_rx: Option<super::types::ToolReceiver>,
}

impl AgentSession {
    /// Take the tool receiver for use in `wait_for_session_output`.
    /// Returns None if there's no CLI socket handle (e.g., ACP transport).
    pub const fn take_tool_rx(&mut self) -> Option<super::types::ToolReceiver> {
        self.tool_rx.take()
    }
}

/// Spawn and initialize an agent transport using the backend with model fallback.
///
/// This function handles the pattern of creating a transport via the backend,
/// initializing it, and creating a session. It tries each model in the chain,
/// with automatic retry for transient errors.
///
/// Returns (Box<dyn AgentTransport>, `SessionInfo`, `actual_model_used`) on success.
async fn spawn_transport_with_retry(
    backend: &dyn Backend,
    transport_kind: TransportKind,
    request_timeout: Duration,
    model_chain: &[String],
    mcp_servers: Vec<Value>,
    cwd: &str,
    agent_id: &str,
) -> Result<(Box<dyn AgentTransport>, SessionInfo, String)> {
    use super::retry::is_model_not_available_error;

    let retry_config = RetryConfig::from_env();

    // Track the last error for reporting if all models fail
    let mut last_error: Option<anyhow::Error> = None;

    for model in model_chain {
        let operation_name = format!(
            "spawn {} session for {agent_id} with model {model}",
            transport_kind.as_str()
        );

        // Create transport config for this model
        let config = TransportConfig::new(PathBuf::from(cwd))
            .with_model(model.clone())
            .with_request_timeout(request_timeout)
            .with_mcp_servers(mcp_servers.clone());

        // Try to create and initialize the transport
        let result = retry_async(&retry_config, &operation_name, || {
            let config = config.clone();
            let model = model.clone();
            let agent_id = agent_id.to_string();

            async move {
                // Create a fresh transport instance via the backend
                let mut transport = backend
                    .create_transport(transport_kind, AgentType::Implementer, config.clone())
                    .await
                    .with_context(|| {
                        format!("Failed to create transport for agent_id={agent_id}")
                    })?;

                // Initialize the transport connection
                transport.initialize().await.with_context(|| {
                    format!("Failed to initialize transport for agent_id={agent_id}")
                })?;

                // Create the session with MCP servers (Agent mode for full tool access)
                let session_config =
                    SessionConfig::new(model, config.workspace.to_string_lossy().to_string())
                        .with_mcp_servers(config.mcp_servers)
                        .with_mode(SessionMode::Agent);

                let session_info = transport
                    .create_session(session_config)
                    .await
                    .with_context(|| format!("Failed to create session for agent_id={agent_id}"))?;

                Ok((transport, session_info))
            }
        })
        .await;

        match result {
            Ok((transport, session_info)) => return Ok((transport, session_info, model.clone())),
            Err(e) => {
                if is_model_not_available_error(&e) {
                    tracing::debug!(
                        "Model '{}' not available for agent {}, trying next in chain...",
                        model,
                        agent_id
                    );
                    last_error = Some(e);
                    continue; // Try next model
                }
                // Non-model error - propagate immediately
                return Err(e);
            }
        }
    }

    // All models in the chain failed
    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("No models in fallback chain"))
        .context(format!(
            "All models in fallback chain failed for agent {agent_id}: {model_chain:?}"
        )))
}

impl App {
    /// Spawn an implementer agent (convenience wrapper for `spawn_agent_with_resolved_spec`).
    /// Returns an `AgentSession` containing session info and socket handle.
    #[tracing::instrument(skip(self), fields(agent_type = "implementer"))]
    pub(crate) async fn spawn_implementer(&mut self, task: &str) -> Result<AgentSession> {
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
    /// Returns an `AgentSession` containing session info and socket handle.
    /// The socket handle must be kept alive until the agent session completes.
    #[tracing::instrument(skip(self, spec), fields(agent_type = %spec.role, task_id = ?spec.task_id, session_id))]
    pub(crate) async fn spawn_agent_with_resolved_spec(
        &mut self,
        spec: &ResolvedAgentSpec,
    ) -> Result<AgentSession> {
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();

        // Get the path to the current binary
        let binary_path =
            std::env::current_exe().context("Failed to get current executable path")?;

        // For CLI transport, create a unique socket to prevent MCP server caching.
        // This ensures each agent session gets its own MCP server process with the
        // correct agent type and tools.
        let (socket_address_str, cli_socket_handle) =
            if self.acp_worker.kind() == TransportKind::Cli {
                let agent_id = format!("cli-{}", &uuid::Uuid::new_v4().to_string()[..8]);
                let socket_handle = setup_agent_socket(&agent_id).await.with_context(|| {
                    format!("Failed to create unique socket for CLI agent: {agent_id}")
                })?;
                let addr = socket_handle.socket_address.as_str().to_string();
                tracing::debug!(
                    "🔌 Created unique socket for CLI worker: {} -> {}",
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

        // Determine prompt and removed_tools based on role
        // Note: context is empty string in sequential path (only used in concurrent spawning)
        let (prompt, removed_tools) = self.resolve_prompt_and_tools(spec, "")?;

        // Validate that all placeholders have been replaced
        validate_no_unreplaced_placeholders(&prompt, &spec.role)?;

        // Configure MCP server with agent type
        let mcp_server = build_mcp_server_config(
            &binary_path,
            &socket_address_str,
            &spec.role,
            &removed_tools,
            None,
        );
        let mcp_servers = vec![mcp_server];

        // Resolve model: fallback chain -> tier -> auto resolution -> backend model chain
        let model_chain = self.resolve_implementer_model(spec.model_complexity)?;

        // Create session with retry logic and model fallback
        let (response, actual_model) = self
            .create_worker_session_with_retry(&model_chain, mcp_servers, &cwd)
            .await?;

        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &response.session_id);
        tracing::debug!("🔨 {} prompt:\n{}", spec.role, prompt);

        self.acp_worker
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

        Ok(AgentSession {
            session_id: response.session_id,
            model: actual_model,
            prompt,
            socket_handle,
            tool_rx,
        })
    }

    /// Create a worker session with model fallback chain and retry logic.
    ///
    /// This tries each model in the chain, and for each model:
    /// - Handles transient MCP server startup errors with exponential backoff
    /// - Falls back to the next model if "model not available" error occurs
    ///
    /// Returns (`SessionInfo`, `actual_model_used`) on success.
    async fn create_worker_session_with_retry(
        &mut self,
        model_chain: &[String],
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<(SessionInfo, String)> {
        use super::retry::{is_model_not_available_error, is_transient_error};

        let retry_config = RetryConfig::from_env();

        // Track the last error for reporting if all models fail
        let mut last_error: Option<anyhow::Error> = None;

        for model in model_chain {
            let mut attempt = 0;
            let mut delay = retry_config.initial_delay;

            loop {
                attempt += 1;

                // Create session config with Agent mode for full tool access
                let config = SessionConfig::new(model, cwd)
                    .with_mcp_servers(mcp_servers.clone())
                    .with_mode(SessionMode::Agent);

                match self.acp_worker.create_session(config).await {
                    Ok(response) => {
                        if attempt > 1 {
                            tracing::info!(
                                "🔄 Worker create_session succeeded on attempt {}/{} with model {}",
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
                                "Model '{}' not available, trying next in chain...",
                                model
                            );
                            last_error = Some(e);
                            break; // Move to next model in chain
                        }

                        let is_transient = is_transient_error(&e);
                        let can_retry = attempt <= retry_config.max_retries && is_transient;

                        if can_retry {
                            tracing::warn!(
                                "⚠️ Worker create_session failed (attempt {}/{}, model {}): {}. Retrying in {:?}...",
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
                                "❌ Worker create_session failed after {attempt} attempt(s) ({reason}): {e:#}",
                            );
                            return Err(e).context(format!(
                                "Worker create_session failed after {attempt} attempt(s)"
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
                "All models in fallback chain failed: {model_chain:?}"
            )))
    }

    /// Spawn an agent with its own dedicated IPC endpoint.
    ///
    /// This is used for concurrent agent execution where each agent needs
    /// its own IPC endpoint to receive tool call responses.
    ///
    /// Returns (`session_id`, `model`, prompt, `AgentSocketHandle`, `Box<dyn AgentTransport>`) so the caller can:
    /// - Log the model and prompt
    /// - Receive tool calls on the agent's dedicated socket
    /// - Clean up the socket when done
    /// - Keep the transport alive until the agent completes
    #[tracing::instrument(skip(self, spec, context), fields(agent_type = %spec.role, task_id = ?spec.task_id, session_id))]
    pub(crate) async fn spawn_agent_with_own_socket(
        &mut self,
        spec: &ResolvedAgentSpec,
        context: &str,
    ) -> Result<SpawnWithSocketResult> {
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

        let socket_address_str = socket_handle.socket_address.as_str().to_string();

        // Determine prompt and removed_tools based on role
        let (prompt, removed_tools) = self.resolve_prompt_and_tools(spec, context)?;

        // Validate that all placeholders have been replaced
        validate_no_unreplaced_placeholders(&prompt, &spec.role)?;

        // Verify socket endpoint exists before configuring MCP server (on Unix)
        if !socket_handle.socket_address.exists() {
            anyhow::bail!(
                "Socket {} does not exist before MCP server config (agent_id={}). \
                This may indicate a race condition or premature cleanup.",
                socket_handle.socket_address,
                &agent_id[..8]
            );
        }

        // Configure MCP server with unique suffix for concurrent agents
        let mcp_server = build_mcp_server_config(
            &binary_path,
            &socket_address_str,
            &spec.role,
            &removed_tools,
            Some(&agent_id[..8]),
        );
        let mcp_servers = vec![mcp_server.clone()];

        tracing::info!(
            "🔧 MCP server config for agent_id={}: name={}, socket={}, socket_exists=true",
            &agent_id[..8],
            mcp_server["name"],
            socket_address_str
        );

        // Resolve model: fallback chain -> tier -> auto resolution -> backend model chain
        let model_chain = self.resolve_implementer_model(spec.model_complexity)?;

        // Spawn transport using the configured backend (respects cursor:cli vs auggie:acp)
        let (mut agent_transport, session_info, actual_model) = spawn_transport_with_retry(
            self.backend.as_ref(),
            self.transport_kind,
            self.timeout_config.request_timeout,
            &model_chain,
            mcp_servers,
            &cwd,
            &agent_id[..8],
        )
        .await?;

        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &session_info.session_id);
        tracing::debug!(
            "🔨 {} prompt (with own socket, agent_id={}):\n{}",
            spec.role,
            &agent_id[..8],
            prompt
        );

        agent_transport
            .send_prompt(&session_info.session_id, &prompt)
            .await
            .with_context(|| {
                format!(
                    "Failed to send prompt for agent_id={}, session_id={}",
                    &agent_id[..8],
                    session_info.session_id
                )
            })?;

        // Return the transport so it stays alive while the agent runs
        Ok((
            session_info.session_id,
            actual_model,
            prompt.clone(),
            socket_handle,
            agent_transport,
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

    /// Resolve an [`AgentSpec`] using the `TaskManager` for `task_id` lookups.
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
    /// Each agent gets its own IPC endpoint and tool handler task, enabling
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

        // Spawn the agent with its own dedicated socket and transport
        let spawn_result = self.spawn_agent_with_own_socket(&resolved, context).await;

        // If spawn failed, log the error to the implementer log file before propagating
        let (session_id, model, agent_prompt, socket_handle, mut agent_transport) =
            match spawn_result {
                Ok(result) => result,
                Err(e) => {
                    // Mark task as failed if spawn failed
                    if let Some(ref tid) = task_id {
                        let mut tm = self.task_manager.write().await;
                        tm.update_status(
                            tid,
                            &TaskStatus::Failed {
                                // Use {:?} to include the full error chain
                                error: format!("Agent spawn failed: {e:?}"),
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
        impl_writer.set_model(model);
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

        // Route agent notifications to the session router
        spawn_notification_router(
            agent_transport.as_mut(),
            Arc::clone(&self.session_router),
            &impl_name,
        );

        // Spawn handler task for this agent
        let handler_params = AgentHandlerParams {
            agent_transport,
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
    /// Each agent gets its own dedicated IPC endpoint for tool call handling,
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

    /// Resolve the implementer model configuration to a fallback chain of model strings.
    ///
    /// This method:
    /// 1. Resolves the fallback chain to a tier using `available_tiers`
    /// 2. If the tier is Auto, resolves it based on complexity
    /// 3. Uses the backend to convert the tier to a list of model IDs to try
    ///
    /// Returns a vector of model strings to try in order (first is preferred).
    fn resolve_implementer_model(
        &self,
        complexity: Option<crate::mcp_server::ModelComplexity>,
    ) -> Result<Vec<String>> {
        // Step 1: Resolve fallback chain to a tier
        let tier = self
            .model_config
            .implementer_model
            .resolve(&self.model_config.available_tiers)?;

        // Step 2: If Auto, resolve based on complexity
        let resolved_tier = if tier.is_auto() {
            let concrete = tier.resolve_auto(complexity);
            tracing::info!(
                "🤖 Auto model resolved: {:?} → {} for implementer",
                complexity,
                concrete
            );
            concrete
        } else {
            tier
        };

        // Step 3: Convert tier to backend-specific model fallback chain with effort level
        self.backend
            .resolve_tier(resolved_tier, Some(self.model_config.implementer_effort))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpClientTrait;
    use crate::backend::transport::SessionUpdate;
    use crate::backend::{AgentCacheType, Backend};
    use crate::logging::RunLogManager;
    use crate::mcp_server::AgentSpec;
    use crate::models::{EffortLevel, ModelConfig, ModelTier};
    use crate::tasks::TaskStatus;
    use crate::testing::{MockAcpClient, MockTransport};
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use serial_test::serial;
    use std::collections::{HashSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedEvent {
        CreateTransport { model: String },
        Initialize { model: String },
        CreateSession { model: String },
    }

    #[derive(Clone)]
    struct TransportOutcome {
        initialize_error: Option<&'static str>,
        session_error: Option<&'static str>,
        session_id: &'static str,
    }

    #[derive(Clone)]
    enum AttemptOutcome {
        CreateTransportError(&'static str),
        Transport(TransportOutcome),
    }

    struct RecordingTransport {
        model: String,
        outcome: TransportOutcome,
        events: Arc<Mutex<Vec<RecordedEvent>>>,
    }

    #[async_trait]
    impl AgentTransport for RecordingTransport {
        async fn initialize(&mut self) -> Result<()> {
            self.events
                .lock()
                .expect("record initialize event")
                .push(RecordedEvent::Initialize {
                    model: self.model.clone(),
                });

            if let Some(error) = self.outcome.initialize_error {
                return Err(anyhow!(error));
            }

            Ok(())
        }

        async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo> {
            self.events
                .lock()
                .expect("record create_session event")
                .push(RecordedEvent::CreateSession {
                    model: config.model,
                });

            if let Some(error) = self.outcome.session_error {
                return Err(anyhow!(error));
            }

            Ok(SessionInfo::new(self.outcome.session_id))
        }

        async fn send_prompt(&mut self, _session_id: &str, _prompt: &str) -> Result<()> {
            Ok(())
        }

        fn take_notifications(&mut self) -> Option<tokio::sync::mpsc::Receiver<SessionUpdate>> {
            None
        }

        async fn respond_to_tool(
            &mut self,
            _session_id: &str,
            _tool_use_id: &str,
            _result: crate::backend::transport::ToolResult,
        ) -> Result<()> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct RecordingBackend {
        attempts: Arc<Mutex<VecDeque<AttemptOutcome>>>,
        events: Arc<Mutex<Vec<RecordedEvent>>>,
        fallback_chain: Vec<String>,
    }

    impl RecordingBackend {
        fn new(attempts: Vec<AttemptOutcome>) -> Self {
            Self {
                attempts: Arc::new(Mutex::new(attempts.into())),
                events: Arc::new(Mutex::new(Vec::new())),
                fallback_chain: vec!["sonnet".to_string()],
            }
        }

        fn with_fallback_chain(mut self, fallback_chain: Vec<String>) -> Self {
            self.fallback_chain = fallback_chain;
            self
        }

        fn events(&self) -> Vec<RecordedEvent> {
            self.events.lock().expect("read recorded events").clone()
        }
    }

    #[async_trait]
    impl Backend for RecordingBackend {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn check_auth(&self) -> Result<()> {
            Ok(())
        }

        async fn available_tiers(&self) -> Result<HashSet<ModelTier>> {
            Ok(std::iter::once(ModelTier::Sonnet).collect())
        }

        fn resolve_tier(
            &self,
            _tier: ModelTier,
            _effort: Option<EffortLevel>,
        ) -> Result<Vec<String>> {
            Ok(self.fallback_chain.clone())
        }

        async fn setup_mcp(&self, _socket_path: &str) -> Result<()> {
            Ok(())
        }

        fn cleanup_mcp(&self) -> Result<()> {
            Ok(())
        }

        async fn create_client(
            &self,
            _agent_type: AgentCacheType,
            _cache_dir: Option<&str>,
            _request_timeout: Duration,
        ) -> Result<Box<dyn AcpClientTrait + Send>> {
            Ok(Box::new(MockAcpClient::empty()))
        }

        fn setup_agent_cache(
            &self,
            _agent_type: AgentCacheType,
            _removed_tools: &[&str],
        ) -> Result<PathBuf> {
            Ok(std::env::temp_dir())
        }

        async fn create_transport(
            &self,
            _kind: TransportKind,
            _agent_type: AgentType,
            config: TransportConfig,
        ) -> Result<Box<dyn AgentTransport>> {
            let model = config
                .model
                .expect("transport model should be set for tests");

            self.events
                .lock()
                .expect("record create_transport event")
                .push(RecordedEvent::CreateTransport {
                    model: model.clone(),
                });

            let outcome = self
                .attempts
                .lock()
                .expect("pop scripted backend outcome")
                .pop_front()
                .expect("missing scripted backend attempt");

            match outcome {
                AttemptOutcome::CreateTransportError(error) => Err(anyhow!(error)),
                AttemptOutcome::Transport(outcome) => Ok(Box::new(RecordingTransport {
                    model,
                    outcome,
                    events: Arc::clone(&self.events),
                })),
            }
        }

        fn login_hint(&self) -> &'static str {
            "recording login"
        }
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn test_model_config() -> ModelConfig {
        ModelConfig::new(
            [ModelTier::Sonnet, ModelTier::Opus, ModelTier::Haiku]
                .into_iter()
                .collect(),
        )
    }

    #[tokio::test]
    #[serial]
    async fn spawn_transport_retries_transient_session_creation_failure_in_order() {
        let _retry_guard = EnvGuard::set("PAPERBOAT_SPAWN_RETRIES", "1");
        let _delay_guard = EnvGuard::set("PAPERBOAT_SPAWN_RETRY_DELAY_MS", "0");

        let backend = RecordingBackend::new(vec![
            AttemptOutcome::Transport(TransportOutcome {
                initialize_error: None,
                session_error: Some("mcp server startup timeout"),
                session_id: "unused-first",
            }),
            AttemptOutcome::Transport(TransportOutcome {
                initialize_error: None,
                session_error: None,
                session_id: "worker-session-002",
            }),
        ]);

        let (_transport, session, actual_model) = spawn_transport_with_retry(
            &backend,
            TransportKind::Acp,
            Duration::from_secs(1),
            &[String::from("sonnet")],
            vec![],
            "/tmp",
            "agent-1234",
        )
        .await
        .expect("transient create_session failure should retry");

        assert_eq!(actual_model, "sonnet");
        assert_eq!(session.session_id, "worker-session-002");
        assert_eq!(
            backend.events(),
            vec![
                RecordedEvent::CreateTransport {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::Initialize {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::CreateSession {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::CreateTransport {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::Initialize {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::CreateSession {
                    model: "sonnet".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    #[serial]
    async fn spawn_transport_falls_back_to_next_model_without_retrying_same_model() {
        let _retry_guard = EnvGuard::set("PAPERBOAT_SPAWN_RETRIES", "0");
        let _delay_guard = EnvGuard::set("PAPERBOAT_SPAWN_RETRY_DELAY_MS", "0");

        let backend = RecordingBackend::new(vec![
            AttemptOutcome::Transport(TransportOutcome {
                initialize_error: None,
                session_error: Some("Cannot use this model"),
                session_id: "unused-sonnet",
            }),
            AttemptOutcome::Transport(TransportOutcome {
                initialize_error: None,
                session_error: None,
                session_id: "worker-session-opus",
            }),
        ]);

        let (_transport, session, actual_model) = spawn_transport_with_retry(
            &backend,
            TransportKind::Cli,
            Duration::from_secs(1),
            &[String::from("sonnet"), String::from("opus")],
            vec![],
            "/tmp",
            "agent-5678",
        )
        .await
        .expect("model unavailability should fall back to the next model");

        assert_eq!(actual_model, "opus");
        assert_eq!(session.session_id, "worker-session-opus");
        assert_eq!(
            backend.events(),
            vec![
                RecordedEvent::CreateTransport {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::Initialize {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::CreateSession {
                    model: "sonnet".to_string(),
                },
                RecordedEvent::CreateTransport {
                    model: "opus".to_string(),
                },
                RecordedEvent::Initialize {
                    model: "opus".to_string(),
                },
                RecordedEvent::CreateSession {
                    model: "opus".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn spawn_agent_async_marks_task_failed_and_logs_startup_error() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let run_dir = temp_dir.path().join("logs");
        let log_manager =
            Arc::new(RunLogManager::with_run_dir(run_dir.clone()).expect("create test run dir"));

        let backend = RecordingBackend::new(vec![AttemptOutcome::CreateTransportError(
            "backend session bootstrap exploded",
        )])
        .with_fallback_chain(vec!["sonnet".to_string()]);

        let mut app = App::with_mock_transports(
            Box::new(backend),
            Box::new(MockTransport::empty()),
            Box::new(MockTransport::empty()),
            Box::new(MockTransport::empty()),
            test_model_config(),
            Arc::clone(&log_manager),
        );

        let task_id = {
            let mut tm = app.task_manager.write().await;
            tm.create(
                "Tracked task",
                "Exercise startup failure propagation",
                vec![],
            )
        };

        let spec = AgentSpec {
            role: Some("implementer".to_string()),
            task: None,
            task_id: Some(task_id.clone()),
            prompt: None,
            tools: None,
            model_complexity: None,
        };

        let error = app
            .spawn_agent_async_with_context(&spec, "previous attempt failed")
            .await
            .expect_err("startup failure should propagate");

        // Check if the error chain contains the expected message
        let error_chain = format!("{error:?}");
        assert!(
            error_chain.contains("backend session bootstrap exploded"),
            "Error chain should contain bootstrap failure: {error_chain}"
        );

        let task = app
            .task_manager
            .read()
            .await
            .get(&task_id)
            .cloned()
            .expect("tracked task should still exist");

        match task.status {
            TaskStatus::Failed { error } => {
                assert!(error.contains("Agent spawn failed"));
                assert!(error.contains("backend session bootstrap exploded"));
            }
            other => panic!("expected failed task status, got {other:?}"),
        }

        let implementer_log = run_dir.join("implementer-001.log");
        let log_contents = std::fs::read_to_string(&implementer_log)
            .expect("startup failure should still finalize an implementer log");
        assert!(log_contents.contains("SPAWN FAILED"));
        assert!(log_contents.contains("backend session bootstrap exploded"));
        assert!(log_contents.contains("FAILURE"));
    }
}
