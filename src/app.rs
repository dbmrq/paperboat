//! Main orchestrator application

use crate::acp::{AcpClient, AcpClientTrait};
use crate::error::{OrchestratorError, TimeoutConfig, TimeoutOperation};
use crate::logging::{AgentWriter, LogScope, RunLogManager};
use crate::mcp_server::{ToolCall, ToolRequest, ToolResponse};
use crate::models::ModelConfig;
use crate::types::{SessionOutput, TaskResult};
use anyhow::{Context, Result};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

/// Message sent from tool handlers back to the orchestrator loop.
///
/// This is public to allow test harnesses to inject tool calls directly
/// without going through Unix sockets.
#[derive(Debug)]
pub enum ToolMessage {
    /// A tool request that needs processing
    Request {
        /// The tool request with request_id and tool_call
        request: ToolRequest,
        /// Channel to send response back
        response_tx: tokio::sync::oneshot::Sender<ToolResponse>,
    },
}

pub struct App {
    acp_orchestrator: Box<dyn AcpClientTrait + Send>,
    acp_worker: Box<dyn AcpClientTrait + Send>,
    socket_path: Option<PathBuf>,
    /// Channel for receiving tool messages from socket handlers
    tool_rx: Option<mpsc::Receiver<ToolMessage>>,
    model_config: ModelConfig,
    /// Timeout configuration for orchestrator operations
    timeout_config: TimeoutConfig,
    /// The user's original goal/prompt
    original_goal: String,
    /// Handle to the socket listener task for cleanup
    socket_listener_task: Option<tokio::task::JoinHandle<()>>,
    /// Log manager for this run
    log_manager: Arc<RunLogManager>,
    /// Current logging scope (changes during decompose/subtasks)
    current_scope: LogScope,
    /// Stored plan from planner agent (written via write_plan tool)
    stored_plan: Option<String>,
}

/// Path to the orchestrator-specific auggie cache directory.
/// This directory has a settings.json with editing tools removed,
/// forcing the orchestrator to delegate work to worker agents.
const ORCHESTRATOR_CACHE_DIR: &str = "~/.villalobos/augment-orchestrator";

/// Tools to remove from the orchestrator agent.
/// These are editing/execution tools that should only be available to worker agents.
const ORCHESTRATOR_REMOVED_TOOLS: &[&str] = &[
    "str-replace-editor",
    "save-file",
    "remove-files",
    "apply_patch",
    "launch-process",
    "kill-process",
    "read-process",
    "write-process",
    "list-processes",
    "web-search",
    "web-fetch",
];

/// System prompt for the orchestrator agent (loaded from prompts/orchestrator.txt)
const ORCHESTRATOR_PROMPT: &str = include_str!("../prompts/orchestrator.txt");

/// System prompt for the planner agent (loaded from prompts/planner.txt)
const PLANNER_PROMPT: &str = include_str!("../prompts/planner.txt");

/// System prompt for the implementer agent (loaded from prompts/implementer.txt)
const IMPLEMENTER_PROMPT: &str = include_str!("../prompts/implementer.txt");

impl App {
    /// Set up the orchestrator cache directory with the required configuration.
    /// This ensures the orchestrator agent has editing tools removed.
    fn setup_orchestrator_cache() -> Result<String> {
        // First, check if auggie is authenticated
        let main_augment_dir = shellexpand::tilde("~/.augment").to_string();
        let main_session = std::path::Path::new(&main_augment_dir).join("session.json");

        if !main_session.exists() {
            anyhow::bail!(
                "Augment CLI is not authenticated.\n\n\
                Please run 'auggie login' first to authenticate, then try again."
            );
        }

        let cache_dir = shellexpand::tilde(ORCHESTRATOR_CACHE_DIR).to_string();
        let cache_path = std::path::Path::new(&cache_dir);

        // Create directory if it doesn't exist
        if !cache_path.exists() {
            std::fs::create_dir_all(cache_path)
                .context("Failed to create orchestrator cache directory")?;
            tracing::info!("Created orchestrator cache directory: {}", cache_dir);
        }

        // Copy session.json from main augment directory for authentication
        let orchestrator_session = cache_path.join("session.json");

        if !orchestrator_session.exists() {
            std::fs::copy(&main_session, &orchestrator_session)
                .context("Failed to copy session.json to orchestrator cache")?;
            tracing::info!("Copied session.json to orchestrator cache");
        }

        // Always write settings.json to ensure removedTools is current
        let settings = json!({
            "removedTools": ORCHESTRATOR_REMOVED_TOOLS
        });
        let settings_path = cache_path.join("settings.json");
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
            .context("Failed to write orchestrator settings.json")?;
        tracing::debug!("Wrote orchestrator settings.json with {} removed tools",
            ORCHESTRATOR_REMOVED_TOOLS.len());

        Ok(cache_dir)
    }

    /// Create a new App with a pre-created log manager.
    pub async fn with_log_manager(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
    ) -> Result<Self> {
        Self::with_log_manager_and_timeout(model_config, log_manager, TimeoutConfig::default())
            .await
    }

    /// Create a new App with a pre-created log manager and custom timeout configuration.
    pub async fn with_log_manager_and_timeout(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        timeout_config: TimeoutConfig,
    ) -> Result<Self> {
        // Set up orchestrator cache directory with removed tools
        let orchestrator_cache = Self::setup_orchestrator_cache()?;

        // Orchestrator uses a custom cache directory with editing tools removed
        let mut acp_orchestrator = AcpClient::spawn_with_timeout(
            Some(&orchestrator_cache),
            timeout_config.request_timeout,
        )
        .await?;
        acp_orchestrator.initialize().await?;

        // Workers use the default cache directory with all tools available
        let mut acp_worker =
            AcpClient::spawn_with_timeout(None, timeout_config.request_timeout).await?;
        acp_worker.initialize().await?;

        let current_scope = log_manager.root_scope();

        tracing::info!(
            "⏱️  Timeout config: session={}s, request={}s",
            timeout_config.session_timeout.as_secs(),
            timeout_config.request_timeout.as_secs()
        );

        Ok(Self {
            acp_orchestrator: Box::new(acp_orchestrator),
            acp_worker: Box::new(acp_worker),
            socket_path: None,
            tool_rx: None,
            model_config,
            timeout_config,
            original_goal: String::new(),
            socket_listener_task: None,
            log_manager,
            current_scope,
            stored_plan: None,
        })
    }

    /// Create a new App with mock ACP clients for testing.
    ///
    /// This constructor allows injection of mock ACP clients, enabling deterministic
    /// testing without requiring live agent processes.
    ///
    /// # Arguments
    /// * `orchestrator` - Mock ACP client for orchestrator agent
    /// * `worker` - Mock ACP client for worker agents (planner, implementer)
    /// * `model_config` - Model configuration for agent selection
    /// * `log_manager` - Log manager for this run
    pub fn with_mock_clients(
        orchestrator: Box<dyn AcpClientTrait + Send>,
        worker: Box<dyn AcpClientTrait + Send>,
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
    ) -> Self {
        let current_scope = log_manager.root_scope();

        Self {
            acp_orchestrator: orchestrator,
            acp_worker: worker,
            socket_path: None,
            tool_rx: None,
            model_config,
            timeout_config: TimeoutConfig::default(),
            original_goal: String::new(),
            socket_listener_task: None,
            log_manager,
            current_scope,
            stored_plan: None,
        }
    }

    /// Create a new App with mock ACP clients and an injected tool channel for testing.
    ///
    /// This constructor enables full test control over tool call handling by injecting
    /// the tool_rx channel directly, bypassing Unix socket setup.
    ///
    /// # Arguments
    /// * `orchestrator` - Mock ACP client for orchestrator agent
    /// * `worker` - Mock ACP client for worker agents (planner, implementer)
    /// * `model_config` - Model configuration for agent selection
    /// * `log_manager` - Log manager for this run
    /// * `tool_rx` - Pre-created channel receiver for tool messages
    pub fn with_mock_clients_and_tool_rx(
        orchestrator: Box<dyn AcpClientTrait + Send>,
        worker: Box<dyn AcpClientTrait + Send>,
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        tool_rx: mpsc::Receiver<ToolMessage>,
    ) -> Self {
        let current_scope = log_manager.root_scope();

        Self {
            acp_orchestrator: orchestrator,
            acp_worker: worker,
            socket_path: None,
            tool_rx: Some(tool_rx),
            model_config,
            timeout_config: TimeoutConfig::default(),
            original_goal: String::new(),
            socket_listener_task: None,
            log_manager,
            current_scope,
            stored_plan: None,
        }
    }

    /// Run the orchestrator with a goal
    ///
    /// This first spawns a Planner to create a high-level plan, then spawns
    /// an Orchestrator to execute the plan by delegating to implementers.
    pub async fn run(&mut self, goal: &str) -> Result<TaskResult> {
        tracing::info!("Starting with goal: {}", goal);

        // Store the original goal for context passing to implementers
        self.original_goal = goal.to_string();

        // Set up Unix socket for MCP server communication (unless tool_rx is already set for tests)
        let socket_path: Option<PathBuf> = if self.tool_rx.is_some() {
            // Test mode: tool_rx is already injected, skip socket setup
            // Use a placeholder socket path for MCP server config (won't actually be used)
            tracing::debug!("Test mode: skipping socket setup, tool_rx already set");
            let placeholder = PathBuf::from("/tmp/villalobos-test-socket-placeholder");
            self.socket_path = Some(placeholder.clone());
            Some(placeholder)
        } else {
            Some(self.setup_socket().await?)
        };

        // Create planner writer at root scope
        let mut planner_writer = self
            .current_scope
            .planner_writer()
            .await
            .context("Failed to create planner writer")?;

        // First, spawn a Planner to create a plan from the goal
        tracing::info!("📝 Planning phase: spawning planner agent");
        let planner_session = match self.spawn_planner(goal).await {
            Ok((session, prompt)) => {
                planner_writer.set_session_id(session.clone());
                if let Err(e) = planner_writer.write_header_with_prompt(goal, &prompt).await {
                    tracing::warn!("Failed to write planner header: {}", e);
                }
                session
            }
            Err(e) => {
                if let Some(ref path) = socket_path {
                    self.cleanup_socket(path);
                }
                return Err(e);
            }
        };

        // Wait for planner to complete and collect its output (with timeout)
        let planner_output = match self
            .wait_for_session_output(&planner_session, &mut planner_writer)
            .await
        {
            Ok(output) => output,
            Err(e) => {
                // Clean up before returning error
                if let Some(ref path) = socket_path {
                    self.cleanup_socket(path);
                }
                return Err(anyhow::anyhow!("{}", e));
            }
        };

        // Finalize planner log
        if let Err(e) = planner_writer.finalize(true).await {
            tracing::warn!("Failed to finalize planner log: {}", e);
        }

        // Use the stored plan (from write_plan tool) if available,
        // otherwise fall back to full planner output (for backward compatibility)
        let plan_to_execute = if let Some(ref plan) = self.stored_plan {
            tracing::info!("📋 Using stored plan ({} chars) from write_plan tool", plan.len());
            plan.clone()
        } else if !planner_output.is_empty() {
            tracing::warn!("⚠️  Planner did not use write_plan tool, falling back to full output");
            planner_output.text.clone()
        } else {
            if let Some(ref path) = socket_path {
                self.cleanup_socket(path);
            }
            return Err(anyhow::anyhow!("Planner produced no plan (neither write_plan nor text output)"));
        };

        // Create orchestrator writer
        let mut orchestrator_writer = self
            .current_scope
            .orchestrator_writer()
            .await
            .context("Failed to create orchestrator writer")?;

        // Now spawn the orchestrator to execute the plan
        // Pass the clean plan (not the planner's full stream-of-consciousness)
        tracing::info!("🎭 Execution phase: spawning orchestrator agent");
        let result = self
            .run_orchestrator_with_writer(&plan_to_execute, &mut orchestrator_writer)
            .await;

        // Clear stored plan after use
        self.stored_plan = None;

        // Finalize orchestrator log
        let success = result.as_ref().map(|r| r.success).unwrap_or(false);
        if let Err(e) = orchestrator_writer.finalize(success).await {
            tracing::warn!("Failed to finalize orchestrator log: {}", e);
        }

        // Always clean up, even if orchestrator failed
        if let Some(ref path) = socket_path {
            self.cleanup_socket(path);
        }

        result
    }

    /// Clean up the socket file and listener task
    fn cleanup_socket(&mut self, socket_path: &PathBuf) {
        // Abort the socket listener task
        if let Some(task) = self.socket_listener_task.take() {
            task.abort();
        }

        // Remove socket file
        if let Err(e) = std::fs::remove_file(socket_path) {
            tracing::warn!("Failed to remove socket file: {}", e);
        }
    }

    /// Gracefully shutdown the application and all child processes.
    ///
    /// This should be called before the App is dropped to ensure clean termination
    /// of all agent processes and background tasks.
    pub async fn shutdown(&mut self) -> Result<()> {
        tracing::info!("🛑 Shutting down application...");

        // Clean up socket if it exists
        if let Some(socket_path) = self.socket_path.take() {
            self.cleanup_socket(&socket_path);
        }

        // Shutdown both ACP clients
        // Run them concurrently since they're independent
        let (orchestrator_result, worker_result) = tokio::join!(
            self.acp_orchestrator.shutdown(),
            self.acp_worker.shutdown()
        );

        if let Err(e) = orchestrator_result {
            tracing::warn!("Error shutting down orchestrator ACP client: {}", e);
        }
        if let Err(e) = worker_result {
            tracing::warn!("Error shutting down worker ACP client: {}", e);
        }

        tracing::info!("✅ Shutdown complete");
        Ok(())
    }

    /// Set up Unix socket for MCP server communication
    async fn setup_socket(&mut self) -> Result<PathBuf> {
        let socket_path = std::env::temp_dir().join(format!("villalobos-{}.sock", uuid::Uuid::new_v4()));

        // Remove socket file if it exists
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)
            .context("Failed to bind Unix socket")?;

        tracing::info!("Unix socket listening at: {:?}", socket_path);

        // Spawn task to accept connections and forward tool requests
        let (tool_tx, tool_rx) = mpsc::channel::<ToolMessage>(100);

        let listener_task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let tool_tx = tool_tx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_mcp_connection(stream, tool_tx).await {
                                tracing::error!("MCP connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        // This error is expected when the listener is dropped during shutdown
                        tracing::debug!("Socket listener stopped: {}", e);
                        break;
                    }
                }
            }
        });

        self.socket_path = Some(socket_path.clone());
        self.tool_rx = Some(tool_rx);
        self.socket_listener_task = Some(listener_task);

        Ok(socket_path)
    }
    /// Handle ACP messages and stream agent output
    async fn handle_acp_message(&self, msg: &serde_json::Value, agent_type: &str) {
        if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
            if method == "session/update" {
                if let Some(params) = msg.get("params") {
                    if let Some(update) = params.get("update") {
                        if let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
                            match session_update {
                                "agent_message_chunk" | "agent_thought_chunk" => {
                                    // Stream agent messages to stdout
                                    if let Some(content) = update.get("content") {
                                        if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                            print!("{}", text);
                                            std::io::Write::flush(&mut std::io::stdout()).ok();
                                        }
                                    }
                                }
                                "tool_call" => {
                                    if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                                        tracing::info!("🔧 {} tool call: {}", agent_type, title);
                                    }
                                }
                                "tool_result" => {
                                    let title = update.get("title").and_then(|t| t.as_str()).unwrap_or("unknown");
                                    let is_error = update.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
                                    if is_error {
                                        let content = update.get("content").and_then(|c| {
                                            c.get("text").and_then(|t| t.as_str())
                                        }).unwrap_or("no error message");
                                        tracing::error!("❌ {} tool failed: {} - {}", agent_type, title, content);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle decompose tool call, returning a ToolResponse
    async fn handle_decompose_with_response(&mut self, task: &str, request_id: &str) -> ToolResponse {
        match self.handle_decompose_inner(task).await {
            Ok(summary) => ToolResponse::success(request_id.to_string(), summary),
            Err(e) => ToolResponse::failure(request_id.to_string(), e.to_string()),
        }
    }

    /// Inner decompose logic that can fail
    async fn handle_decompose_inner(&mut self, task: &str) -> Result<String> {
        let start_time = std::time::Instant::now();
        tracing::info!("🔄 Starting decomposition: {}", truncate_for_log(task, 100));

        // Create child scope (subtask folder) for this decomposition
        let child_scope = self
            .current_scope
            .child_scope(task)
            .await
            .context("Failed to create child scope")?;

        let subtask_dir = child_scope.dir().display().to_string();
        tracing::debug!("📁 Created subtask directory: {}", subtask_dir);
        let previous_scope = std::mem::replace(&mut self.current_scope, child_scope);

        // Create planner writer for subtask
        let mut planner_writer = self
            .current_scope
            .planner_writer()
            .await
            .context("Failed to create subtask planner writer")?;

        // 1. Spawn planner to create plan
        let (planner_session, planner_prompt) = self.spawn_planner(task).await?;
        planner_writer.set_session_id(planner_session.clone());
        if let Err(e) = planner_writer.write_header_with_prompt(task, &planner_prompt).await {
            tracing::warn!("Failed to write subtask planner header: {}", e);
        }

        // 2. Wait for planner to complete and collect output (with timeout)
        let planner_output = self
            .wait_for_session_output(&planner_session, &mut planner_writer)
            .await
            .map_err(|e| {
                // Restore scope before returning error
                self.current_scope = previous_scope.clone();
                anyhow::anyhow!("{}", e)
            })?;

        // Finalize planner log
        if let Err(e) = planner_writer.finalize(true).await {
            tracing::warn!("Failed to finalize subtask planner log: {}", e);
        }

        // Use the stored plan (from write_plan tool) if available,
        // otherwise fall back to full planner output (for backward compatibility)
        let plan_to_execute = if let Some(ref plan) = self.stored_plan {
            tracing::info!("📋 Using stored plan ({} chars) from write_plan tool", plan.len());
            plan.clone()
        } else if !planner_output.is_empty() {
            tracing::warn!("⚠️  Planner did not use write_plan tool, falling back to full output");
            planner_output.text.clone()
        } else {
            self.current_scope = previous_scope;
            return Err(anyhow::anyhow!("Planner produced no plan for decomposition"));
        };

        // 3. Create orchestrator writer for subtask
        let mut orchestrator_writer = self
            .current_scope
            .orchestrator_writer()
            .await
            .context("Failed to create subtask orchestrator writer")?;

        // 4. Spawn child orchestrator with clean plan (not full planner output)
        let result = self
            .run_orchestrator_with_writer(&plan_to_execute, &mut orchestrator_writer)
            .await?;

        // Clear stored plan after use
        self.stored_plan = None;

        // Finalize orchestrator log
        if let Err(e) = orchestrator_writer.finalize(result.success).await {
            tracing::warn!("Failed to finalize subtask orchestrator log: {}", e);
        }

        // Restore previous scope
        self.current_scope = previous_scope;

        let elapsed = start_time.elapsed();
        let elapsed_str = format_duration_human(elapsed);

        if result.success {
            tracing::info!("✅ Decomposition complete ({}) - {}", elapsed_str, truncate_for_log(task, 60));
        } else {
            tracing::error!("❌ Decomposition FAILED ({}) - {}", elapsed_str, truncate_for_log(task, 60));
        }

        Ok(format!(
            "Decomposed and executed. Result: {}",
            if result.success { "success" } else { "failure" }
        ))
    }

    /// Handle implement tool call, returning a ToolResponse
    async fn handle_implement_with_response(&mut self, task: &str, request_id: &str) -> ToolResponse {
        match self.handle_implement_inner(task).await {
            Ok(summary) => ToolResponse::success(request_id.to_string(), summary),
            Err(e) => ToolResponse::failure(request_id.to_string(), e.to_string()),
        }
    }

    /// Inner implement logic that can fail
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
        tracing::info!("🔨 [{}] Starting implementation: {}", impl_name, truncate_for_log(task, 100));

        // Spawn implementer
        let (impl_session, impl_prompt) = self.spawn_implementer(task).await?;
        impl_writer.set_session_id(impl_session.clone());
        if let Err(e) = impl_writer.write_header_with_prompt(task, &impl_prompt).await {
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
            tracing::error!("❌ [{}] Implementation FAILED after {} - task: {}", impl_name, elapsed_str, truncate_for_log(task, 80));
            return Err(anyhow::anyhow!("Implementation failed for task: {}", task));
        }

        tracing::info!("✅ [{}] Implementation complete ({}) - {}", impl_name, elapsed_str, truncate_for_log(task, 60));

        Ok(format!("Task completed: {}", task))
    }

    /// Wait for a session to complete, collecting all message output.
    /// This is the unified wait function for all agent types (planner, implementer, etc.)
    async fn wait_for_session_output(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> Result<SessionOutput, OrchestratorError> {
        let timeout_duration = self.timeout_config.session_timeout;

        match tokio::time::timeout(
            timeout_duration,
            self.wait_for_session_output_inner(session_id, writer),
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
                    context: Some(format!("session_id: {}", session_id)),
                })
            }
        }
    }

    /// Inner implementation of wait_for_session_output without timeout.
    /// Waits for the agent to call the `complete` tool via our MCP server.
    async fn wait_for_session_output_inner(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> Result<SessionOutput> {
        tracing::debug!("⏳ Waiting for session: {}", session_id);
        let mut output = SessionOutput::new();
        // Track which unhandled update types we've already logged (to avoid spam)
        let mut seen_unhandled: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Take tool_rx for this session
        let mut tool_rx = self.tool_rx.take().context("Tool receiver not set up")?;

        loop {
            tokio::select! {
                // Handle tool calls from MCP server (complete signal)
                Some(tool_msg) = tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;

                    match &request.tool_call {
                        ToolCall::Complete { success, message } => {
                            tracing::info!(
                                "✅ Session {} signaled complete: success={}, message={:?}",
                                session_id,
                                success,
                                message
                            );

                            // Log the completion message to the agent's log file
                            if let Some(msg) = message {
                                let _ = writer.write_result(msg).await;
                            }

                            // Send success response back to MCP server
                            let response = ToolResponse::success(
                                request.request_id,
                                message.clone().unwrap_or_else(|| "Done".to_string()),
                            );
                            let _ = response_tx.send(response);

                            // Continue draining messages until we get session_finished
                            // or a short timeout, to ensure clean session termination
                            tracing::debug!("⏳ Draining remaining messages for session {}", session_id);
                            let drain_result = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                self.drain_session_messages(session_id, writer),
                            ).await;

                            if drain_result.is_err() {
                                tracing::debug!("⚠️ Timeout waiting for session_finished, proceeding anyway");
                            }

                            // Restore tool_rx and return
                            self.tool_rx = Some(tool_rx);
                            return Ok(output);
                        }
                        ToolCall::WritePlan { plan } => {
                            tracing::info!(
                                "📝 Session {} submitted plan ({} chars)",
                                session_id,
                                plan.len()
                            );

                            // Store the plan for the orchestrator
                            self.stored_plan = Some(plan.clone());

                            // Send success response back to MCP server
                            let response = ToolResponse::success(
                                request.request_id,
                                "Plan stored successfully".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                        other => {
                            // Planner/implementer shouldn't call other tools, but handle gracefully
                            tracing::warn!("Unexpected tool call from session {}: {:?}", session_id, other);
                            let response = ToolResponse::failure(
                                request.request_id,
                                "This tool is not available. Use complete() to signal you're done.".to_string(),
                            );
                            let _ = response_tx.send(response);
                        }
                    }
                }

                // Handle ACP messages (streaming output)
                msg_result = self.acp_worker.recv() => {
                    let msg = msg_result?;
                    let method = msg.get("method").and_then(|v| v.as_str());

                    if let Some(method) = method {
                        if method == "session/update" {
                            if let Some(params) = msg.get("params") {
                                let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

                                // Ignore messages for other sessions
                                if msg_session_id != Some(session_id) {
                                    continue;
                                }

                                if let Some(update) = params.get("update") {
                                    let session_update =
                                        update.get("sessionUpdate").and_then(|v| v.as_str());

                                    if let Some(session_update) = session_update {
                                        match session_update {
                                            // Message chunks - stream to stdout and collect
                                            "agent_message_chunk" | "agent_thought_chunk" => {
                                                if let Some(text) = update
                                                    .get("content")
                                                    .and_then(|c| c.get("text"))
                                                    .and_then(|t| t.as_str())
                                                {
                                                    output.append(text);
                                                    let _ = writer.write_message_chunk(text).await;
                                                    print!("{}", text);
                                                    std::io::Write::flush(&mut std::io::stdout()).ok();
                                                }
                                            }
                                            "tool_call" => {
                                                if let Some(title) =
                                                    update.get("title").and_then(|t| t.as_str())
                                                {
                                                    let _ = writer.write_tool_call(title).await;
                                                    tracing::info!("🔧 tool call: {}", title);
                                                }
                                            }
                                            "tool_result" => {
                                                let title = update
                                                    .get("title")
                                                    .and_then(|t| t.as_str())
                                                    .unwrap_or("unknown");
                                                let is_error = update
                                                    .get("isError")
                                                    .and_then(|e| e.as_bool())
                                                    .unwrap_or(false);
                                                let content = update
                                                    .get("content")
                                                    .and_then(|c| c.get("text"))
                                                    .and_then(|t| t.as_str())
                                                    .unwrap_or("");
                                                let _ = writer
                                                    .write_tool_result(title, is_error, content)
                                                    .await;
                                                if is_error {
                                                    tracing::error!(
                                                        "❌ tool failed: {} - {}",
                                                        title,
                                                        content
                                                    );
                                                }
                                            }
                                            // Also handle explicit completion signals from ACP
                                            "agent_turn_finished" | "session_finished" => {
                                                tracing::info!(
                                                    "✅ Session {} completed: {}",
                                                    session_id,
                                                    session_update
                                                );
                                                self.tool_rx = Some(tool_rx);
                                                return Ok(output);
                                            }
                                            // Tool progress updates (streaming output from tools)
                                            "tool_call_update" => {
                                                let tool_name = update
                                                    .get("title")
                                                    .and_then(|t| t.as_str())
                                                    .unwrap_or("unknown");
                                                if let Some(text) = update
                                                    .get("content")
                                                    .and_then(|c| c.get("text"))
                                                    .and_then(|t| t.as_str())
                                                {
                                                    // Stream tool progress for UI observation
                                                    let _ = writer
                                                        .write_tool_progress(tool_name, text)
                                                        .await;
                                                    tracing::trace!(
                                                        "🔄 tool progress: {} - {} chars",
                                                        tool_name,
                                                        text.len()
                                                    );
                                                }
                                            }
                                            // Silently ignore known non-essential updates
                                            "agent_turn_started" | "thinking_start" | "thinking_end" => {}
                                            // Log unknown types once per type to help diagnose issues
                                            other => {
                                                if seen_unhandled.insert(other.to_string()) {
                                                    tracing::debug!(
                                                        "📨 Unhandled session update type: {}",
                                                        other
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Drain remaining messages for a session after complete() is called.
    /// This ensures we don't leave stale messages in the notification channel.
    /// Waits until we receive session_finished or the caller times out.
    async fn drain_session_messages(&mut self, session_id: &str, writer: &mut AgentWriter) {
        loop {
            match self.acp_worker.recv().await {
                Ok(msg) => {
                    let method = msg.get("method").and_then(|v| v.as_str());
                    if method == Some("session/update") {
                        if let Some(params) = msg.get("params") {
                            let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

                            // Only process messages for this session
                            if msg_session_id == Some(session_id) {
                                if let Some(update) = params.get("update") {
                                    let session_update = update.get("sessionUpdate").and_then(|v| v.as_str());

                                    if let Some(update_type) = session_update {
                                        match update_type {
                                            "session_finished" | "agent_turn_finished" => {
                                                tracing::debug!("✅ Session {} properly finished", session_id);
                                                return;
                                            }
                                            "agent_message_chunk" | "agent_thought_chunk" => {
                                                // Continue logging any remaining output
                                                if let Some(text) = update
                                                    .get("content")
                                                    .and_then(|c| c.get("text"))
                                                    .and_then(|t| t.as_str())
                                                {
                                                    let _ = writer.write_message_chunk(text).await;
                                                    // Also print to stdout for visibility
                                                    print!("{}", text);
                                                    std::io::Write::flush(&mut std::io::stdout()).ok();
                                                }
                                            }
                                            _ => {
                                                // Silently ignore other update types during drain
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Channel closed during drain: {}", e);
                    return;
                }
            }
        }
    }

    /// Drain remaining messages for an orchestrator session after complete() is called.
    /// Similar to drain_session_messages but uses acp_orchestrator channel.
    async fn drain_orchestrator_messages(&mut self, session_id: &str, writer: &mut AgentWriter) {
        loop {
            match self.acp_orchestrator.recv().await {
                Ok(msg) => {
                    let method = msg.get("method").and_then(|v| v.as_str());
                    if method == Some("session/update") {
                        if let Some(params) = msg.get("params") {
                            let msg_session_id = params.get("sessionId").and_then(|v| v.as_str());

                            // Only process messages for this session
                            if msg_session_id == Some(session_id) {
                                if let Some(update) = params.get("update") {
                                    let session_update = update.get("sessionUpdate").and_then(|v| v.as_str());

                                    if let Some(update_type) = session_update {
                                        match update_type {
                                            "session_finished" | "agent_turn_finished" => {
                                                tracing::debug!("✅ Orchestrator session {} properly finished", session_id);
                                                return;
                                            }
                                            "agent_message_chunk" | "agent_thought_chunk" => {
                                                // Continue logging any remaining output
                                                if let Some(text) = update
                                                    .get("content")
                                                    .and_then(|c| c.get("text"))
                                                    .and_then(|t| t.as_str())
                                                {
                                                    let _ = writer.write_message_chunk(text).await;
                                                    print!("{}", text);
                                                    std::io::Write::flush(&mut std::io::stdout()).ok();
                                                }
                                            }
                                            _ => {
                                                // Silently ignore other update types during drain
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("Orchestrator channel closed during drain: {}", e);
                    return;
                }
            }
        }
    }

    /// Spawn an orchestrator agent
    /// Returns (session_id, full_prompt) so the prompt can be logged
    async fn spawn_orchestrator(&mut self, prompt: &str) -> Result<(String, String)> {
        let cwd = std::env::current_dir()?
            .to_string_lossy()
            .to_string();

        // Get the path to the current binary
        let binary_path = std::env::current_exe()
            .context("Failed to get current executable path")?;

        // Get socket path
        let socket_path = self.socket_path.as_ref()
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
            .session_new(self.model_config.orchestrator_model.as_str(), mcp_servers, &cwd)
            .await?;

        let full_prompt = format!(
            "{}\n\n## PLAN TO EXECUTE\n\nThe following plan was created by a planner agent. Your job is to execute it by calling implement() or decompose() for each task. Do NOT re-plan or re-analyze. Just execute the tasks in order.\n\n{}",
            ORCHESTRATOR_PROMPT.trim(),
            prompt
        );
        tracing::debug!("🎭 Orchestrator prompt:\n{}", full_prompt);

        self.acp_orchestrator
            .session_prompt(&response.session_id, &full_prompt)
            .await?;

        Ok((response.session_id, full_prompt))
    }

    /// Spawn a planner agent
    /// Returns (session_id, prompt) so the prompt can be logged
    async fn spawn_planner(&mut self, task: &str) -> Result<(String, String)> {
        let cwd = std::env::current_dir()?
            .to_string_lossy()
            .to_string();

        // Get the path to the current binary and socket
        let binary_path = std::env::current_exe()
            .context("Failed to get current executable path")?;
        let socket_path = self.socket_path.as_ref()
            .context("Socket not set up")?
            .to_string_lossy()
            .to_string();

        // Configure MCP server with planner agent type (only gets write_plan and complete tools)
        // Use unique name "villalobos-planner" to prevent MCP server caching/reuse between agent types
        let mcp_servers = vec![json!({
            "name": "villalobos-planner",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server"],
            "env": [
                {"name": "VILLALOBOS_SOCKET", "value": socket_path},
                {"name": "VILLALOBOS_AGENT_TYPE", "value": "planner"}
            ]
        })];

        let response = self
            .acp_worker
            .session_new(self.model_config.planner_model.as_str(), mcp_servers, &cwd)
            .await?;

        let prompt = PLANNER_PROMPT.replace("{task}", task);
        tracing::debug!("📝 Planner prompt:\n{}", prompt);

        self.acp_worker.session_prompt(&response.session_id, &prompt).await?;

        Ok((response.session_id, prompt))
    }

    /// Spawn an implementer agent
    /// Returns (session_id, prompt) so the prompt can be logged
    async fn spawn_implementer(&mut self, task: &str) -> Result<(String, String)> {
        let cwd = std::env::current_dir()?
            .to_string_lossy()
            .to_string();

        // Get the path to the current binary and socket
        let binary_path = std::env::current_exe()
            .context("Failed to get current executable path")?;
        let socket_path = self.socket_path.as_ref()
            .context("Socket not set up")?
            .to_string_lossy()
            .to_string();

        // Configure MCP server with implementer agent type (only gets complete tool)
        // Use unique name "villalobos-implementer" to prevent MCP server caching/reuse between agent types
        let mcp_servers = vec![json!({
            "name": "villalobos-implementer",
            "command": binary_path.to_string_lossy(),
            "args": ["--mcp-server"],
            "env": [
                {"name": "VILLALOBOS_SOCKET", "value": socket_path},
                {"name": "VILLALOBOS_AGENT_TYPE", "value": "implementer"}
            ]
        })];

        let response = self
            .acp_worker
            .session_new(self.model_config.implementer_model.as_str(), mcp_servers, &cwd)
            .await?;

        let prompt = IMPLEMENTER_PROMPT
            .replace("{task}", task)
            .replace("{user_goal}", &self.original_goal);
        tracing::debug!("🔨 Implementer prompt:\n{}", prompt);

        self.acp_worker.session_prompt(&response.session_id, &prompt).await?;

        Ok((response.session_id, prompt))
    }

    /// Spawn and run an orchestrator agent with logging.
    fn run_orchestrator_with_writer<'a>(
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
                        ToolCall::Implement { task } => {
                            // Log the MCP tool call to orchestrator log
                            let tool_desc = format!("implement_villalobos: {}", truncate_for_log(task, 100));
                            let _ = writer.write_tool_call(&tool_desc).await;
                            tracing::info!("🔨 MCP tool call: implement({})", truncate_for_log(task, 80));

                            // Restore tool_rx before spawning implementer (it needs to receive complete signal)
                            self.tool_rx = Some(tool_rx);
                            let response = self.handle_implement_with_response(task, &request.request_id).await;
                            tool_rx = self.tool_rx.take().context("Tool receiver lost during implement")?;

                            // Log the result to orchestrator log
                            let _ = writer.write_mcp_tool_result(
                                "implement",
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

    /// Handle ACP messages and write to the agent's log file.
    async fn handle_acp_message_with_writer(
        &self,
        msg: &serde_json::Value,
        writer: &mut AgentWriter,
    ) {
        let method = msg.get("method").and_then(|v| v.as_str());

        if method != Some("session/update") {
            tracing::trace!("📨 Orchestrator received non-update message: {:?}", method);
            return;
        }

        let Some(params) = msg.get("params") else {
            tracing::trace!("📨 Orchestrator session/update missing params");
            return;
        };

        let Some(update) = params.get("update") else {
            tracing::trace!("📨 Orchestrator session/update missing update field");
            return;
        };

        let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) else {
            tracing::trace!("📨 Orchestrator update missing sessionUpdate field");
            return;
        };

        match session_update {
            "agent_message_chunk" | "agent_thought_chunk" => {
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    tracing::trace!("📨 Writing orchestrator message chunk: {} chars", text.len());
                    let _ = writer.write_message_chunk(text).await;
                    print!("{}", text);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            }
            "tool_call" => {
                if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                    let _ = writer.write_tool_call(title).await;
                    tracing::info!("🔧 orchestrator tool call: {}", title);
                }
            }
            "tool_result" => {
                let title = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let is_error = update
                    .get("isError")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                let content = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let _ = writer.write_tool_result(title, is_error, content).await;
                if is_error {
                    tracing::error!(
                        "❌ orchestrator tool failed: {} - {}",
                        title,
                        content
                    );
                }
            }
            "agent_turn_finished" | "session_finished" => {
                tracing::debug!("📨 Orchestrator session event: {}", session_update);
            }
            // Tool progress updates (streaming output from tools)
            "tool_call_update" => {
                let tool_name = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    let _ = writer.write_tool_progress(tool_name, text).await;
                    tracing::trace!(
                        "🔄 orchestrator tool progress: {} - {} chars",
                        tool_name,
                        text.len()
                    );
                }
            }
            // Silently ignore known non-essential updates
            "agent_turn_started" | "thinking_start" | "thinking_end" => {}
            _ => {
                tracing::trace!("📨 Orchestrator unhandled sessionUpdate: {}", session_update);
            }
        }
    }
}


/// Truncate a string for logging, adding "..." if truncated.
fn truncate_for_log(s: &str, max_len: usize) -> String {
    // Replace newlines with spaces for cleaner log output
    let single_line = s.replace('\n', " ");
    if single_line.len() <= max_len {
        single_line
    } else {
        format!("{}...", &single_line[..max_len])
    }
}

/// Format a duration as a human-readable string for logging.
fn format_duration_human(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 60 {
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{}m {}s", mins, secs)
    } else if secs > 0 {
        format!("{}s", secs)
    } else {
        format!("{}ms", duration.as_millis())
    }
}

/// Handle an MCP connection from the MCP server.
///
/// Each connection represents a single tool call. The connection stays open
/// until the tool call completes, allowing the response to be sent back.
async fn handle_mcp_connection(
    stream: UnixStream,
    tool_tx: mpsc::Sender<ToolMessage>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read the tool request (one line of JSON)
    if reader.read_line(&mut line).await? == 0 {
        return Ok(()); // Connection closed
    }

    let request: ToolRequest = serde_json::from_str(&line)
        .context("Failed to parse tool request")?;

    // Create a oneshot channel for the response
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();

    // Send the request to the app for processing
    tool_tx.send(ToolMessage::Request { request, response_tx }).await
        .context("Failed to send tool request to app")?;

    // Wait for the response
    let response = response_rx.await
        .context("Failed to receive response from app")?;

    // Send the response back to the MCP server
    let response_json = serde_json::to_string(&response)?;
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    Ok(())
}
