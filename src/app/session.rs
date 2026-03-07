//! Session handling - waiting for agent sessions and processing ACP messages.

use super::types::ToolMessage;
use super::App;
use crate::error::{OrchestratorError, TimeoutOperation};
use crate::logging::AgentWriter;
use crate::mcp_server::{ToolCall, ToolResponse};
use crate::types::SessionOutput;
use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::mpsc;

impl App {
    /// Handle ACP messages and stream agent output.
    #[tracing::instrument(skip(self, msg), fields(agent_type = %agent_type))]
    pub(crate) async fn handle_acp_message(&self, msg: &serde_json::Value, agent_type: &str) {
        if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
            if method == "session/update" {
                if let Some(params) = msg.get("params") {
                    if let Some(update) = params.get("update") {
                        if let Some(session_update) =
                            update.get("sessionUpdate").and_then(|v| v.as_str())
                        {
                            match session_update {
                                "tool_call" => {
                                    if let Some(title) =
                                        update.get("title").and_then(|t| t.as_str())
                                    {
                                        tracing::info!("🔧 {} tool call: {}", agent_type, title);
                                    }
                                }
                                "tool_result" => {
                                    let title = update
                                        .get("title")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("unknown");
                                    let is_error = update
                                        .get("isError")
                                        .and_then(serde_json::Value::as_bool)
                                        .unwrap_or(false);
                                    if is_error {
                                        let content = update
                                            .get("content")
                                            .and_then(|c| c.get("text").and_then(|t| t.as_str()))
                                            .unwrap_or("no error message");
                                        tracing::error!(
                                            "❌ {} tool failed: {} - {}",
                                            agent_type,
                                            title,
                                            content
                                        );
                                    }
                                }
                                // agent_message_chunk, agent_thought_chunk, and others are logged elsewhere or ignored
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    /// Register a session with the router and get its per-session receiver.
    ///
    /// This should be called before waiting for session output.
    /// The session should be unregistered after completion via `unregister_session`.
    #[tracing::instrument(skip(self), fields(session_id = %session_id))]
    pub(crate) async fn register_session(&self, session_id: &str) -> mpsc::Receiver<Value> {
        let mut router = self.session_router.write().await;
        let rx = router.register(session_id);
        tracing::debug!("Registered session {} with router", session_id);
        rx
    }

    /// Unregister a session from the router.
    ///
    /// This should be called after the session completes to clean up.
    #[tracing::instrument(skip(self), fields(session_id = %session_id))]
    pub(crate) async fn unregister_session(&self, session_id: &str) {
        let mut router = self.session_router.write().await;
        router.unregister(session_id);
        tracing::debug!("Unregistered session {} from router", session_id);
    }

    /// Wait for a session to complete, collecting all message output.
    /// This is the unified wait function for all agent types (planner, implementer, etc.)
    #[tracing::instrument(skip(self, writer), fields(session_id = %session_id))]
    pub(crate) async fn wait_for_session_output(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> Result<SessionOutput, OrchestratorError> {
        let timeout_duration = self.timeout_config.session_timeout;

        // If router is active, use routed mode with per-session receiver
        // Otherwise, use direct mode (for tests with mock clients)
        if self.router_active {
            // Register the session with the router to get a per-session receiver
            let session_rx = self.register_session(session_id).await;

            let result = tokio::time::timeout(
                timeout_duration,
                self.wait_for_session_output_routed(session_id, writer, session_rx),
            )
            .await;

            // Always unregister the session after completion (success or failure)
            self.unregister_session(session_id).await;

            match result {
                Ok(inner_result) => inner_result.map_err(OrchestratorError::from),
                Err(_elapsed) => {
                    tracing::error!(
                        "⏰ Timeout waiting for session after {:?} (session: {})",
                        timeout_duration,
                        session_id
                    );
                    Err(OrchestratorError::Timeout {
                        operation: TimeoutOperation::WaitForSession,
                        duration: timeout_duration,
                        context: Some(format!("session_id: {session_id}")),
                    })
                }
            }
        } else {
            // Direct mode: call acp_worker.recv() directly (for tests)
            match tokio::time::timeout(
                timeout_duration,
                self.wait_for_session_output_direct(session_id, writer),
            )
            .await
            {
                Ok(result) => result.map_err(OrchestratorError::from),
                Err(_elapsed) => {
                    tracing::error!(
                        "⏰ Timeout waiting for session after {:?} (session: {})",
                        timeout_duration,
                        session_id
                    );
                    Err(OrchestratorError::Timeout {
                        operation: TimeoutOperation::WaitForSession,
                        duration: timeout_duration,
                        context: Some(format!("session_id: {session_id}")),
                    })
                }
            }
        }
    }

    /// Wait for session output using routed mode (per-session receiver from router).
    #[allow(clippy::ignored_unit_patterns)] // `update` pattern is () when tui feature disabled
    #[tracing::instrument(skip(self, writer, session_rx), fields(session_id = %session_id, mode = "routed"))]
    async fn wait_for_session_output_routed(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
        mut session_rx: mpsc::Receiver<Value>,
    ) -> Result<SessionOutput> {
        tracing::debug!("⏳ Waiting for session (routed): {}", session_id);
        let mut output = SessionOutput::new();
        let mut seen_unhandled: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let mut tool_rx = self.tool_rx.take().context("Tool receiver not set up")?;

        // Take the config update receiver out of self so we don't hold a mutable borrow
        #[cfg(feature = "tui")]
        let mut config_update_rx = self.config_update_rx.take();

        loop {
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
                // we must process it before polling recv() again.
                biased;

                // Handle config updates from TUI (update is only used with tui feature)
                Some(update) = config_update_fut => {
                    let _ = &update; // Suppress unused variable warning when tui feature is disabled
                    #[cfg(feature = "tui")]
                    self.apply_model_config_update(&update);
                }

                Some(tool_msg) = tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;

                    match &request.tool_call {
                        ToolCall::Complete { success, message, .. } => {
                            tracing::info!(
                                "✅ Session {} signaled complete: success={}, message={:?}",
                                session_id,
                                success,
                                message
                            );

                            if let Some(msg) = message {
                                let _ = writer.write_result(msg).await;
                            }

                            let response = ToolResponse::success(
                                request.request_id,
                                message.clone().unwrap_or_else(|| "Done".to_string()),
                            );
                            let _ = response_tx.send(response);

                            // Brief drain to capture any final messages (1 second max)
                            let drain_result = tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                Self::drain_session_messages_from_rx(&mut session_rx, session_id, writer),
                            ).await;

                            if drain_result.is_err() {
                                tracing::trace!("Drain timeout, proceeding");
                            }

                            self.tool_rx = Some(tool_rx);
                            #[cfg(feature = "tui")]
                            {
                                self.config_update_rx = config_update_rx;
                            }
                            return Ok(output);
                        }
                        ToolCall::CreateTask { name, description, dependencies } => {
                            let task_id = {
                                let mut tm = self.task_manager.write().await;
                                tm.create(name, description, dependencies.clone())
                            };

                            tracing::info!(
                                "📋 Session {} created task '{}' (id: {})",
                                session_id, name, task_id
                            );

                            let response = ToolResponse::success(
                                request.request_id,
                                format!("Task '{name}' created with id {task_id}"),
                            );
                            let _ = response_tx.send(response);
                        }
                        ToolCall::SetGoal { summary, acceptance_criteria } => {
                            {
                                let mut tm = self.task_manager.write().await;
                                tm.set_goal(summary.clone(), acceptance_criteria.clone());
                            }

                            tracing::info!(
                                "📎 Session {} set goal: {}",
                                session_id, summary
                            );

                            let response = ToolResponse::success(
                                request.request_id,
                                format!("Goal set: {summary}"),
                            );
                            let _ = response_tx.send(response);
                        }
                        other => {
                            tracing::warn!("Unexpected tool call from session {}: {:?}", session_id, other);
                            let response = ToolResponse::failure(
                                request.request_id,
                                "This tool is not available. Use complete() to signal you're done.".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                    }
                }

                Some(msg) = session_rx.recv() => {
                    let finished = self.handle_worker_session_message(&msg, session_id, writer, &mut output, &mut seen_unhandled).await?;
                    if finished {
                        self.tool_rx = Some(tool_rx);
                        #[cfg(feature = "tui")]
                        {
                            self.config_update_rx = config_update_rx;
                        }
                        return Ok(output);
                    }
                }
            }
        }
    }

    /// Wait for session output using direct mode (polling ACP clients directly).
    /// Used for tests with mock clients that don't support the routing infrastructure.
    /// Polls both planner and worker clients to handle all session types.
    #[allow(clippy::ignored_unit_patterns)] // `update` pattern is () when tui feature disabled
    #[tracing::instrument(skip(self, writer), fields(session_id = %session_id, mode = "direct"))]
    async fn wait_for_session_output_direct(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> Result<SessionOutput> {
        tracing::debug!("⏳ Waiting for session (direct): {}", session_id);
        let mut output = SessionOutput::new();
        let mut seen_unhandled: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let mut tool_rx = self.tool_rx.take().context("Tool receiver not set up")?;

        // Take the config update receiver out of self so we don't hold a mutable borrow
        #[cfg(feature = "tui")]
        let mut config_update_rx = self.config_update_rx.take();

        // Track which clients have been exhausted (for mock clients)
        let mut worker_exhausted = false;
        let mut planner_exhausted = false;

        loop {
            // If both clients are exhausted and no tool messages, we're stuck
            if worker_exhausted && planner_exhausted {
                self.tool_rx = Some(tool_rx);
                #[cfg(feature = "tui")]
                {
                    self.config_update_rx = config_update_rx;
                }
                return Err(anyhow::anyhow!("No more mock updates available"));
            }

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
                // be interleaved.
                biased;

                // Handle config updates from TUI (update is only used with tui feature)
                Some(update) = config_update_fut => {
                    let _ = &update; // Suppress unused variable warning when tui feature is disabled
                    #[cfg(feature = "tui")]
                    self.apply_model_config_update(&update);
                }

                Some(tool_msg) = tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;

                    match &request.tool_call {
                        ToolCall::Complete { success, message, .. } => {
                            tracing::info!(
                                "✅ Session {} signaled complete: success={}, message={:?}",
                                session_id,
                                success,
                                message
                            );

                            if let Some(msg) = message {
                                let _ = writer.write_result(msg).await;
                            }

                            let response = ToolResponse::success(
                                request.request_id,
                                message.clone().unwrap_or_else(|| "Done".to_string()),
                            );
                            let _ = response_tx.send(response);

                            // Brief drain to capture any final messages (500ms max)
                            let drain_result = tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                self.drain_session_messages_direct(session_id, writer),
                            ).await;

                            if drain_result.is_err() {
                                tracing::trace!("Drain timeout, proceeding");
                            }

                            self.tool_rx = Some(tool_rx);
                            #[cfg(feature = "tui")]
                            {
                                self.config_update_rx = config_update_rx;
                            }
                            return Ok(output);
                        }
                        ToolCall::CreateTask { name, description, dependencies } => {
                            let task_id = {
                                let mut tm = self.task_manager.write().await;
                                tm.create(name, description, dependencies.clone())
                            };

                            tracing::info!(
                                "📋 Session {} created task '{}' (id: {})",
                                session_id, name, task_id
                            );

                            let response = ToolResponse::success(
                                request.request_id,
                                format!("Task '{name}' created with id {task_id}"),
                            );
                            let _ = response_tx.send(response);
                        }
                        ToolCall::SetGoal { summary, acceptance_criteria } => {
                            {
                                let mut tm = self.task_manager.write().await;
                                tm.set_goal(summary.clone(), acceptance_criteria.clone());
                            }

                            tracing::info!(
                                "📎 Session {} set goal: {}",
                                session_id, summary
                            );

                            let response = ToolResponse::success(
                                request.request_id,
                                format!("Goal set: {summary}"),
                            );
                            let _ = response_tx.send(response);
                        }
                        other => {
                            tracing::warn!("Unexpected tool call from session {}: {:?}", session_id, other);
                            let response = ToolResponse::failure(
                                request.request_id,
                                "This tool is not available. Use complete() to signal you're done.".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                    }
                }

                // Poll worker client (handles implementer sessions)
                msg_result = self.acp_worker.recv(), if !worker_exhausted => {
                    if let Ok(msg) = msg_result {
                        let finished = self.handle_worker_session_message(&msg, session_id, writer, &mut output, &mut seen_unhandled).await?;
                        if finished {
                            self.tool_rx = Some(tool_rx);
                            #[cfg(feature = "tui")]
                            {
                                self.config_update_rx = config_update_rx;
                            }
                            return Ok(output);
                        }
                    } else {
                        // Worker client exhausted (mock client)
                        worker_exhausted = true;
                        tracing::trace!("Worker client exhausted");
                    }
                }

                // Poll planner client (handles planner sessions)
                msg_result = self.acp_planner.recv(), if !planner_exhausted => {
                    if let Ok(msg) = msg_result {
                        let finished = self.handle_worker_session_message(&msg, session_id, writer, &mut output, &mut seen_unhandled).await?;
                        if finished {
                            self.tool_rx = Some(tool_rx);
                            #[cfg(feature = "tui")]
                            {
                                self.config_update_rx = config_update_rx;
                            }
                            return Ok(output);
                        }
                    } else {
                        // Planner client exhausted (mock client)
                        planner_exhausted = true;
                        tracing::trace!("Planner client exhausted");
                    }
                }
            }
        }
    }
}
