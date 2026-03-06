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
        // Use unique name "villalobos-orchestrator" to prevent caching issues
        let mcp_servers = vec![json!({
            "name": "villalobos-orchestrator",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server"],
            "env": [{
                "name": "VILLALOBOS_SOCKET",
                "value": socket_path
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

        let full_prompt = format!(
            "{}\n\n## PLAN TO EXECUTE\n\nThe following plan was created by a planner agent. Your job is to execute it by calling spawn_agents() or decompose() for each task. Do NOT re-plan or re-analyze. Just execute the tasks in order.\n\n{}",
            ORCHESTRATOR_PROMPT.trim(),
            prompt
        );
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
        let (session_id, full_prompt) = self.spawn_orchestrator(prompt).await?;
        writer.set_session_id(session_id.clone());
        // Use the plan as the task description, but log the full prompt for debugging
        if let Err(e) = writer.write_header_with_prompt(prompt, &full_prompt).await {
            tracing::warn!("Failed to write orchestrator header: {}", e);
        }

        // Take tool_rx for this orchestrator run, but restore it when done
        let mut tool_rx = self.tool_rx.take().context("Tool receiver not set up")?;

        // Handle tool calls from MCP server
        let result = loop {
            tokio::select! {
                Some(tool_msg) = tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;
                    tracing::debug!("📨 Orchestrator MCP tool call received: {:?}", request.tool_call.tool_type());

                    match &request.tool_call {
                        ToolCall::Decompose { task } => {
                            // Log the MCP tool call to orchestrator log
                            let tool_desc = format!("decompose_villalobos: {}", truncate_for_log(task, 100));
                            let _ = writer.write_tool_call(&tool_desc).await;
                            tracing::info!("🔄 MCP tool call: decompose({})", truncate_for_log(task, 80));

                            self.tool_rx = Some(tool_rx);
                            let response = self.handle_decompose_with_response(task, &request.request_id).await;
                            tool_rx = self.tool_rx.take().context("Tool receiver lost during decompose")?;

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
                            let tool_desc = format!("spawn_agents: {agent_count} agent(s), wait={wait:?}");
                            let _ = writer.write_tool_call(&tool_desc).await;
                            tracing::info!("🚀 MCP tool call: spawn_agents({agent_count} agents, wait={wait:?})");

                            // Log each agent being spawned
                            for (i, agent) in agents.iter().enumerate() {
                                tracing::info!(
                                    "  📋 Agent {}/{}: [{}] {}",
                                    i + 1,
                                    agent_count,
                                    agent.role,
                                    truncate_for_log(&agent.task, 60)
                                );
                            }

                            // Validate custom agents have required fields
                            let mut validation_error: Option<String> = None;
                            for agent in agents {
                                if agent.role.to_lowercase() == "custom" {
                                    if agent.prompt.is_none() {
                                        validation_error = Some("Custom agent requires 'prompt' field".to_string());
                                        break;
                                    }
                                    if agent.tools.is_none() {
                                        validation_error = Some("Custom agent requires 'tools' whitelist".to_string());
                                        break;
                                    }
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

                            // NOTE: Concurrent mode is currently disabled because tool calls from
                            // worker agents are not properly routed. The concurrent implementation
                            // (spawn_agents_concurrent) only waits for session_finished messages
                            // but doesn't handle tool calls (e.g., the `complete` call workers must
                            // make). This causes workers to hang waiting for tool responses that
                            // never come, resulting in 30-minute timeouts.
                            //
                            // TODO: To enable concurrent mode, we need to either:
                            // 1. Add session IDs to tool calls so they can be routed to the correct
                            //    handler, OR
                            // 2. Have each concurrent agent spawn its own MCP server instance
                            //
                            // For now, we always use sequential mode which properly handles tool
                            // calls through the orchestrator's tool_rx channel.
                            let _ = self.router_active; // silence unused warning
                            let (all_success, combined_summary) = {
                                // Sequential mode: spawn agents one at a time
                                // This uses handle_implement_with_response which works with mock clients
                                let mut all_success = true;
                                let mut summaries = Vec::new();

                                for agent in agents {
                                    // Restore tool_rx before spawning (it needs to receive complete signal)
                                    self.tool_rx = Some(tool_rx);
                                    let impl_response = self.handle_implement_with_response(&agent.task, &request.request_id).await;
                                    tool_rx = self.tool_rx.take().context("Tool receiver lost during spawn_agents")?;

                                    let status = if impl_response.success { "✓" } else { "✗" };
                                    summaries.push(format!("[{}] {} {}", agent.role, status, truncate_for_log(&impl_response.summary, 80)));

                                    if !impl_response.success {
                                        all_success = false;
                                    }
                                }

                                // Log overall completion
                                let success_count = summaries.iter().filter(|s| s.contains("✓")).count();
                                if all_success {
                                    tracing::info!(
                                        "✅ spawn_agents complete: {}/{} agents succeeded (sequential mode)",
                                        success_count,
                                        agents.len()
                                    );
                                } else {
                                    tracing::warn!(
                                        "⚠️ spawn_agents complete: {}/{} agents succeeded, {} failed (sequential mode)",
                                        success_count,
                                        agents.len(),
                                        agents.len() - success_count
                                    );
                                }

                                (all_success, summaries.join("\n"))
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
                        ToolCall::Complete { success, message } => {
                            // Log the completion message
                            if let Some(msg) = &message {
                                let _ = writer.write_result(msg).await;
                            }

                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                message.clone().unwrap_or_else(|| "Orchestration complete".to_string()),
                            );
                            let _ = response_tx.send(response);

                            // Drain remaining orchestrator messages before returning
                            tracing::debug!("⏳ Draining remaining orchestrator messages for session {}", session_id);
                            let drain_result = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                self.drain_orchestrator_messages(&session_id, writer),
                            ).await;

                            if drain_result.is_err() {
                                tracing::debug!("⚠️ Timeout waiting for orchestrator session_finished, proceeding anyway");
                            }

                            break TaskResult { success: *success, message: message.clone() };
                        }
                        ToolCall::WritePlan { .. } => {
                            // WritePlan is only for planner agents, not orchestrator
                            tracing::warn!("Orchestrator received unexpected WritePlan call");
                            let response = ToolResponse::failure(
                                request.request_id.clone(),
                                "write_plan is not available to orchestrator agents".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                        ToolCall::CreateTask { .. } => {
                            // CreateTask is only for planner agents, not orchestrator
                            tracing::warn!("Orchestrator received unexpected CreateTask call");
                            let response = ToolResponse::failure(
                                request.request_id.clone(),
                                "create_task is not available to orchestrator agents".to_string(),
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
