//! Orchestrator agent handling.

use super::types::{truncate_for_log, ToolMessage, ORCHESTRATOR_PROMPT};
use super::App;
use crate::logging::AgentWriter;
use crate::mcp_server::{ToolCall, ToolResponse};
use crate::types::TaskResult;
use anyhow::{Context, Result};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;

impl App {
    /// Spawn an orchestrator agent.
    /// Returns (`session_id`, `full_prompt`) so the prompt can be logged.
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

        let response = self
            .acp_orchestrator
            .session_new(
                self.model_config.orchestrator_model.as_str(),
                mcp_servers,
                &cwd,
            )
            .await?;

        // Get the goal from task manager
        let goal = {
            let tm = self.task_manager.read().await;
            tm.format_goal()
        };

        let full_prompt = ORCHESTRATOR_PROMPT
            .replace("{goal}", &goal)
            .replace("{plan}", prompt);
        tracing::debug!("🎭 Orchestrator prompt:\n{}", full_prompt);

        self.acp_orchestrator
            .session_prompt(&response.session_id, &full_prompt)
            .await?;

        Ok((response.session_id, full_prompt))
    }

    /// Spawn and run an orchestrator agent with logging.
    pub(crate) fn run_orchestrator_with_writer<'a>(
        &'a mut self,
        prompt: &'a str,
        writer: &'a mut AgentWriter,
    ) -> Pin<Box<dyn Future<Output = Result<TaskResult>> + 'a>> {
        Box::pin(async move { self.run_orchestrator_with_writer_impl(prompt, writer).await })
    }

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
                    tracing::warn!("Failed to write spawn error to orchestrator log: {}", write_err);
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

        // Handle tool calls from MCP server
        let result = loop {
            tokio::select! {
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

                            self.tool_rx = Some(tool_rx);
                            let response = self
                                .handle_decompose_with_response(&resolved_task, &request.request_id)
                                .await;
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

                                let results = self.spawn_agents_concurrent(agents.clone(), wait.clone()).await;

                                let mut summaries = Vec::new();
                                let mut all_success = true;

                                for result in &results {
                                    let status = if result.success { "✓" } else { "✗" };
                                    let msg = result.message.as_deref().unwrap_or("No message");
                                    summaries.push(format!("[{}] {} {}", result.role, status, truncate_for_log(msg, 80)));

                                    if !result.success {
                                        all_success = false;
                                    }
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

                                // Include notes from agents in the response
                                let mut combined = summaries.join("\n");
                                {
                                    let tm = self.task_manager.read().await;
                                    if let Some(notes_section) = tm.format_notes() {
                                        combined.push_str("\n\n");
                                        combined.push_str(&notes_section);
                                    }
                                }

                                (all_success, combined)
                            } else {
                                // Sequential mode: spawn agents one at a time
                                // This uses handle_implement_with_response which works with mock clients
                                tracing::debug!(
                                    "Running {} agents in SEQUENTIAL mode (mock/test)",
                                    agents.len()
                                );

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

                                // Include notes from agents in the response
                                let mut combined = summaries.join("\n");
                                {
                                    let tm = self.task_manager.read().await;
                                    if let Some(notes_section) = tm.format_notes() {
                                        combined.push_str("\n\n");
                                        combined.push_str(&notes_section);
                                    }
                                }

                                (all_success, combined)
                            };

                            let response = if all_success {
                                ToolResponse::success(request.request_id.clone(), combined_summary)
                            } else {
                                ToolResponse::failure(request.request_id.clone(), combined_summary)
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
                            // Log the completion message
                            if let Some(msg) = &message {
                                let _ = writer.write_result(msg).await;
                            }

                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                message.clone().unwrap_or_else(|| "Orchestration complete".to_string()),
                            );
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
                            let task_id = {
                                let mut tm = self.task_manager.write().await;
                                tm.create(name, description, dependencies.clone())
                            };

                            tracing::info!(
                                "📋 Orchestrator created task [{}]: {}",
                                task_id, name
                            );

                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                format!("Task '{name}' created with ID: {task_id}"),
                            );
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
        Ok(result)
    }
}
