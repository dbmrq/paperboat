//! Generic agent spawning.
//!
//! This module provides a generalized interface for spawning worker agents
//! (implementers, verifiers, etc.) with support for concurrent execution.

use super::agent_handler::{run_agent_handler, update_task_completion};
use super::socket::{setup_agent_socket, AgentSocketHandle};
use super::types::{format_duration_human, truncate_for_log};
use super::App;
use crate::acp::AcpClientTrait;
use crate::agents::AgentRole;
use crate::mcp_server::{AgentSpec, ResolvedAgentSpec, ToolResponse, WaitMode};
use crate::tasks::TaskStatus;
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

/// Known placeholders that should be replaced in prompt templates.
const KNOWN_PLACEHOLDERS: &[&str] = &["{task}", "{user_goal}", "{context}"];

/// Validates that no known placeholders remain unreplaced in the prompt.
///
/// Returns an error if any placeholder from `KNOWN_PLACEHOLDERS` is found.
fn validate_no_unreplaced_placeholders(prompt: &str, role: &str) -> Result<()> {
    let unreplaced: Vec<&str> = KNOWN_PLACEHOLDERS
        .iter()
        .filter(|p| prompt.contains(*p))
        .copied()
        .collect();

    if !unreplaced.is_empty() {
        anyhow::bail!(
            "Prompt for role '{}' has unreplaced placeholders: {}",
            role,
            unreplaced.join(", ")
        );
    }

    Ok(())
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
        let depth = self.current_scope.depth();

        // Create implementer writer (this assigns the implementer number)
        let mut impl_writer = self
            .current_scope
            .implementer_writer()
            .await
            .context("Failed to create implementer writer")?;

        // Get the implementer name for logging
        let impl_name = impl_writer.agent_name();
        tracing::info!(
            "[L{}] 🔨 [{}] Starting: {}",
            depth,
            impl_name,
            truncate_for_log(task, 100)
        );

        // Spawn implementer
        let (impl_session, impl_prompt) = match self.spawn_implementer(task).await {
            Ok(result) => result,
            Err(e) => {
                // Write error to implementer log so it's not empty
                tracing::error!(
                    "[L{}] ❌ [{}] Failed to spawn implementer: {:#}",
                    depth,
                    impl_name,
                    e
                );
                if let Err(write_err) = impl_writer.write_spawn_error(&e).await {
                    tracing::warn!("Failed to write spawn error to implementer log: {}", write_err);
                }
                if let Err(finalize_err) = impl_writer.finalize(false).await {
                    tracing::warn!("Failed to finalize implementer log after spawn error: {}", finalize_err);
                }
                return Err(e);
            }
        };
        impl_writer.set_session_id(impl_session.clone());
        if let Err(e) = impl_writer
            .write_header_with_prompt(task, &impl_prompt)
            .await
        {
            tracing::warn!("Failed to write implementer header: {}", e);
        }
        // Emit AgentStarted event for TUI
        impl_writer.emit_agent_started(task);

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

    /// Spawn an implementer agent (convenience wrapper for `spawn_agent_with_resolved_spec`).
    /// Returns (`session_id`, prompt) so the prompt can be logged.
    pub(crate) async fn spawn_implementer(&mut self, task: &str) -> Result<(String, String)> {
        let spec = ResolvedAgentSpec {
            role: "implementer".to_string(),
            task: task.to_string(),
            task_id: None,
            prompt: None,
            tools: None,
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
        let (prompt, removed_tools) = match AgentRole::from_str(&spec.role) {
            Some(AgentRole::Custom) => {
                // Custom: require prompt and tools from spec
                let custom_prompt = spec
                    .prompt
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Custom agent requires 'prompt'"))?
                    .clone();

                // Append completion instructions so custom agents know how to signal completion
                let full_prompt = format!(
                    "{custom_prompt}\n\n## When Done\n\n\
                    Call `complete` with:\n\
                    - **success**: Whether you accomplished your task (true/false)\n\
                    - **message**: Brief summary of what you found or did\n\
                    - **notes** (optional): Context for future agents or the orchestrator\n\
                    - **add_tasks** (optional): Any work you discovered that should be done",
                );

                // If tools whitelist is provided, derive removed_tools from it
                // Otherwise, allow all default auggie tools (like implementer)
                let removed = if let Some(allowed_tools) = &spec.tools {
                    let all_tools = vec![
                        "str-replace-editor",
                        "save-file",
                        "remove-files",
                        "launch-process",
                        "kill-process",
                        "read-process",
                        "write-process",
                        "list-processes",
                        "web-search",
                        "web-fetch",
                    ];
                    all_tools
                        .iter()
                        .filter(|t| !allowed_tools.contains(&(*t).to_string()))
                        .map(std::string::ToString::to_string)
                        .collect()
                } else {
                    // No tools specified = all default tools enabled (same as implementer)
                    vec![]
                };

                (full_prompt, removed)
            }
            Some(role) => {
                // Template role: get from registry
                let template = self
                    .agent_registry
                    .get(&role)
                    .ok_or_else(|| anyhow::anyhow!("No template for role: {role:?}"))?;

                // Note: {context} is replaced with empty string in sequential path
                // (context is only used in concurrent spawning with spawn_agent_with_own_socket)
                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal)
                    .replace("{context}", "");
                let removed = template
                    .removed_tools
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect();

                (prompt, removed)
            }
            None => {
                // Unknown role - treat as implementer for backward compatibility
                tracing::warn!(
                    "Unknown agent role '{}', treating as implementer",
                    spec.role
                );
                let template = self.agent_registry.get(&AgentRole::Implementer).unwrap();
                // Note: {context} is replaced with empty string in sequential path
                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal)
                    .replace("{context}", "");
                (prompt, vec![])
            }
        };

        // Validate that all placeholders have been replaced
        validate_no_unreplaced_placeholders(&prompt, &spec.role)?;

        // Build environment variables for the MCP server
        let mut env_vars = vec![json!({
            "name": "PAPERBOAT_AGENT_TYPE",
            "value": spec.role.clone()
        })];

        // Add removed tools to environment if any
        if !removed_tools.is_empty() {
            env_vars.push(
                json!({"name": "PAPERBOAT_REMOVED_TOOLS", "value": removed_tools.join(",")}),
            );
        }

        // Configure MCP server with agent type
        // Use unique name based on role to prevent MCP server caching/reuse between agent types
        // Pass --socket directly to avoid env var caching issues across auggie sessions
        let mcp_servers = vec![json!({
            "name": format!("paperboat-{}", spec.role),
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server", "--socket", &socket_path],
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
        let (prompt, removed_tools) = match AgentRole::from_str(&spec.role) {
            Some(AgentRole::Custom) => {
                // Custom: require prompt, tools whitelist is optional
                let custom_prompt = spec
                    .prompt
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Custom agent requires 'prompt'"))?
                    .clone();

                // Append completion instructions so custom agents know how to signal completion
                let full_prompt = format!(
                    "{custom_prompt}\n\n## When Done\n\n\
                    Call `complete` with:\n\
                    - **success**: Whether you accomplished your task (true/false)\n\
                    - **message**: Brief summary of what you found or did\n\
                    - **notes** (optional): Context for future agents or the orchestrator\n\
                    - **add_tasks** (optional): Any work you discovered that should be done",
                );

                // If tools whitelist is provided, derive removed_tools from it
                // Otherwise, allow all default auggie tools (like implementer)
                let removed = if let Some(allowed_tools) = &spec.tools {
                    let all_tools = vec![
                        "str-replace-editor",
                        "save-file",
                        "remove-files",
                        "launch-process",
                        "kill-process",
                        "read-process",
                        "write-process",
                        "list-processes",
                        "web-search",
                        "web-fetch",
                    ];
                    all_tools
                        .iter()
                        .filter(|t| !allowed_tools.contains(&(*t).to_string()))
                        .map(std::string::ToString::to_string)
                        .collect()
                } else {
                    // No tools specified = all default tools enabled (same as implementer)
                    vec![]
                };

                (full_prompt, removed)
            }
            Some(role) => {
                // Template role: get from registry
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

                (prompt, removed)
            }
            None => {
                // Unknown role - treat as implementer for backward compatibility
                tracing::warn!(
                    "Unknown agent role '{}', treating as implementer",
                    spec.role
                );
                let template = self.agent_registry.get(&AgentRole::Implementer).unwrap();
                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal)
                    .replace("{context}", context);
                (prompt, vec![])
            }
        };

        // Validate that all placeholders have been replaced
        validate_no_unreplaced_placeholders(&prompt, &spec.role)?;

        // Build environment variables for the MCP server
        let mut env_vars =
            vec![json!({"name": "PAPERBOAT_AGENT_TYPE", "value": spec.role.clone()})];

        // Add removed tools to environment if any
        if !removed_tools.is_empty() {
            env_vars.push(
                json!({"name": "PAPERBOAT_REMOVED_TOOLS", "value": removed_tools.join(",")}),
            );
        }

        // Configure MCP server with agent type
        // Pass socket path as an ARG (not just env var) to ensure auggie spawns unique processes
        // Auggie may cache MCP servers by command+args, so unique args = unique process
        let mcp_server_name = format!("paperboat-{}-{}", spec.role, &agent_id[..8]);
        let mcp_servers = vec![json!({
            "name": mcp_server_name.clone(),
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server", "--socket", &socket_path_str],
            "env": env_vars
        })];

        tracing::info!(
            "🔧 MCP server config for agent_id={}: name={}, socket={}",
            &agent_id[..8],
            mcp_server_name,
            socket_path_str
        );

        // Create a fresh auggie instance for this agent to avoid MCP server caching issues.
        // Each implementer gets its own auggie process with isolated MCP server state.
        let mut agent_acp = crate::acp::AcpClient::spawn_with_timeout(
            None, // Use default cache
            self.timeout_config.request_timeout,
        )
        .await
        .with_context(|| format!("Failed to spawn auggie for agent_id={}", &agent_id[..8]))?;
        agent_acp.initialize().await.with_context(|| {
            format!(
                "Failed to initialize auggie for agent_id={}",
                &agent_id[..8]
            )
        })?;

        let response = agent_acp
            .session_new(
                self.model_config.implementer_model.as_str(),
                mcp_servers,
                &cwd,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to create ACP session for agent_id={}",
                    &agent_id[..8]
                )
            })?;

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
        Ok((response.session_id, prompt, socket_handle, agent_acp))
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

    /// Resolve an `AgentSpec` using the `TaskManager` for `task_id` lookups.
    ///
    /// The lookup supports both exact task IDs (e.g., "task001") and task names
    /// (e.g., "Setup database") as a fallback.
    async fn resolve_agent_spec(&self, spec: &AgentSpec) -> Result<ResolvedAgentSpec> {
        let task_manager = self.task_manager.read().await;
        let available_ids = task_manager.list_task_ids();

        spec.resolve(|tid| {
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
        .map_err(|e| anyhow::anyhow!(e))
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

        tracing::info!(
            "🔨 [{}] Starting concurrent agent spawn (role={}, task_id={:?}): {}",
            impl_name,
            role,
            task_id,
            truncate_for_log(&task, 100)
        );

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

        // Take the notification channel from this agent's ACP client and route it to the session router.
        // Each concurrent agent has its own ACP client, so we need to route its messages
        // to the shared session_router for the handler to receive them.
        let agent_notification_rx = agent_acp.take_notification_rx();
        if let Some(notification_rx) = agent_notification_rx {
            let session_router_for_agent = Arc::clone(&self.session_router);
            let agent_id_for_log = impl_name.clone();
            tokio::spawn(async move {
                let mut rx = notification_rx;
                while let Some(msg) = rx.recv().await {
                    let routed = {
                        let router = session_router_for_agent.read().await;
                        router.route(msg.clone())
                    };
                    if !routed {
                        // This can happen if the session finishes before all messages are processed
                        tracing::trace!(
                            "[{}] Agent message not routed (session may have ended)",
                            agent_id_for_log
                        );
                    }
                }
                tracing::debug!("[{}] Agent notification channel closed", agent_id_for_log);
            });
        } else {
            tracing::warn!(
                "[{}] Could not take notification channel from agent ACP client",
                impl_name
            );
        }

        // Clone what we need for the spawned task
        let session_router = Arc::clone(&self.session_router);
        let task_manager = Arc::clone(&self.task_manager);
        let session_id_clone = session_id.clone();
        let timeout_duration = self.timeout_config.session_timeout;

        // Spawn a handler task that:
        // 1. Receives tool calls on the agent's dedicated socket
        // 2. Handles the Complete tool call to detect agent completion
        // 3. Cleans up the socket when done
        // 4. Sends AgentResult via the oneshot channel
        // 5. Updates task status if task_id is provided
        // 6. Stores notes and creates suggested tasks
        tokio::spawn(async move {
            // Keep agent_acp alive for the duration of the agent's execution.
            // When this task ends, the AcpClient is dropped and auggie shuts down.
            let _agent_acp = agent_acp;

            let start_time = std::time::Instant::now();

            // Wait for the agent to complete, handling tool calls
            let completion = run_agent_handler(
                socket_handle,
                session_rx,
                timeout_duration,
                &role,
                &task,
                &impl_name,
                &mut impl_writer,
            )
            .await;

            let elapsed = start_time.elapsed();
            let elapsed_str = format_duration_human(elapsed);

            // Store notes if provided
            if let Some(ref notes) = completion.notes {
                let mut tm = task_manager.write().await;
                tm.add_note(&role, task_id.clone(), notes.clone());
            }

            // Create suggested tasks if provided
            if let Some(ref suggested_tasks) = completion.add_tasks {
                let mut tm = task_manager.write().await;
                for suggested in suggested_tasks {
                    let deps = suggested.depends_on.clone().unwrap_or_default();
                    let new_id = tm.create(&suggested.name, &suggested.description, deps);
                    tracing::info!("📋 Created suggested task [{}]: {}", new_id, suggested.name);
                }
            }

            // Update task status if linked to a tracked task
            if let Some(ref tid) = task_id {
                update_task_completion(
                    &task_manager,
                    tid,
                    completion.success,
                    completion.message.as_deref(),
                )
                .await;
            }

            // Finalize the writer
            if let Err(e) = impl_writer.finalize(completion.success).await {
                tracing::warn!("Failed to finalize implementer log: {}", e);
            }

            // Unregister from router
            {
                let mut router = session_router.write().await;
                router.unregister(&session_id_clone);
            }

            if completion.success {
                tracing::info!(
                    "✅ [concurrent] Agent {} completed ({}) - {}",
                    role,
                    elapsed_str,
                    truncate_for_log(&task, 60)
                );
            } else {
                tracing::error!(
                    "❌ [concurrent] Agent {} FAILED after {} - {}",
                    role,
                    elapsed_str,
                    truncate_for_log(&task, 80)
                );
            }

            let result = AgentResult {
                role,
                task,
                success: completion.success,
                message: completion.message,
            };

            // Send result (ignore if receiver dropped)
            let _ = result_tx.send(result);
        });

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
                    // Get role and task from spec with defaults
                    let role = spec
                        .role
                        .clone()
                        .unwrap_or_else(|| "implementer".to_string());
                    let task = spec
                        .task
                        .clone()
                        .or_else(|| spec.task_id.clone())
                        .unwrap_or_else(|| "(unknown)".to_string());
                    receivers.push((role, task, rx));
                }
                Err(e) => {
                    let role = spec
                        .role
                        .clone()
                        .unwrap_or_else(|| "implementer".to_string());
                    let task = spec
                        .task
                        .clone()
                        .or_else(|| spec.task_id.clone())
                        .unwrap_or_else(|| "(unknown)".to_string());
                    tracing::error!("Failed to spawn agent [{}]: {:#}", role, e);
                    spawn_errors.push(AgentResult {
                        role,
                        task,
                        success: false,
                        message: Some(format!("Failed to spawn: {e:#}")),
                    });
                }
            }
        }

        match wait {
            WaitMode::None => {
                // Fire and forget - return immediately with spawn errors only
                tracing::info!(
                    "🔥 Fire-and-forget mode: {} agents spawned",
                    receivers.len()
                );
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
