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
use crate::logging::AgentWriter;
use crate::mcp_server::{TaskStateInfo, ToolCall, ToolResponse};
use crate::types::TaskResult;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

impl App {
    /// Spawn an orchestrator agent.
    /// Returns (`session_id`, `full_prompt`) so the prompt can be logged.
    #[tracing::instrument(skip(self, prompt), fields(agent_type = "orchestrator", session_id))]
    pub(crate) async fn spawn_orchestrator(&mut self, prompt: &str) -> Result<(String, String)> {
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
        let response = self
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
            .session_prompt(&response.session_id, &full_prompt)
            .await?;

        Ok((response.session_id, full_prompt))
    }

    /// Create an orchestrator session with retry logic.
    ///
    /// This handles transient MCP server startup errors by retrying the session
    /// creation with exponential backoff.
    async fn create_orchestrator_session_with_retry(
        &mut self,
        mcp_servers: Vec<Value>,
        cwd: &str,
    ) -> Result<crate::acp::SessionNewResponse> {
        let retry_config = RetryConfig::from_env();
        let model = self.model_config.orchestrator_model.as_str().to_string();
        let mut attempt = 0;
        let mut delay = retry_config.initial_delay;

        loop {
            attempt += 1;

            match self
                .acp_orchestrator
                .session_new(&model, mcp_servers.clone(), cwd)
                .await
            {
                Ok(response) => {
                    if attempt > 1 {
                        tracing::info!(
                            "🔄 Orchestrator session_new succeeded on attempt {}/{}",
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
                            "⚠️ Orchestrator session_new failed (attempt {}/{}): {}. Retrying in {:?}...",
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
                            "❌ Orchestrator session_new failed after {} attempt(s) ({}): {:#}",
                            attempt,
                            reason,
                            e
                        );
                        return Err(e).context(format!(
                            "Orchestrator session_new failed after {} attempt(s)",
                            attempt
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
        let (session_id, full_prompt) = match self.spawn_orchestrator(prompt).await {
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
                            // Resolve task description from task_id if needed
                            let resolved_task = if let Some(ref tid) = task_id {
                                let tm = self.task_manager.read().await;
                                // Use get_by_id_or_name for flexible lookup (supports both IDs and names)
                                tm.get_by_id_or_name(tid)
                                    .map_or_else(|| {
                                        tracing::warn!(
                                            "Task '{}' not found in TaskManager. Available: {:?}",
                                            tid,
                                            tm.list_task_ids()
                                        );
                                        format!("(task {tid} not found)")
                                    }, |t| t.description.clone())
                            } else {
                                task.clone().unwrap_or_else(|| "(no task)".to_string())
                            };

                            // Log the MCP tool call to orchestrator log
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

                            eprintln!("[orchestrator] Before calling handle_decompose_with_response");
                            self.tool_rx = Some(tool_rx);
                            let response = self
                                .handle_decompose_with_response(&resolved_task, &request.request_id)
                                .await;
                            eprintln!("[orchestrator] After handle_decompose_with_response: success={}", response.success);
                            tool_rx = self
                                .tool_rx
                                .take()
                                .context("Tool receiver lost during decompose")?;

                            // Log the result to orchestrator log
                            let _ = writer.write_mcp_tool_result(
                                "decompose",
                                response.success,
                                &truncate_for_log(&response.summary, 100)
                            ).await;

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

                            // Validate custom agents have required fields
                            let mut validation_error: Option<String> = None;
                            for agent in agents {
                                let role = agent.role.as_deref().unwrap_or("implementer");
                                if role.to_lowercase() == "custom" && agent.prompt.is_none() {
                                    validation_error =
                                        Some("Custom agent requires 'prompt' field".to_string());
                                    break;
                                }
                                // tools is optional - if not provided, all default tools are enabled
                                // Validate that either task or task_id is provided
                                if agent.task.is_none() && agent.task_id.is_none() {
                                    validation_error = Some(
                                        "Agent requires either 'task' or 'task_id' field".to_string(),
                                    );
                                    break;
                                }
                            }

                            if let Some(error_msg) = validation_error {
                                let response = ToolResponse::failure(
                                    request.request_id.clone(),
                                    error_msg,
                                );
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
                                tracing::info!("🚀 Running {} agents in CONCURRENT mode", agents.len());

                                let results = self.spawn_agents_concurrent(agents.clone(), *wait).await;

                                let mut summaries = Vec::new();
                                let mut all_success = true;
                                let mut all_suggested_task_ids = Vec::new();

                                for result in &results {
                                    let status = if result.success { "✓" } else { "✗" };
                                    let msg = result.message.as_deref().unwrap_or("No message");
                                    summaries.push(format!("[{}] {} {}", result.role, status, truncate_for_log(msg, 80)));

                                    if !result.success {
                                        all_success = false;
                                    }

                                    // Collect suggested task IDs
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

                                // Include notes and suggested tasks in the response
                                let combined = self.build_summary_with_notes_and_suggested_tasks(
                                    summaries,
                                    all_suggested_task_ids
                                ).await;

                                (all_success, combined)
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

                                let mut all_success = true;
                                let mut summaries = Vec::new();
                                let agent_len = agents.len();

                                for agent in agents {
                                    // Resolve task from task_id if needed
                                    let role =
                                        agent.role.as_deref().unwrap_or("implementer").to_string();
                                    let task = if let Some(ref tid) = agent.task_id {
                                        // Look up task description from task manager
                                        // Use get_by_id_or_name for flexible lookup
                                        let tm = self.task_manager.read().await;
                                        tm.get_by_id_or_name(tid)
                                            .map_or_else(|| {
                                                tracing::warn!(
                                                    "Task '{}' not found in TaskManager. Available: {:?}",
                                                    tid,
                                                    tm.list_task_ids()
                                                );
                                                format!("(task {tid} not found)")
                                            }, |t| t.description.clone())
                                    } else {
                                        agent
                                            .task
                                            .clone()
                                            .unwrap_or_else(|| "(no task)".to_string())
                                    };

                                    // Restore tool_rx before spawning (it needs to receive complete signal)
                                    self.tool_rx = Some(tool_rx);
                                    let impl_response = self
                                        .handle_implement_with_response(&task, &request.request_id)
                                        .await;
                                    tool_rx = self
                                        .tool_rx
                                        .take()
                                        .context("Tool receiver lost during spawn_agents")?;

                                    let status = if impl_response.success { "✓" } else { "✗" };
                                    summaries.push(format!(
                                        "[{}] {} {}",
                                        role,
                                        status,
                                        truncate_for_log(&impl_response.summary, 80)
                                    ));

                                    if !impl_response.success {
                                        all_success = false;
                                    }
                                }

                                // Log overall completion
                                let success_count =
                                    summaries.iter().filter(|s| s.contains("✓")).count();
                                if all_success {
                                    tracing::info!(
                                        "✅ spawn_agents complete: {}/{} agents succeeded (sequential mode)",
                                        success_count,
                                        agent_len
                                    );
                                } else {
                                    tracing::warn!(
                                        "⚠️ spawn_agents complete: {}/{} agents succeeded, {} failed (sequential mode)",
                                        success_count,
                                        agent_len,
                                        agent_len - success_count
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

                                // Include notes and suggested tasks in the response
                                let combined = self.build_summary_with_notes_and_suggested_tasks(
                                    summaries,
                                    suggested_task_ids
                                ).await;

                                (all_success, combined)
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
                            let success_str = if *success { "success=true" } else { "success=false" };
                            let msg_preview = message.as_deref().map(|m| truncate_for_log(m, 50)).unwrap_or_default();
                            let _ = writer.write_mcp_tool_call("complete", &format!("{success_str}, {msg_preview}")).await;
                            tracing::info!("[L{}] 🏁 complete({success_str})", depth);

                            // Task reconciliation: ensure all tasks have a terminal status
                            // before allowing successful completion.
                            // See module-level docs and src/tasks/mod.rs for details.
                            if *success {
                                use crate::tasks::TaskStatus;

                                let pending_tasks: Vec<(String, String)> = {
                                    let tm = self.task_manager.read().await;
                                    tm.all_tasks()
                                        .iter()
                                        .filter(|t| matches!(t.status, TaskStatus::NotStarted))
                                        .map(|t| (t.id.clone(), t.name.clone()))
                                        .collect()
                                };

                                if !pending_tasks.is_empty() {
                                    // Reject completion: orchestrator must address pending tasks
                                    tracing::info!(
                                        "⚠️ Reconciliation check: {} pending task(s) remain",
                                        pending_tasks.len()
                                    );

                                    // Build formatted list of pending tasks
                                    let task_list: Vec<String> = pending_tasks
                                        .iter()
                                        .map(|(id, name)| format!("- {id}: {name}"))
                                        .collect();

                                    let pending_msg = format!(
                                        "Cannot complete: {} pending task(s) remain:\n{}\n\n\
                                        Please either:\n\
                                        - Use spawn_agents to execute remaining tasks, or\n\
                                        - Use skip_tasks to explicitly skip tasks that are not needed",
                                        pending_tasks.len(),
                                        task_list.join("\n")
                                    );

                                    let response = ToolResponse::success(
                                        request.request_id.clone(),
                                        pending_msg.clone(),
                                    );
                                    let _ = writer.write_mcp_tool_result(
                                        "complete",
                                        false,
                                        &format!("Rejected: {} pending tasks", pending_tasks.len()),
                                    ).await;
                                    let _ = response_tx.send(response);

                                    // Continue loop - do NOT break, allow further tool calls
                                    continue;
                                }
                            }

                            // No pending tasks or success=false - proceed with normal completion
                            // Log the completion message
                            if let Some(msg) = &message {
                                let _ = writer.write_result(msg).await;
                            }

                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                message.clone().unwrap_or_else(|| "Orchestration complete".to_string()),
                            );
                            let _ = writer.write_mcp_tool_result(
                                "complete",
                                *success,
                                if *success { "Accepted" } else { "Task marked as failed" },
                            ).await;
                            let _ = response_tx.send(response);

                            // Brief drain for any final messages (500ms max)
                            let drain_result = tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                self.drain_orchestrator_messages(&session_id, writer),
                            ).await;

                            if drain_result.is_err() {
                                tracing::trace!("Drain timeout, proceeding");
                            }

                            break TaskResult { success: *success, message: message.clone() };
                        }
                        ToolCall::CreateTask { name, description, dependencies } => {
                            // Orchestrator can create tasks dynamically
                            let depth = self.current_scope.depth();
                            let _ = writer.write_mcp_tool_call("create_task", &format!("{name}")).await;

                            let task_id = {
                                let mut tm = self.task_manager.write().await;
                                tm.create(name, description, dependencies.clone())
                            };

                            tracing::info!(
                                "[L{}] 📋 Orchestrator created task [{}]: {}",
                                depth, task_id, name
                            );

                            // Fetch task state for context-aware response
                            let task_state = self.get_task_state().await;

                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                format!("Task '{name}' created with ID: {task_id}. Remember to use spawn_agents with task_id=\"{task_id}\" to execute it."),
                            ).with_task_state(task_state);
                            let _ = writer.write_mcp_tool_result("create_task", true, &format!("Created {task_id}")).await;
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
                            // Skip tasks that are no longer needed. Only tasks in NotStarted
                            // status can be skipped. See src/tasks/mod.rs for status transitions.
                            use crate::tasks::TaskStatus;

                            let depth = self.current_scope.depth();
                            let task_count = task_ids.len();
                            let reason_str = reason.as_deref().unwrap_or("No reason provided");
                            let _ = writer.write_mcp_tool_call(
                                "skip_tasks",
                                &format!("{task_count} task(s): {reason_str}"),
                            ).await;
                            tracing::info!(
                                "[L{}] ⏭️ skip_tasks({} tasks, reason={:?})",
                                depth, task_count, reason
                            );

                            // Track results for each task
                            let mut skipped_tasks: Vec<String> = Vec::new();
                            let mut errors: Vec<String> = Vec::new();

                            // Validate and update task statuses
                            {
                                let mut tm = self.task_manager.write().await;
                                for task_id in task_ids {
                                    // Look up the task (supports both ID and name lookup)
                                    if let Some(task) = tm.get_by_id_or_name(task_id) {
                                        let actual_id = task.id.clone();
                                        let task_name = task.name.clone();

                                        // Validate task is in NotStarted status
                                        match &task.status {
                                            TaskStatus::NotStarted => {
                                                // Task can be skipped
                                                let status = TaskStatus::Skipped {
                                                    reason: reason.clone().unwrap_or_else(|| "No reason provided".to_string()),
                                                };
                                                tm.update_status(&actual_id, &status);
                                                tracing::info!("⏭️ Task '{}' ({}) marked as skipped", actual_id, task_name);
                                                skipped_tasks.push(format!("{actual_id}:{task_name}"));
                                            }
                                            TaskStatus::InProgress { .. } => {
                                                let msg = format!("Task '{task_id}' is currently in progress and cannot be skipped");
                                                tracing::warn!("⚠️ {}", msg);
                                                errors.push(msg);
                                            }
                                            TaskStatus::Complete { .. } => {
                                                let msg = format!("Task '{task_id}' is already complete and cannot be skipped");
                                                tracing::warn!("⚠️ {}", msg);
                                                errors.push(msg);
                                            }
                                            TaskStatus::Failed { .. } => {
                                                let msg = format!("Task '{task_id}' has already failed and cannot be skipped");
                                                tracing::warn!("⚠️ {}", msg);
                                                errors.push(msg);
                                            }
                                            TaskStatus::Skipped { .. } => {
                                                // Already skipped, treat as success but note it
                                                tracing::info!("⏭️ Task '{}' is already skipped", task_id);
                                                skipped_tasks.push(format!("{actual_id}:{task_name} (already skipped)"));
                                            }
                                        }
                                    } else {
                                        let msg = format!("Task '{task_id}' not found");
                                        tracing::warn!("⚠️ {}", msg);
                                        errors.push(msg);
                                    }
                                }
                            }

                            // Fetch task state for context-aware response
                            let task_state = self.get_task_state().await;

                            // Build response based on results
                            let skipped_count = skipped_tasks.len();
                            let (response, result_msg) = if errors.is_empty() {
                                // All tasks skipped successfully
                                let summary = format!(
                                    "Skipped {} task(s): [{}]",
                                    skipped_count,
                                    skipped_tasks.join(", ")
                                );
                                (
                                    ToolResponse::success(request.request_id.clone(), summary.clone())
                                        .with_task_state(task_state),
                                    format!("✓ {summary}"),
                                )
                            } else if skipped_count > 0 {
                                // Partial success: some skipped, some errors
                                let summary = format!(
                                    "Skipped {} task(s): [{}]. Errors: {}",
                                    skipped_count,
                                    skipped_tasks.join(", "),
                                    errors.join("; ")
                                );
                                (
                                    ToolResponse::success(request.request_id.clone(), summary.clone())
                                        .with_task_state(task_state),
                                    format!("⚠️ {summary}"),
                                )
                            } else {
                                // All failed
                                let error_msg = errors.join("; ");
                                (
                                    ToolResponse::failure(request.request_id.clone(), error_msg.clone())
                                        .with_task_state(task_state),
                                    format!("✗ Failed to skip tasks: {error_msg}"),
                                )
                            };

                            let _ = writer.write_mcp_tool_result(
                                "skip_tasks",
                                response.success,
                                &result_msg,
                            ).await;

                            let _ = response_tx.send(response);
                        }
                        ToolCall::ListTasks { ref status_filter } => {
                            use crate::tasks::TaskStatus;

                            let depth = self.current_scope.depth();
                            let filter = status_filter.as_deref().unwrap_or("all");
                            let _ = writer.write_mcp_tool_call("list_tasks", &format!("filter={filter}")).await;
                            tracing::info!("[L{}] 📋 list_tasks(filter={filter})", depth);

                            // Build task list response
                            let task_list = {
                                let tm = self.task_manager.read().await;
                                let tasks = tm.all_tasks();

                                // Filter by status if specified
                                let filtered: Vec<_> = tasks.into_iter().filter(|t| {
                                    match filter {
                                        "all" => true,
                                        "pending" => matches!(t.status, TaskStatus::NotStarted),
                                        "in_progress" => matches!(t.status, TaskStatus::InProgress { .. }),
                                        "completed" => matches!(t.status, TaskStatus::Complete { .. }),
                                        "failed" => matches!(t.status, TaskStatus::Failed { .. }),
                                        "skipped" => matches!(t.status, TaskStatus::Skipped { .. }),
                                        _ => true,
                                    }
                                }).collect();

                                // Format task list
                                let mut lines = Vec::new();
                                lines.push(format!("## Tasks ({} total, filter={})\n", filtered.len(), filter));
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

                            let response = ToolResponse::success(request.request_id.clone(), task_list.clone());
                            let _ = writer.write_mcp_tool_result("list_tasks", true, &format!("{} tasks returned", task_list.lines().count() - 1)).await;
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
    /// Creates a `TaskStateInfo` snapshot using the TaskManager helper methods.
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
}
