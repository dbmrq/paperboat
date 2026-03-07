//! Generic agent spawning.
//!
//! This module provides a generalized interface for spawning worker agents
//! (implementers, verifiers, etc.) with support for concurrent execution.

use super::socket::{setup_agent_socket, AgentSocketHandle};
use super::types::{format_duration_human, truncate_for_log, ToolMessage};
use super::App;
use crate::acp::AcpClientTrait;
use crate::agents::AgentRole;
use crate::logging::AgentWriter;
use crate::mcp_server::{AgentSpec, ResolvedAgentSpec, SuggestedTask, ToolCall, ToolResponse, WaitMode};
use crate::tasks::{TaskManager, TaskStatus};
use anyhow::{Context, Result};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

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

/// Internal result of an agent completion, including notes and suggested tasks.
struct AgentCompletionData {
    success: bool,
    message: Option<String>,
    notes: Option<String>,
    add_tasks: Option<Vec<SuggestedTask>>,
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

    /// Spawn an implementer agent (convenience wrapper for spawn_agent_with_resolved_spec).
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

    /// Spawn an agent based on a ResolvedAgentSpec.
    ///
    /// Determines the prompt and removed_tools based on the role:
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
                    "{}\n\n## When Done\n\n\
                    Call `complete` with:\n\
                    - **success**: Whether you accomplished your task (true/false)\n\
                    - **message**: Brief summary of what you found or did\n\
                    - **notes** (optional): Context for future agents or the orchestrator\n\
                    - **add_tasks** (optional): Any work you discovered that should be done",
                    custom_prompt
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
                        .filter(|t| !allowed_tools.contains(&t.to_string()))
                        .map(|s| s.to_string())
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
                    .ok_or_else(|| anyhow::anyhow!("No template for role: {:?}", role))?;

                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{user_goal}", &self.original_goal);
                let removed = template.removed_tools.iter().map(|s| s.to_string()).collect();

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
            env_vars.push(
                json!({"name": "VILLALOBOS_REMOVED_TOOLS", "value": removed_tools.join(",")}),
            );
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

    /// Spawn an agent with its own dedicated Unix socket.
    ///
    /// This is used for concurrent agent execution where each agent needs
    /// its own socket to receive tool call responses.
    ///
    /// Returns (`session_id`, prompt, `AgentSocketHandle`, `AcpClient`) so the caller can:
    /// - Log the prompt
    /// - Receive tool calls on the agent's dedicated socket
    /// - Clean up the socket when done
    /// - Keep the AcpClient alive until the agent completes
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
        let socket_handle = setup_agent_socket(&agent_id)
            .await
            .with_context(|| {
                format!(
                    "Failed to create agent socket for agent_id={}",
                    &agent_id[..8]
                )
            })?;
        tracing::debug!("Socket created successfully for agent_id={}", &agent_id[..8]);

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
                    "{}\n\n## When Done\n\n\
                    Call `complete` with:\n\
                    - **success**: Whether you accomplished your task (true/false)\n\
                    - **message**: Brief summary of what you found or did\n\
                    - **notes** (optional): Context for future agents or the orchestrator\n\
                    - **add_tasks** (optional): Any work you discovered that should be done",
                    custom_prompt
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
                        .filter(|t| !allowed_tools.contains(&t.to_string()))
                        .map(|s| s.to_string())
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
                    .ok_or_else(|| anyhow::anyhow!("No template for role: {:?}", role))?;

                let prompt = template
                    .prompt_template
                    .replace("{task}", &spec.task)
                    .replace("{context}", context);
                let removed = template.removed_tools.iter().map(|s| s.to_string()).collect();

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
                    .replace("{context}", context);
                (prompt, vec![])
            }
        };

        // Build environment variables for the MCP server
        let mut env_vars = vec![json!({"name": "VILLALOBOS_AGENT_TYPE", "value": spec.role.clone()})];

        // Add removed tools to environment if any
        if !removed_tools.is_empty() {
            env_vars.push(
                json!({"name": "VILLALOBOS_REMOVED_TOOLS", "value": removed_tools.join(",")}),
            );
        }

        // Configure MCP server with agent type
        // Pass socket path as an ARG (not just env var) to ensure auggie spawns unique processes
        // Auggie may cache MCP servers by command+args, so unique args = unique process
        let mcp_server_name = format!("villalobos-{}-{}", spec.role, &agent_id[..8]);
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
        agent_acp
            .initialize()
            .await
            .with_context(|| format!("Failed to initialize auggie for agent_id={}", &agent_id[..8]))?;

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

    /// Spawn an agent asynchronously with its own dedicated socket.
    pub(crate) async fn spawn_agent_async(
        &mut self,
        spec: &AgentSpec,
    ) -> Result<(String, oneshot::Receiver<AgentResult>)> {
        self.spawn_agent_async_with_context(spec, "").await
    }

    /// Resolve an AgentSpec using the TaskManager for task_id lookups.
    async fn resolve_agent_spec(&self, spec: &AgentSpec) -> Result<ResolvedAgentSpec> {
        let task_manager = self.task_manager.read().await;
        spec.resolve(|tid| {
            task_manager
                .get(&tid.to_string())
                .map(|t| t.description.clone())
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
                TaskStatus::InProgress {
                    agent_session: Some(impl_name.clone()),
                },
            );
            tracing::info!("📋 Task {} marked as InProgress (agent: {})", tid, impl_name);
        }

        // Write a preliminary header so we have context in the log even if spawn fails
        if let Err(e) = impl_writer.write_header(&task).await {
            tracing::warn!("Failed to write preliminary header: {}", e);
        }

        // Spawn the agent with its own dedicated socket and auggie instance
        let spawn_result = self.spawn_agent_with_own_socket(&resolved, context).await;

        // If spawn failed, log the error to the implementer log file before propagating
        let (session_id, agent_prompt, socket_handle, agent_acp) = match spawn_result {
            Ok(result) => result,
            Err(e) => {
                // Mark task as failed if spawn failed
                if let Some(ref tid) = task_id {
                    let mut tm = self.task_manager.write().await;
                    tm.update_status(
                        tid,
                        TaskStatus::Failed {
                            error: format!("Agent spawn failed: {}", e),
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

        // Create the completion channel
        let (result_tx, result_rx) = oneshot::channel();

        // Register session with router for ACP messages
        let session_rx = self.register_session(&session_id).await;

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
            let completion = Self::run_agent_handler(
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
                    tracing::info!(
                        "📋 Created suggested task [{}]: {}",
                        new_id,
                        suggested.name
                    );
                }
            }

            // Update task status if linked to a tracked task
            if let Some(ref tid) = task_id {
                Self::update_task_completion(&task_manager, tid, completion.success, completion.message.as_deref()).await;
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

    /// Update task status when an agent completes.
    async fn update_task_completion(
        task_manager: &Arc<RwLock<TaskManager>>,
        task_id: &str,
        success: bool,
        message: Option<&str>,
    ) {
        let mut tm = task_manager.write().await;
        if success {
            tm.update_status(
                &task_id.to_string(),
                TaskStatus::Complete {
                    success: true,
                    summary: message.unwrap_or("Task completed").to_string(),
                },
            );
            tracing::info!("📋 Task {} marked as Complete", task_id);
        } else {
            tm.update_status(
                &task_id.to_string(),
                TaskStatus::Failed {
                    error: message.unwrap_or("Task failed").to_string(),
                },
            );
            tracing::info!("📋 Task {} marked as Failed", task_id);
        }
    }

    /// Run the agent handler task, processing tool calls until completion.
    ///
    /// This handles the agent's dedicated socket, responding to tool calls
    /// (especially the `Complete` call that signals the agent is done).
    async fn run_agent_handler(
        mut socket_handle: AgentSocketHandle,
        mut session_rx: tokio::sync::mpsc::Receiver<serde_json::Value>,
        timeout: std::time::Duration,
        role: &str,
        task: &str,
        agent_name: &str,
        writer: &mut AgentWriter,
    ) -> AgentCompletionData {
        let result = tokio::time::timeout(timeout, async {
            loop {
                tokio::select! {
                    // Handle tool calls from the agent's socket
                    Some(tool_msg) = socket_handle.tool_rx.recv() => {
                        let ToolMessage::Request { request, response_tx } = tool_msg;

                        match &request.tool_call {
                            ToolCall::Complete { success, message, notes, add_tasks } => {
                                tracing::info!(
                                    "✅ [{}] Agent {} signaled complete (socket={:?}): success={}, message={:?}",
                                    agent_name, role, socket_handle.socket_path, success, message
                                );

                                if let Some(msg) = message {
                                    let _ = writer.write_result(msg).await;
                                }

                                // Log notes if provided
                                if let Some(n) = notes {
                                    tracing::info!("📝 [{}] Agent notes: {}", agent_name, n);
                                }

                                // Log add_tasks if provided
                                if let Some(tasks) = add_tasks {
                                    tracing::info!("📋 [{}] Agent suggested {} task(s)", agent_name, tasks.len());
                                }

                                // Send success response
                                let response = ToolResponse::success(
                                    request.request_id,
                                    message.clone().unwrap_or_else(|| "Done".to_string()),
                                );
                                let _ = response_tx.send(response);

                                // Clean up socket before returning
                                socket_handle.cleanup();

                                return AgentCompletionData {
                                    success: *success,
                                    message: message.clone(),
                                    notes: notes.clone(),
                                    add_tasks: add_tasks.clone(),
                                };
                            }
                            other => {
                                // Worker agents should only call complete()
                                // Log warning and return error
                                tracing::warn!(
                                    "⚠️ [{}] Agent {} made unexpected tool call: {:?}",
                                    agent_name, role, other.tool_type()
                                );
                                let response = ToolResponse::failure(
                                    request.request_id,
                                    format!(
                                        "Worker agents can only call complete(). \
                                         Tool '{}' is not available.",
                                        other.tool_type()
                                    ),
                                );
                                let _ = response_tx.send(response);
                            }
                        }
                    }

                    // Handle ACP session messages (for logging)
                    Some(msg) = session_rx.recv() => {
                        // Check for session_finished (fallback completion signal)
                        if let Some(params) = msg.get("params") {
                            if let Some(update) = params.get("update") {
                                if let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
                                    if session_update == "session_finished" {
                                        tracing::debug!(
                                            "[{}] Agent {} received session_finished without complete call",
                                            agent_name, role
                                        );
                                        // Clean up socket
                                        socket_handle.cleanup();
                                        // Treat as failure since agent didn't call complete()
                                        return AgentCompletionData {
                                            success: false,
                                            message: Some(format!("Agent finished without calling complete() for task: {task}")),
                                            notes: None,
                                            add_tasks: None,
                                        };
                                    }
                                }
                            }
                        }
                    }

                    // Both channels closed unexpectedly
                    else => {
                        tracing::warn!(
                            "[{}] Agent {} channels closed unexpectedly",
                            agent_name, role
                        );
                        socket_handle.cleanup();
                        return AgentCompletionData {
                            success: false,
                            message: Some(format!("Agent channels closed for task: {task}")),
                            notes: None,
                            add_tasks: None,
                        };
                    }
                }
            }
        })
        .await;

        if let Ok(data) = result {
            data
        } else {
            tracing::error!(
                "⏰ [{}] Agent {} timed out after {:?}",
                agent_name, role, timeout
            );
            // Socket cleanup happens when socket_handle is dropped
            AgentCompletionData {
                success: false,
                message: Some(format!("Agent timed out for task: {task}")),
                notes: None,
                add_tasks: None,
            }
        }
    }

    /// Internal helper to wait for an agent to complete via session messages.
    ///
    /// This is a simplified version that just waits for the session to finish.
    /// Used by the old sequential mode - kept for fallback compatibility.
    #[allow(dead_code)]
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
    /// Generate context about neighboring tasks and dependency summaries for an implementer.
    async fn generate_task_context(&self, agents: &[AgentSpec], index: usize, spec: &AgentSpec) -> String {
        let mut sections = Vec::new();

        // 1. Dependency summaries from TaskManager (if task_id is provided)
        if let Some(ref task_id) = spec.task_id {
            let tm = self.task_manager.read().await;
            if let Some(dep_summary) = tm.format_dependency_summaries(task_id) {
                sections.push(dep_summary);
            }
        }

        // 2. Neighboring tasks context
        if agents.len() > 1 {
            let mut neighbor_lines = Vec::new();

            // Helper to get task description from spec
            let get_task = |spec: &AgentSpec| -> String {
                spec.task
                    .clone()
                    .or_else(|| spec.task_id.clone().map(|id| format!("[{}]", id)))
                    .unwrap_or_else(|| "(unknown task)".to_string())
            };

            // Previous tasks (up to 2)
            let prev_start = index.saturating_sub(2);
            if prev_start < index {
                neighbor_lines.push("## Previous Tasks".to_string());
                for i in prev_start..index {
                    neighbor_lines.push(format!("- {}", truncate_for_log(&get_task(&agents[i]), 100)));
                }
            }

            // Next tasks (up to 2)
            let next_end = (index + 3).min(agents.len());
            if index + 1 < next_end {
                neighbor_lines.push("## Next Tasks".to_string());
                for i in (index + 1)..next_end {
                    neighbor_lines.push(format!("- {}", truncate_for_log(&get_task(&agents[i]), 100)));
                }
            }

            if !neighbor_lines.is_empty() {
                sections.push(neighbor_lines.join("\n"));
            }
        }

        sections.join("\n\n")
    }

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
                    let role = spec.role.clone().unwrap_or_else(|| "implementer".to_string());
                    let task = spec
                        .task
                        .clone()
                        .or_else(|| spec.task_id.clone())
                        .unwrap_or_else(|| "(unknown)".to_string());
                    receivers.push((role, task, rx));
                }
                Err(e) => {
                    let role = spec.role.clone().unwrap_or_else(|| "implementer".to_string());
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

