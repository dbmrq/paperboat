//! Orchestrator agent handling.
//!
//! The orchestrator is responsible for coordinating task execution by delegating
//! work to specialized agents. It has access to the following MCP tools:
//!
//! - **`decompose`**: Break down a task into smaller subtasks with a nested orchestrator
//! - **`spawn_agents`**: Spawn one or more worker agents to execute tasks
//! - **`complete`**: Signal that orchestration is finished
//! - **`create_task`**: Dynamically add new tasks to the plan
//! - **`skip_tasks`**: Mark tasks as skipped (not needed)
//!
//! # Task Reconciliation
//!
//! When the orchestrator calls `complete(success=true)`, the system performs
//! reconciliation to ensure all tasks have a definitive final status:
//!
//! 1. Any tasks still in `NotStarted` status are flagged as pending
//! 2. If pending tasks exist, completion is rejected with a message asking the
//!    orchestrator to either spawn agents or skip the remaining tasks
//! 3. Only after all tasks are in a terminal state (Complete, Failed, or Skipped)
//!    will the completion be accepted
//!
//! This prevents the orchestrator from finishing with unaddressed tasks, ensuring
//! an accurate audit trail in the task list.

use super::retry::{is_transient_error, RetryConfig};
use super::types::{truncate_for_log, ToolMessage, ORCHESTRATOR_PROMPT};
use super::App;
use crate::acp::SessionMode;
use crate::backend::transport::{SessionConfig, SessionInfo};
use crate::logging::AgentWriter;
use crate::mcp_server::{AgentSpec, TaskStateInfo, ToolCall, ToolResponse};
use crate::tasks::TaskStatus;
use crate::types::TaskResult;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

impl App {
    /// Spawn an orchestrator agent.
    /// Returns (`session_id`, `model`, `full_prompt`) so the model and prompt can be logged.
    #[tracing::instrument(skip(self, prompt), fields(agent_type = "orchestrator", session_id))]
    pub(crate) async fn spawn_orchestrator(
        &mut self,
        prompt: &str,
    ) -> Result<(String, String, String)> {
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();

        // Get the path to the current binary
        let binary_path =
            std::env::current_exe().context("Failed to get current executable path")?;

        // Get socket path
        let socket_path = self
            .socket_path
            .as_ref()
            .context("Socket not set up")?
            .to_string_lossy()
            .to_string();

        // Configure MCP server
        // For stdio transport, env is an array of {name, value} objects
        // Use unique name "paperboat-orchestrator" to prevent caching issues
        // Pass --socket directly to avoid env var caching issues across auggie sessions
        let mcp_servers = vec![json!({
            "name": "paperboat-orchestrator",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server", "--socket", &socket_path],
            "env": [{
                "name": "PAPERBOAT_AGENT_TYPE",
                "value": "orchestrator"
            }]
        })];

        tracing::info!("🎭 Spawning orchestrator with MCP tools");

        // Create session with retry logic for transient MCP server startup errors
        let (response, model) = self
            .create_orchestrator_session_with_retry(mcp_servers, &cwd)
            .await?;

        // Get the goal from task manager
        let goal = {
            let tm = self.task_manager.read().await;
            tm.format_goal()
        };

        let full_prompt = ORCHESTRATOR_PROMPT
            .replace("{goal}", &goal)
            .replace("{plan}", prompt);

        // Record session_id in the current span for tracing correlation
        tracing::Span::current().record("session_id", &response.session_id);
        tracing::debug!("🎭 Orchestrator prompt:\n{}", full_prompt);

        self.acp_orchestrator
            .send_prompt(&response.session_id, &full_prompt)
            .await?;

        Ok((response.session_id, model, full_prompt))
    }

    /// Create an orchestrator session with retry logic.
    ///
    /// This handles transient MCP server startup errors by retrying the session
    /// creation with exponential backoff.
    /// Returns (`SessionInfo`, `model`) so the model can be recorded in logs.
    async fn create_orchestrator_session_with_retry(
        &mut self,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<(SessionInfo, String)> {
        let retry_config = RetryConfig::from_env();
        let model = self.resolve_orchestrator_model()?;
        let mut attempt = 0;
        let mut delay = retry_config.initial_delay;

        loop {
            attempt += 1;

            // Create session config with Agent mode - orchestrator needs to call MCP tools
            // Note: Cursor's "plan" mode is read-only and can't call tools
            let config = SessionConfig::new(&model, cwd)
                .with_mcp_servers(mcp_servers.clone())
                .with_mode(SessionMode::Agent);

            match self.acp_orchestrator.create_session(config).await {
                Ok(response) => {
                    if attempt > 1 {
                        tracing::info!(
                            "🔄 Orchestrator create_session succeeded on attempt {}/{}",
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
                            "⚠️ Orchestrator create_session failed (attempt {}/{}): {}. Retrying in {:?}...",
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
                            "❌ Orchestrator create_session failed after {attempt} attempt(s) ({reason}): {e:#}",
                        );
                        return Err(e).context(format!(
                            "Orchestrator create_session failed after {attempt} attempt(s)"
                        ));
                    }
                }
            }
        }
    }

    /// Spawn and run an orchestrator agent with logging.
    pub(crate) fn run_orchestrator_with_writer<'a>(
        &'a mut self,
        prompt: &'a str,
        writer: &'a mut AgentWriter,
    ) -> Pin<Box<dyn Future<Output = Result<TaskResult>> + 'a>> {
        Box::pin(async move { self.run_orchestrator_with_writer_impl(prompt, writer).await })
    }

    #[allow(clippy::ignored_unit_patterns)] // `update` pattern is () when tui feature disabled
    #[tracing::instrument(skip(self, prompt, writer), fields(agent_type = "orchestrator"))]
    async fn run_orchestrator_with_writer_impl(
        &mut self,
        prompt: &str,
        writer: &mut AgentWriter,
    ) -> Result<TaskResult> {
        let (session_id, model, full_prompt) = match self.spawn_orchestrator(prompt).await {
            Ok(result) => result,
            Err(e) => {
                // Write error to orchestrator log so it's not empty
                let depth = self.current_scope.depth();
                tracing::error!("[L{}] ❌ Failed to spawn orchestrator: {:#}", depth, e);
                if let Err(write_err) = writer.write_spawn_error(&e).await {
                    tracing::warn!(
                        "Failed to write spawn error to orchestrator log: {}",
                        write_err
                    );
                }
                // Note: finalize is called by the caller (run_execution_phase)
                return Err(e);
            }
        };
        writer.set_session_id(session_id.clone());
        writer.set_model(model);
        // Use the plan as the task description, but log the full prompt for debugging
        if let Err(e) = writer.write_header_with_prompt(prompt, &full_prompt).await {
            tracing::warn!("Failed to write orchestrator header: {}", e);
        }
        // Emit AgentStarted event for TUI
        writer.emit_agent_started(prompt);

        // Take tool_rx for this orchestrator run, but restore it when done
        let mut tool_rx = self.tool_rx.take().context("Tool receiver not set up")?;

        // Take the config update receiver out of self so we don't hold a mutable borrow
        #[cfg(feature = "tui")]
        let mut config_update_rx = self.config_update_rx.take();

        // Handle tool calls from MCP server
        let result = loop {
            // Config update future - receives from TUI channel when available
            #[cfg(feature = "tui")]
            let config_update_fut = async {
                match config_update_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            };
            #[cfg(not(feature = "tui"))]
            let config_update_fut = std::future::pending::<Option<()>>();

            tokio::select! {
                // Use biased selection to ensure tool_rx is checked first.
                // This is critical for tests: when mock ACP recv() injects a tool call,
                // we must process it before polling recv() again, or tool calls may
                // be interleaved (e.g., orchestrator's complete received by implementer).
                biased;

                // Handle config updates from TUI (update is only used with tui feature)
                Some(update) = config_update_fut => {
                    let _ = &update; // Suppress unused variable warning when tui feature is disabled
                    #[cfg(feature = "tui")]
                    self.apply_model_config_update(&update);
                }

                Some(tool_msg) = tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;
                    tracing::debug!("📨 Orchestrator MCP tool call received: {:?}", request.tool_call.tool_type());

                    match &request.tool_call {
                        ToolCall::Decompose { task_id, task } => {
                            let resolved_task =
                                self.resolve_task_description(task_id.as_ref(), task.as_ref()).await;

                            let depth = self.current_scope.depth();
                            let _ = writer
                                .write_mcp_tool_call(
                                    "decompose",
                                    &truncate_for_log(&resolved_task, 100),
                                )
                                .await;
                            tracing::info!(
                                "[L{}] 🔄 decompose({})",
                                depth,
                                truncate_for_log(&resolved_task, 80)
                            );

                            eprintln!(
                                "[orchestrator] Before calling handle_decompose_with_response"
                            );
                            self.tool_rx = Some(tool_rx);
                            let response = self
                                .handle_decompose_with_response(&resolved_task, &request.request_id)
                                .await;
                            eprintln!(
                                "[orchestrator] After handle_decompose_with_response: success={}",
                                response.success
                            );
                            tool_rx = self
                                .tool_rx
                                .take()
                                .context("Tool receiver lost during decompose")?;

                            let _ = writer
                                .write_mcp_tool_result(
                                    "decompose",
                                    response.success,
                                    &truncate_for_log(&response.summary, 100),
                                )
                                .await;

                            eprintln!("[orchestrator] Decompose complete, continuing loop");
                            let _ = response_tx.send(response);
                        }
                        ToolCall::SpawnAgents { ref agents, ref wait } => {
                            // Log the MCP tool call to orchestrator log
                            let agent_count = agents.len();
                            let depth = self.current_scope.depth();
                            let desc = format!("{agent_count} agent(s), wait={wait:?}");
                            let _ = writer.write_mcp_tool_call("spawn_agents", &desc).await;
                            tracing::info!("[L{}] 🚀 spawn_agents({agent_count} agents, wait={wait:?})", depth);

                            // Log each agent being spawned
                            for (i, agent) in agents.iter().enumerate() {
                                let role = agent.role.as_deref().unwrap_or("implementer");
                                let task_desc = agent
                                    .task
                                    .as_deref()
                                    .or(agent.task_id.as_deref())
                                    .unwrap_or("(no task)");
                                tracing::info!(
                                    "[L{}]   📋 Agent {}/{}: [{}] {}",
                                    depth,
                                    i + 1,
                                    agent_count,
                                    role,
                                    truncate_for_log(task_desc, 60)
                                );
                            }

                            // Validate agent specifications before spawning
                            if let Some(error_msg) = Self::validate_agent_specs(agents) {
                                let response =
                                    ToolResponse::failure(request.request_id.clone(), error_msg);
                                let _ = response_tx.send(response);
                                continue;
                            }

                            // Choose execution mode based on router_active:
                            // - Concurrent mode (router_active=true): Each agent gets its own socket
                            //   and tool handler, enabling true parallel execution
                            // - Sequential mode (router_active=false): Fallback for mock tests where
                            //   agents share the orchestrator's tool_rx channel
                            let (all_success, combined_summary) = if self.router_active {
                                // Concurrent mode: use per-agent sockets
                                tracing::info!(
                                    "🚀 Running {} agents in CONCURRENT mode",
                                    agents.len()
                                );

                                let results =
                                    self.spawn_agents_concurrent(agents.clone(), *wait).await;
                                self.process_concurrent_results(&results).await
                            } else {
                                // Sequential mode: spawn agents one at a time
                                // This uses handle_implement_with_response which works with mock clients
                                tracing::debug!(
                                    "Running {} agents in SEQUENTIAL mode (mock/test)",
                                    agents.len()
                                );

                                // Snapshot task IDs before spawning to detect new tasks
                                let task_ids_before: std::collections::HashSet<String> = {
                                    let tm = self.task_manager.read().await;
                                    tm.list_task_ids().into_iter().collect()
                                };

                                let mut summaries = Vec::new();

                                for agent in agents {
                                    let role =
                                        agent.role.as_deref().unwrap_or("implementer").to_string();

                                    // Resolve task_id (with inference like concurrent mode)
                                    let resolved_task_id = self.resolve_task_id_for_agent(agent).await;

                                    let task = self
                                        .resolve_task_description(agent.task_id.as_ref(), agent.task.as_ref())
                                        .await;

                                    // Mark task as InProgress before spawning (if we have a task_id)
                                    if let Some(ref tid) = resolved_task_id {
                                        let mut tm = self.task_manager.write().await;
                                        tm.update_status(
                                            tid,
                                            &TaskStatus::InProgress {
                                                agent_session: None,
                                            },
                                        );
                                        tracing::info!(
                                            "📋 Task {} marked as InProgress (sequential mode)",
                                            tid
                                        );
                                    }

                                    // Restore tool_rx before spawning (needs to receive complete signal)
                                    self.tool_rx = Some(tool_rx);
                                    let impl_response = self
                                        .handle_implement_with_response(&task, &request.request_id)
                                        .await;
                                    tool_rx = self
                                        .tool_rx
                                        .take()
                                        .context("Tool receiver lost during spawn_agents")?;

                                    // Update task status based on result (if we have a task_id)
                                    if let Some(ref tid) = resolved_task_id {
                                        let mut tm = self.task_manager.write().await;
                                        let status = if impl_response.success {
                                            tracing::info!("📋 Task {} marked as Complete", tid);
                                            TaskStatus::Complete {
                                                success: true,
                                                summary: impl_response.summary.clone(),
                                            }
                                        } else {
                                            tracing::info!("📋 Task {} marked as Failed", tid);
                                            TaskStatus::Failed {
                                                error: impl_response.summary.clone(),
                                            }
                                        };
                                        tm.update_status(tid, &status);
                                    }

                                    let status = if impl_response.success { "✓" } else { "✗" };
                                    summaries.push(format!(
                                        "[{}] {} {}",
                                        role,
                                        status,
                                        truncate_for_log(&impl_response.summary, 80)
                                    ));
                                }

                                self.process_sequential_results(summaries, &task_ids_before)
                                    .await
                            };

                            // Fetch task state for context-aware response
                            let task_state = self.get_task_state().await;

                            let response = if all_success {
                                ToolResponse::success(request.request_id.clone(), combined_summary)
                                    .with_task_state(task_state)
                            } else {
                                ToolResponse::failure(request.request_id.clone(), combined_summary)
                                    .with_task_state(task_state)
                            };

                            // Log the result to orchestrator log
                            let _ = writer.write_mcp_tool_result(
                                "spawn_agents",
                                response.success,
                                &truncate_for_log(&response.summary, 100)
                            ).await;

                            let _ = response_tx.send(response);
                        }
                        ToolCall::Complete { success, message, .. } => {
                            let depth = self.current_scope.depth();
                            let success_str =
                                if *success { "success=true" } else { "success=false" };
                            let msg_preview = message
                                .as_deref()
                                .map(|m| truncate_for_log(m, 50))
                                .unwrap_or_default();
                            let _ = writer
                                .write_mcp_tool_call(
                                    "complete",
                                    &format!("{success_str}, {msg_preview}"),
                                )
                                .await;
                            tracing::info!("[L{}] 🏁 complete({success_str})", depth);

                            // Task reconciliation: check for pending tasks before allowing success
                            if *success {
                                if let Some(pending_msg) = self.check_completion_blockers().await {
                                    let response = ToolResponse::success(
                                        request.request_id.clone(),
                                        pending_msg,
                                    );
                                    let _ = writer
                                        .write_mcp_tool_result("complete", false, "Rejected: pending tasks remain")
                                        .await;
                                    let _ = response_tx.send(response);
                                    continue;
                                }
                            }

                            // No pending tasks or success=false - proceed with completion
                            if let Some(msg) = &message {
                                let _ = writer.write_result(msg).await;
                            }

                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                message
                                    .clone()
                                    .unwrap_or_else(|| "Orchestration complete".to_string()),
                            );
                            let _ = writer
                                .write_mcp_tool_result(
                                    "complete",
                                    *success,
                                    if *success {
                                        "Accepted"
                                    } else {
                                        "Task marked as failed"
                                    },
                                )
                                .await;
                            let _ = response_tx.send(response);

                            // Brief drain for any final messages (500ms max)
                            let drain_result = tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                self.drain_orchestrator_messages(&session_id, writer),
                            )
                            .await;

                            if drain_result.is_err() {
                                tracing::trace!("Drain timeout, proceeding");
                            }

                            break TaskResult {
                                success: *success,
                                message: message.clone(),
                            };
                        }
                        ToolCall::CreateTask { name, description, dependencies } => {
                            let response = self
                                .handle_create_task(
                                    name,
                                    description,
                                    dependencies,
                                    &request.request_id,
                                    writer,
                                )
                                .await;
                            let _ = response_tx.send(response);
                        }
                        ToolCall::SetGoal { .. } => {
                            // SetGoal is only for planner agents, not orchestrator
                            tracing::warn!("Orchestrator received unexpected SetGoal call");
                            let response = ToolResponse::failure(
                                request.request_id.clone(),
                                "set_goal is not available to orchestrator agents".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                        ToolCall::SkipTasks { ref task_ids, ref reason } => {
                            let response = self
                                .handle_skip_tasks(task_ids, reason.as_ref(), &request.request_id, writer)
                                .await;
                            let _ = response_tx.send(response);
                        }
                        ToolCall::ListTasks { ref status_filter } => {
                            let response = self
                                .handle_list_tasks(status_filter.as_ref(), &request.request_id, writer)
                                .await;
                            let _ = response_tx.send(response);
                        }
                    }
                }

                Ok(msg) = self.acp_orchestrator.recv() => {
                    tracing::trace!("📨 Orchestrator received ACP message");
                    self.handle_acp_message_with_writer(&msg, writer).await;
                }

                Ok(msg) = self.acp_worker.recv() => {
                    self.handle_acp_message(&msg, "worker").await;
                }
            }
        };

        self.tool_rx = Some(tool_rx);
        #[cfg(feature = "tui")]
        {
            self.config_update_rx = config_update_rx;
        }
        Ok(result)
    }

    /// Fetch current task state for context-aware responses.
    ///
    /// Creates a [`TaskStateInfo`] snapshot using the `TaskManager` helper methods.
    /// This is efficient because it only acquires a read lock and collects
    /// the minimal information needed for response building.
    async fn get_task_state(&self) -> TaskStateInfo {
        let tm = self.task_manager.read().await;
        let pending = tm.get_pending_tasks();
        let parallel = tm.get_parallel_tasks();
        let blocked = tm.get_blocked_tasks();

        TaskStateInfo {
            pending_count: pending.len(),
            parallel_tasks: parallel,
            blocked_tasks: blocked,
        }
    }

    /// Resolve a task description from `task_id` or task field.
    ///
    /// Returns the task description if found, or a fallback message.
    async fn resolve_task_description(
        &self,
        task_id: Option<&String>,
        task: Option<&String>,
    ) -> String {
        if let Some(tid) = task_id {
            let tm = self.task_manager.read().await;
            tm.get_by_id_or_name(tid).map_or_else(
                || {
                    tracing::warn!(
                        "Task '{}' not found in TaskManager. Available: {:?}",
                        tid,
                        tm.list_task_ids()
                    );
                    format!("(task {tid} not found)")
                },
                |t| t.description.clone(),
            )
        } else {
            task.cloned().unwrap_or_else(|| "(no task)".to_string())
        }
    }

    /// Resolve the `task_id` for an agent spec, with inference support.
    ///
    /// If `task_id` is provided, uses it directly (with validation).
    /// If not, tries to infer the `task_id` from the task description by matching
    /// against existing tasks in the `TaskManager`.
    ///
    /// This mirrors the behavior in concurrent mode (`resolve_agent_spec`).
    async fn resolve_task_id_for_agent(&self, spec: &AgentSpec) -> Option<String> {
        // If task_id is explicitly provided, validate and return it
        if let Some(ref tid) = spec.task_id {
            let tm = self.task_manager.read().await;
            if tm.get_by_id_or_name(tid).is_some() {
                return Some(tid.clone());
            }
            tracing::warn!(
                "Task '{}' not found in TaskManager. Available: {:?}",
                tid,
                tm.list_task_ids()
            );
            return None;
        }

        // Try to infer task_id from task description
        if let Some(ref task_desc) = spec.task {
            let tm = self.task_manager.read().await;
            if let Some(found_id) = tm.find_by_name_or_description(task_desc) {
                tracing::info!(
                    "📋 Inferred task_id '{}' from task description: {}",
                    found_id,
                    truncate_for_log(task_desc, 60)
                );
                return Some(found_id);
            }
        }

        None
    }

    /// Handle the `create_task` tool call.
    ///
    /// Creates a new task in the task manager and returns a response.
    async fn handle_create_task(
        &self,
        name: &str,
        description: &str,
        dependencies: &[String],
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        let depth = self.current_scope.depth();
        let _ = writer.write_mcp_tool_call("create_task", name).await;

        let task_id = {
            let mut tm = self.task_manager.write().await;
            tm.create(name, description, dependencies.to_vec())
        };

        tracing::info!(
            "[L{}] 📋 Orchestrator created task [{}]: {}",
            depth,
            task_id,
            name
        );

        let task_state = self.get_task_state().await;

        let response = ToolResponse::success(
            request_id.to_string(),
            format!(
                "Task '{name}' created with ID: {task_id}. \
                 Remember to use spawn_agents with task_id=\"{task_id}\" to execute it."
            ),
        )
        .with_task_state(task_state);

        let _ = writer
            .write_mcp_tool_result("create_task", true, &format!("Created {task_id}"))
            .await;

        response
    }

    /// Handle the `list_tasks` tool call.
    ///
    /// Returns a formatted list of tasks, optionally filtered by status.
    async fn handle_list_tasks(
        &self,
        status_filter: Option<&String>,
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        use crate::tasks::TaskStatus;

        let depth = self.current_scope.depth();
        let filter = status_filter.map_or("all", String::as_str);
        let _ = writer
            .write_mcp_tool_call("list_tasks", &format!("filter={filter}"))
            .await;
        tracing::info!("[L{}] 📋 list_tasks(filter={filter})", depth);

        let task_list = {
            let tm = self.task_manager.read().await;
            let tasks = tm.all_tasks();

            // Filter by status if specified
            let filtered: Vec<_> = tasks
                .into_iter()
                .filter(|t| match filter {
                    "pending" => matches!(t.status, TaskStatus::NotStarted),
                    "in_progress" => matches!(t.status, TaskStatus::InProgress { .. }),
                    "completed" => matches!(t.status, TaskStatus::Complete { .. }),
                    "failed" => matches!(t.status, TaskStatus::Failed { .. }),
                    "skipped" => matches!(t.status, TaskStatus::Skipped { .. }),
                    _ => true,
                })
                .collect();

            // Format task list
            let mut lines = Vec::new();
            lines.push(format!(
                "## Tasks ({} total, filter={})\n",
                filtered.len(),
                filter
            ));
            for task in filtered {
                let status_str = match &task.status {
                    TaskStatus::NotStarted => "pending",
                    TaskStatus::InProgress { .. } => "in_progress",
                    TaskStatus::Complete { .. } => "completed",
                    TaskStatus::Failed { .. } => "failed",
                    TaskStatus::Skipped { .. } => "skipped",
                };
                lines.push(format!(
                    "- **[{}]** {} ({}): {}",
                    task.id, task.name, status_str, task.description
                ));
            }
            lines.join("\n")
        };

        let response = ToolResponse::success(request_id.to_string(), task_list.clone());
        let _ = writer
            .write_mcp_tool_result(
                "list_tasks",
                true,
                &format!("{} tasks returned", task_list.lines().count() - 1),
            )
            .await;

        response
    }

    /// Handle the `skip_tasks` tool call.
    ///
    /// Marks tasks as skipped if they are in `NotStarted` status.
    async fn handle_skip_tasks(
        &self,
        task_ids: &[String],
        reason: Option<&String>,
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        use crate::tasks::TaskStatus;

        let depth = self.current_scope.depth();
        let task_count = task_ids.len();
        let reason_str = reason.map_or("No reason provided", String::as_str);
        let _ = writer
            .write_mcp_tool_call("skip_tasks", &format!("{task_count} task(s): {reason_str}"))
            .await;
        tracing::info!(
            "[L{}] ⏭️ skip_tasks({} tasks, reason={:?})",
            depth,
            task_count,
            reason
        );

        // Track results for each task
        let mut skipped_tasks: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        // Validate and update task statuses
        {
            let mut tm = self.task_manager.write().await;
            for task_id in task_ids {
                if let Some(task) = tm.get_by_id_or_name(task_id) {
                    let actual_id = task.id.clone();
                    let task_name = task.name.clone();

                    match &task.status {
                        TaskStatus::NotStarted => {
                            let status = TaskStatus::Skipped {
                                reason: reason
                                    .cloned()
                                    .unwrap_or_else(|| "No reason provided".to_string()),
                            };
                            tm.update_status(&actual_id, &status);
                            tracing::info!(
                                "⏭️ Task '{}' ({}) marked as skipped",
                                actual_id,
                                task_name
                            );
                            skipped_tasks.push(format!("{actual_id}:{task_name}"));
                        }
                        TaskStatus::InProgress { .. } => {
                            let msg = format!(
                                "Task '{task_id}' is currently in progress and cannot be skipped"
                            );
                            tracing::warn!("⚠️ {}", msg);
                            errors.push(msg);
                        }
                        TaskStatus::Complete { .. } => {
                            let msg = format!(
                                "Task '{task_id}' is already complete and cannot be skipped"
                            );
                            tracing::warn!("⚠️ {}", msg);
                            errors.push(msg);
                        }
                        TaskStatus::Failed { .. } => {
                            let msg = format!(
                                "Task '{task_id}' has already failed and cannot be skipped"
                            );
                            tracing::warn!("⚠️ {}", msg);
                            errors.push(msg);
                        }
                        TaskStatus::Skipped { .. } => {
                            tracing::info!("⏭️ Task '{}' is already skipped", task_id);
                            skipped_tasks
                                .push(format!("{actual_id}:{task_name} (already skipped)"));
                        }
                    }
                } else {
                    let msg = format!("Task '{task_id}' not found");
                    tracing::warn!("⚠️ {}", msg);
                    errors.push(msg);
                }
            }
        }

        let task_state = self.get_task_state().await;
        let skipped_count = skipped_tasks.len();

        let (response, result_msg) = if errors.is_empty() {
            let summary = format!(
                "Skipped {} task(s): [{}]",
                skipped_count,
                skipped_tasks.join(", ")
            );
            (
                ToolResponse::success(request_id.to_string(), summary.clone())
                    .with_task_state(task_state),
                format!("✓ {summary}"),
            )
        } else if skipped_count > 0 {
            let summary = format!(
                "Skipped {} task(s): [{}]. Errors: {}",
                skipped_count,
                skipped_tasks.join(", "),
                errors.join("; ")
            );
            (
                ToolResponse::success(request_id.to_string(), summary.clone())
                    .with_task_state(task_state),
                format!("⚠️ {summary}"),
            )
        } else {
            let error_msg = errors.join("; ");
            (
                ToolResponse::failure(request_id.to_string(), error_msg.clone())
                    .with_task_state(task_state),
                format!("✗ Failed to skip tasks: {error_msg}"),
            )
        };

        let _ = writer
            .write_mcp_tool_result("skip_tasks", response.success, &result_msg)
            .await;

        response
    }

    /// Check for pending tasks that would block completion.
    ///
    /// Returns `Some(error_message)` if there are pending tasks, `None` if completion is allowed.
    async fn check_completion_blockers(&self) -> Option<String> {
        use crate::tasks::TaskStatus;

        let pending_tasks: Vec<(String, String)> = {
            let tm = self.task_manager.read().await;
            tm.all_tasks()
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::NotStarted))
                .map(|t| (t.id.clone(), t.name.clone()))
                .collect()
        };

        if pending_tasks.is_empty() {
            return None;
        }

        tracing::info!(
            "⚠️ Reconciliation check: {} pending task(s) remain",
            pending_tasks.len()
        );

        let task_list: Vec<String> = pending_tasks
            .iter()
            .map(|(id, name)| format!("- {id}: {name}"))
            .collect();

        Some(format!(
            "Cannot complete: {} pending task(s) remain:\n{}\n\n\
             Please either:\n\
             - Use spawn_agents to execute remaining tasks, or\n\
             - Use skip_tasks to explicitly skip tasks that are not needed",
            pending_tasks.len(),
            task_list.join("\n")
        ))
    }

    /// Validate agent specifications before spawning.
    ///
    /// Returns `Some(error_message)` if validation fails, `None` if all agents are valid.
    fn validate_agent_specs(agents: &[crate::mcp_server::AgentSpec]) -> Option<String> {
        for agent in agents {
            let role = agent.role.as_deref().unwrap_or("implementer");
            if role.to_lowercase() == "custom" && agent.prompt.is_none() {
                return Some("Custom agent requires 'prompt' field".to_string());
            }
            if agent.task.is_none() && agent.task_id.is_none() {
                return Some("Agent requires either 'task' or 'task_id' field".to_string());
            }
        }
        None
    }

    /// Process results from concurrent agent execution.
    ///
    /// Returns `(all_success, combined_summary)`.
    async fn process_concurrent_results(
        &self,
        results: &[crate::app::spawn_config::AgentResult],
    ) -> (bool, String) {
        let mut summaries = Vec::new();
        let mut all_success = true;
        let mut all_suggested_task_ids = Vec::new();

        for result in results {
            let status = if result.success { "✓" } else { "✗" };
            let msg = result.message.as_deref().unwrap_or("No message");
            let role = &result.role;
            let log_msg = truncate_for_log(msg, 80);
            summaries.push(format!("[{role}] {status} {log_msg}"));

            if !result.success {
                all_success = false;
            }

            all_suggested_task_ids.extend(result.suggested_task_ids.clone());
        }

        let success_count = results.iter().filter(|r| r.success).count();
        if all_success {
            tracing::info!(
                "✅ spawn_agents complete: {}/{} agents succeeded (concurrent mode)",
                success_count,
                results.len()
            );
        } else {
            tracing::warn!(
                "⚠️ spawn_agents complete: {}/{} agents succeeded, {} failed (concurrent mode)",
                success_count,
                results.len(),
                results.len() - success_count
            );
        }

        let combined = self
            .build_summary_with_notes_and_suggested_tasks(summaries, all_suggested_task_ids)
            .await;

        (all_success, combined)
    }

    /// Process results from sequential agent execution.
    ///
    /// Returns `(all_success, combined_summary)`.
    async fn process_sequential_results(
        &self,
        summaries: Vec<String>,
        task_ids_before: &std::collections::HashSet<String>,
    ) -> (bool, String) {
        let all_success = summaries.iter().all(|s| s.contains("✓"));
        let success_count = summaries.iter().filter(|s| s.contains("✓")).count();
        let total = summaries.len();

        if all_success {
            tracing::info!(
                "✅ spawn_agents complete: {}/{} agents succeeded (sequential mode)",
                success_count,
                total
            );
        } else {
            tracing::warn!(
                "⚠️ spawn_agents complete: {}/{} agents succeeded, {} failed (sequential mode)",
                success_count,
                total,
                total - success_count
            );
        }

        // Find tasks created during sequential spawning
        let suggested_task_ids: Vec<String> = {
            let tm = self.task_manager.read().await;
            tm.list_task_ids()
                .into_iter()
                .filter(|id| !task_ids_before.contains(id))
                .collect()
        };

        let combined = self
            .build_summary_with_notes_and_suggested_tasks(summaries, suggested_task_ids)
            .await;

        (all_success, combined)
    }

    /// Resolve the orchestrator model configuration to an actual model string.
    fn resolve_orchestrator_model(&self) -> Result<String> {
        // Resolve fallback chain to a tier
        let tier = self
            .model_config
            .orchestrator_model
            .resolve(&self.model_config.available_tiers)?;

        // Orchestrator doesn't use auto-resolution (no complexity hint)
        // Convert tier to backend-specific model string
        self.backend.resolve_tier(tier)
    }
}
