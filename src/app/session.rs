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
                        // Support both ACP format (sessionUpdate) and CLI format (type)
                        if let Some(session_update) = update
                            .get("sessionUpdate")
                            .or_else(|| update.get("type"))
                            .and_then(|v| v.as_str())
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
    ///
    /// Uses the default `self.tool_rx` for receiving MCP tool calls.
    ///
    /// Note: Currently unused as callers use `wait_for_session_output_with_tool_rx` directly,
    /// but kept for API completeness as a simpler entry point.
    #[allow(dead_code)]
    #[tracing::instrument(skip(self, writer), fields(session_id = %session_id))]
    pub(crate) async fn wait_for_session_output(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> Result<SessionOutput, OrchestratorError> {
        self.wait_for_session_output_with_tool_rx(session_id, writer, None)
            .await
    }

    /// Wait for a session to complete with an optional custom tool_rx.
    ///
    /// For CLI planner sessions, a separate socket is created with its own tool_rx.
    /// Pass that tool_rx here to ensure MCP tool calls are received from the correct socket.
    #[tracing::instrument(skip(self, writer, tool_rx_override), fields(session_id = %session_id))]
    pub(crate) async fn wait_for_session_output_with_tool_rx(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
        tool_rx_override: Option<super::types::ToolReceiver>,
    ) -> Result<SessionOutput, OrchestratorError> {
        let timeout_duration = self.timeout_config.session_timeout;

        // If router is active, use routed mode with per-session receiver
        // Otherwise, use direct mode (for tests with mock clients)
        if self.router_active {
            // Register the session with the router to get a per-session receiver
            let session_rx = self.register_session(session_id).await;

            let result = tokio::time::timeout(
                timeout_duration,
                self.wait_for_session_output_routed(
                    session_id,
                    writer,
                    session_rx,
                    tool_rx_override,
                ),
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
                self.wait_for_session_output_direct(session_id, writer, tool_rx_override),
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
    #[tracing::instrument(skip(self, writer, session_rx, tool_rx_override), fields(session_id = %session_id, mode = "routed"))]
    async fn wait_for_session_output_routed(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
        mut session_rx: mpsc::Receiver<Value>,
        tool_rx_override: Option<super::types::ToolReceiver>,
    ) -> Result<SessionOutput> {
        tracing::debug!("⏳ Waiting for session (routed): {}", session_id);
        let mut output = SessionOutput::new();
        let mut seen_unhandled: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Track whether we're using an override tool_rx (from a dedicated socket)
        // vs the main self.tool_rx. We only restore self.tool_rx when we took it.
        let using_override = tool_rx_override.is_some();

        // Use the override tool_rx if provided (for CLI planner with separate socket),
        // otherwise use self.tool_rx
        let mut tool_rx = match tool_rx_override {
            Some(rx) => rx,
            None => self.tool_rx.take().context("Tool receiver not set up")?,
        };

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

                            // Only restore tool_rx to self if we took it (not using override).
                            // When using override (e.g., implementer's socket), we let the
                            // implementer's tool_rx drop here, preserving the caller's tool_rx.
                            if !using_override {
                                self.tool_rx = Some(tool_rx);
                            }
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
                                "This tool is not available to implementer agents. You have access only to complete(). Call complete(success=true, message=\"...\") when your task is done, or complete(success=false, message=\"...\") if you encountered an error.".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                    }
                }

                Some(msg) = session_rx.recv() => {
                    let finished = self.handle_worker_session_message(&msg, session_id, writer, &mut output, &mut seen_unhandled).await?;
                    if finished {
                        // Only restore tool_rx to self if we took it (not using override)
                        if !using_override {
                            self.tool_rx = Some(tool_rx);
                        }
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
    #[tracing::instrument(skip(self, writer, tool_rx_override), fields(session_id = %session_id, mode = "direct"))]
    async fn wait_for_session_output_direct(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
        tool_rx_override: Option<super::types::ToolReceiver>,
    ) -> Result<SessionOutput> {
        tracing::debug!("⏳ Waiting for session (direct): {}", session_id);
        let mut output = SessionOutput::new();
        let mut seen_unhandled: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Track whether we're using an override tool_rx (from a dedicated socket)
        // vs the main self.tool_rx. We only restore self.tool_rx when we took it.
        let using_override = tool_rx_override.is_some();

        // Use the override tool_rx if provided (for CLI planner with separate socket),
        // otherwise use self.tool_rx
        let mut tool_rx = match tool_rx_override {
            Some(rx) => rx,
            None => self.tool_rx.take().context("Tool receiver not set up")?,
        };

        // Take the config update receiver out of self so we don't hold a mutable borrow
        #[cfg(feature = "tui")]
        let mut config_update_rx = self.config_update_rx.take();

        // Track which clients have been exhausted (for mock clients)
        let mut worker_exhausted = false;
        let mut planner_exhausted = false;

        loop {
            // If both clients are exhausted and no tool messages, we're stuck
            if worker_exhausted && planner_exhausted {
                // Only restore tool_rx to self if we took it (not using override)
                if !using_override {
                    self.tool_rx = Some(tool_rx);
                }
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

                            // Only restore tool_rx to self if we took it (not using override)
                            if !using_override {
                                self.tool_rx = Some(tool_rx);
                            }
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
                                "This tool is not available to implementer agents. You have access only to complete(). Call complete(success=true, message=\"...\") when your task is done, or complete(success=false, message=\"...\") if you encountered an error.".to_string(),
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
                            // Only restore tool_rx to self if we took it (not using override)
                            if !using_override {
                                self.tool_rx = Some(tool_rx);
                            }
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
                            // Only restore tool_rx to self if we took it (not using override)
                            if !using_override {
                                self.tool_rx = Some(tool_rx);
                            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::transport::{AgentTransport, SessionConfig, SessionInfo, ToolResult};
    use crate::logging::RunLogManager;
    use crate::mcp_server::{ToolCall, ToolRequest};
    use crate::models::{ModelConfig, ModelTier};
    use crate::testing::{MockBackend, MockTransport};
    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::{sleep, timeout, Duration};

    struct ChannelTransport {
        rx: Option<mpsc::Receiver<Value>>,
    }

    impl ChannelTransport {
        fn new(rx: mpsc::Receiver<Value>) -> Self {
            Self { rx: Some(rx) }
        }
    }

    #[async_trait]
    impl AgentTransport for ChannelTransport {
        async fn initialize(&mut self) -> Result<()> {
            Ok(())
        }

        async fn create_session(&mut self, _config: SessionConfig) -> Result<SessionInfo> {
            Ok(SessionInfo::new("channel-session"))
        }

        async fn send_prompt(&mut self, _session_id: &str, _prompt: &str) -> Result<()> {
            Ok(())
        }

        fn take_notifications(&mut self) -> Option<mpsc::Receiver<crate::backend::transport::SessionUpdate>> {
            None
        }

        async fn respond_to_tool(
            &mut self,
            _session_id: &str,
            _tool_use_id: &str,
            _result: ToolResult,
        ) -> Result<()> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }

        async fn recv(&mut self) -> Result<Value> {
            let rx = self
                .rx
                .as_mut()
                .ok_or_else(|| anyhow!("notification receiver already taken"))?;

            rx.recv()
                .await
                .ok_or_else(|| anyhow!("no more scripted notifications"))
        }
    }

    fn make_model_config() -> ModelConfig {
        let available = [ModelTier::Sonnet].into_iter().collect();
        ModelConfig::new(available)
    }

    fn session_update(session_id: &str, update_type: &str, content: Option<&str>) -> Value {
        let mut msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": update_type
                }
            }
        });

        if let Some(text) = content {
            msg["params"]["update"]["content"] = json!({ "text": text });
        }

        msg
    }

    fn send_tool_request(
        tool_tx: &mpsc::Sender<ToolMessage>,
        request_id: &str,
        tool_call: ToolCall,
    ) -> oneshot::Receiver<crate::mcp_server::ToolResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        tool_tx
            .try_send(ToolMessage::Request {
                request: ToolRequest {
                    request_id: request_id.to_string(),
                    tool_call,
                },
                response_tx,
            })
            .expect("tool request should enqueue");
        response_rx
    }

    fn make_test_app(
        tool_rx: mpsc::Receiver<ToolMessage>,
        worker: Box<dyn AgentTransport>,
    ) -> (App, TempDir) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let log_manager = Arc::new(
            RunLogManager::new(temp_dir.path().to_str().expect("utf-8 temp path"))
                .expect("log manager"),
        );

        let app = App::with_mock_transports_and_tool_rx(
            Box::new(MockBackend::new()),
            Box::new(MockTransport::empty()),
            Box::new(MockTransport::empty()),
            worker,
            make_model_config(),
            log_manager,
            tool_rx,
        );

        (app, temp_dir)
    }

    #[tokio::test]
    async fn test_wait_for_session_output_routed_drains_target_messages_and_unregisters_session() {
        let (tool_tx, tool_rx) = mpsc::channel(8);
        let (mut app, _temp_dir) = make_test_app(tool_rx, Box::new(MockTransport::empty()));
        app.router_active = true;

        let mut writer = app.current_scope.planner_writer().await.expect("planner writer");
        let log_path = writer.path().clone();

        let create_task_response = send_tool_request(
            &tool_tx,
            "req-create",
            ToolCall::CreateTask {
                name: "race task".to_string(),
                description: "created before completion wins".to_string(),
                dependencies: vec![],
            },
        );
        let complete_response = send_tool_request(
            &tool_tx,
            "req-complete",
            ToolCall::Complete {
                success: true,
                message: Some("completed first".to_string()),
                notes: None,
                add_tasks: None,
            },
        );

        let session_router = Arc::clone(&app.session_router);
        let notifier = tokio::spawn(async move {
            let target_msg = session_update("session-routed", "agent_message_chunk", Some("drained target"));
            let other_msg = session_update("session-other", "agent_message_chunk", Some("ignored other"));
            let finished_msg = session_update("session-routed", "session_finished", None);

            let mut routed = false;
            for _ in 0..20 {
                {
                    let router = session_router.read().await;
                    if router.route(target_msg.clone()) {
                        assert!(!router.route(other_msg.clone()));
                        assert!(router.route(finished_msg.clone()));
                        routed = true;
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }

            assert!(routed, "session should register before notifier gives up");
        });

        let output = timeout(
            Duration::from_secs(2),
            app.wait_for_session_output_with_tool_rx("session-routed", &mut writer, None),
        )
        .await
        .expect("wait should not time out")
        .expect("session should complete");

        notifier.await.expect("notifier should finish");

        let create_task_response = create_task_response.await.expect("create_task response");
        let complete_response = complete_response.await.expect("complete response");
        assert!(create_task_response.success);
        assert!(create_task_response.summary.contains("race task"));
        assert!(complete_response.success);
        assert_eq!(complete_response.summary, "completed first");

        assert!(
            output.text.is_empty(),
            "messages drained after complete() should not be appended to SessionOutput"
        );
        assert!(app.tool_rx.is_some(), "tool receiver should be restored after routed wait");

        let log_contents = std::fs::read_to_string(&log_path).expect("writer log should be readable");
        assert!(log_contents.contains("drained target"));
        assert!(!log_contents.contains("ignored other"));

        let task_manager = app.task_manager.read().await;
        assert!(task_manager
            .all_tasks()
            .into_iter()
            .any(|task| task.name == "race task"));
        drop(task_manager);

        let late_msg = session_update("session-routed", "agent_message_chunk", Some("too late"));
        let router = app.session_router.read().await;
        assert!(
            !router.route(late_msg),
            "messages for the session should stop routing after unregister"
        );
    }

    #[tokio::test]
    async fn test_wait_for_session_output_direct_keeps_pre_complete_output_and_drains_tail() {
        let (tool_tx, tool_rx) = mpsc::channel(8);
        let (worker_tx, worker_rx) = mpsc::channel(8);
        let (mut app, _temp_dir) = make_test_app(tool_rx, Box::new(ChannelTransport::new(worker_rx)));

        let mut writer = app
            .current_scope
            .implementer_writer()
            .await
            .expect("implementer writer");
        let log_path = writer.path().clone();

        let tool_tx_for_task = tool_tx.clone();
        let sender_task = tokio::spawn(async move {
            worker_tx
                .send(session_update(
                    "session-direct",
                    "agent_message_chunk",
                    Some("before complete"),
                ))
                .await
                .expect("first worker update");

            sleep(Duration::from_millis(10)).await;

            let complete_response = send_tool_request(
                &tool_tx_for_task,
                "req-direct-complete",
                ToolCall::Complete {
                    success: true,
                    message: Some("done".to_string()),
                    notes: None,
                    add_tasks: None,
                },
            );

            sleep(Duration::from_millis(10)).await;

            worker_tx
                .send(session_update(
                    "session-direct",
                    "agent_message_chunk",
                    Some("drained tail"),
                ))
                .await
                .expect("tail worker update");
            worker_tx
                .send(session_update("session-direct", "session_finished", None))
                .await
                .expect("session finished");

            complete_response.await.expect("complete response")
        });

        let output = timeout(
            Duration::from_secs(2),
            app.wait_for_session_output_with_tool_rx("session-direct", &mut writer, None),
        )
        .await
        .expect("direct wait should not time out")
        .expect("direct session should complete");

        let complete_response = sender_task.await.expect("sender task should finish");
        assert!(complete_response.success);
        assert_eq!(complete_response.summary, "done");

        assert_eq!(output.text, "before complete");
        assert!(app.tool_rx.is_some(), "tool receiver should be restored after direct wait");

        let log_contents = std::fs::read_to_string(&log_path).expect("writer log should be readable");
        assert!(log_contents.contains("before complete"));
        assert!(
            log_contents.contains("drained tail"),
            "tail output should still be forwarded during the drain phase"
        );
    }
}
