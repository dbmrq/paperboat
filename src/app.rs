//! Main orchestrator application

use crate::acp::AcpClient;
use crate::error::{OrchestratorError, TimeoutConfig, TimeoutOperation};
use crate::mcp_server::{ToolCall, ToolRequest, ToolResponse};
use crate::models::ModelConfig;
use crate::types::{Plan, PlanEntry, TaskResult};
use anyhow::{Context, Result};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

/// A level in the plan hierarchy (used for context passing to implementers)
#[derive(Debug, Clone)]
struct PlanLevel {
    /// The task that was decomposed to create this level
    parent_task: String,
    /// All tasks at this level
    tasks: Vec<String>,
    /// Index of the task currently being executed
    current_index: usize,
}

/// Message sent from tool handlers back to the orchestrator loop
#[derive(Debug)]
enum ToolMessage {
    /// A tool request that needs processing
    Request {
        request: ToolRequest,
        /// Channel to send response back
        response_tx: tokio::sync::oneshot::Sender<ToolResponse>,
    },
}

pub struct App {
    acp_orchestrator: AcpClient,
    acp_worker: AcpClient,
    socket_path: Option<PathBuf>,
    /// Channel for receiving tool messages from socket handlers
    tool_rx: Option<mpsc::Receiver<ToolMessage>>,
    model_config: ModelConfig,
    /// Timeout configuration for orchestrator operations
    timeout_config: TimeoutConfig,
    /// The user's original goal/prompt
    original_goal: String,
    /// Stack of plan levels (for nested decomposition)
    plan_stack: Vec<PlanLevel>,
    /// Handle to the socket listener task for cleanup
    socket_listener_task: Option<tokio::task::JoinHandle<()>>,
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

    /// Create a new App with default timeout configuration.
    pub async fn new(model_config: ModelConfig) -> Result<Self> {
        Self::with_timeout_config(model_config, TimeoutConfig::default()).await
    }

    /// Create a new App with custom timeout configuration.
    pub async fn with_timeout_config(
        model_config: ModelConfig,
        timeout_config: TimeoutConfig,
    ) -> Result<Self> {
        // Set up orchestrator cache directory with removed tools
        let orchestrator_cache = Self::setup_orchestrator_cache()?;

        // Orchestrator uses a custom cache directory with editing tools removed
        let mut acp_orchestrator = AcpClient::spawn(Some(&orchestrator_cache)).await?;
        acp_orchestrator.initialize().await?;

        // Workers use the default cache directory with all tools available
        let mut acp_worker = AcpClient::spawn(None).await?;
        acp_worker.initialize().await?;

        tracing::info!(
            "⏱️  Timeout config: plan={}s, session={}s",
            timeout_config.plan_timeout.as_secs(),
            timeout_config.session_complete_timeout.as_secs()
        );

        Ok(Self {
            acp_orchestrator,
            acp_worker,
            socket_path: None,
            tool_rx: None,
            model_config,
            timeout_config,
            original_goal: String::new(),
            plan_stack: Vec::new(),
            socket_listener_task: None,
        })
    }

    /// Run the orchestrator with a goal
    ///
    /// This first spawns a Planner to create a high-level plan, then spawns
    /// an Orchestrator to execute the plan by delegating to implementers.
    pub async fn run(&mut self, goal: &str) -> Result<TaskResult> {
        tracing::info!("Starting with goal: {}", goal);

        // Store the original goal for context passing to implementers
        self.original_goal = goal.to_string();
        self.plan_stack.clear();

        // Set up Unix socket for MCP server communication
        let socket_path = self.setup_socket().await?;

        // First, spawn a Planner to create a plan from the goal
        tracing::info!("📝 Planning phase: spawning planner agent");
        let planner_session = match self.spawn_planner(goal).await {
            Ok(session) => session,
            Err(e) => {
                self.cleanup_socket(&socket_path);
                return Err(e);
            }
        };

        // Wait for the plan (with timeout)
        let plan = self.wait_for_plan(&planner_session).await.map_err(|e| {
            // Clean up before returning error
            self.cleanup_socket(&socket_path);
            anyhow::anyhow!("{}", e)
        })?;

        let num_entries = plan.entries.len();
        tracing::info!("📋 Plan created with {} entries", num_entries);
        for (i, entry) in plan.entries.iter().enumerate() {
            tracing::info!("  {}. {}", i + 1, entry.content);
        }

        // Push the root plan onto the stack for context tracking
        let tasks: Vec<String> = plan.entries.iter().map(|e| e.content.clone()).collect();
        self.plan_stack.push(PlanLevel {
            parent_task: goal.to_string(),
            tasks: tasks.clone(),
            current_index: 0,
        });

        // Build orchestrator prompt from the plan (same pattern as handle_decompose_inner)
        let tasks_list = tasks
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{}. {}", i + 1, t))
            .collect::<Vec<_>>()
            .join("\n");

        let orchestrator_prompt = format!(
            "You are an orchestrator. Complete these tasks:\n\n{}\n\nFor each task, decide if it's simple enough to implement() or needs to be decompose()d further. When all tasks are done, call complete(true).",
            tasks_list
        );

        // Now spawn the orchestrator to execute the plan
        tracing::info!("🎭 Execution phase: spawning orchestrator agent");
        let result = self.run_orchestrator(&orchestrator_prompt).await;

        // Pop the root plan level
        self.plan_stack.pop();

        // Always clean up, even if orchestrator failed
        self.cleanup_socket(&socket_path);

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

    /// Spawn and run an orchestrator agent
    fn run_orchestrator<'a>(
        &'a mut self,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<TaskResult>> + 'a>> {
        Box::pin(async move {
            self.run_orchestrator_impl(prompt).await
        })
    }

    async fn run_orchestrator_impl(&mut self, prompt: &str) -> Result<TaskResult> {
        let _session_id = self.spawn_orchestrator(prompt).await?;

        // Take tool_rx for this orchestrator run, but restore it when done
        // This allows nested orchestrator calls to share the same channel
        let mut tool_rx = self.tool_rx.take()
            .context("Tool receiver not set up")?;

        // Handle tool calls from MCP server
        let result = loop {
            tokio::select! {
                // Handle tool messages from MCP server
                Some(tool_msg) = tool_rx.recv() => {
                    let ToolMessage::Request { request, response_tx } = tool_msg;
                    tracing::info!("Received tool request: {:?}", request);

                    match &request.tool_call {
                        ToolCall::Decompose { task } => {
                            // Decompose is handled synchronously (it creates a child orchestrator)
                            // Restore tool_rx before recursive call so child can use it
                            self.tool_rx = Some(tool_rx);
                            let response = self.handle_decompose_with_response(&task, &request.request_id).await;
                            // Take it back after child returns
                            tool_rx = self.tool_rx.take()
                                .context("Tool receiver lost during decompose")?;
                            // Send response back
                            let _ = response_tx.send(response);
                        }
                        ToolCall::Implement { task } => {
                            // Implement can run concurrently
                            // Spawn a task to handle it
                            let task_clone = task.clone();
                            let request_id = request.request_id.clone();
                            let response = self.handle_implement_with_response(&task_clone, &request_id).await;
                            // Send response back immediately (blocking call completed)
                            let _ = response_tx.send(response);
                        }
                        ToolCall::Complete { success, message } => {
                            // Send acknowledgment response
                            let response = ToolResponse::success(
                                request.request_id.clone(),
                                message.clone().unwrap_or_else(|| "Orchestration complete".to_string()),
                            );
                            let _ = response_tx.send(response);
                            break TaskResult { success: *success, message: message.clone() };
                        }
                    }
                }

                // Also handle ACP messages from both clients
                Ok(msg) = self.acp_orchestrator.recv() => {
                    self.handle_acp_message(&msg, "orchestrator").await;
                }

                Ok(msg) = self.acp_worker.recv() => {
                    self.handle_acp_message(&msg, "worker").await;
                }
            }
        };

        // Restore tool_rx so it's available for subsequent calls
        self.tool_rx = Some(tool_rx);
        Ok(result)
    }

    /// Handle ACP messages and stream agent output
    async fn handle_acp_message(&self, msg: &serde_json::Value, agent_type: &str) {
        if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
            match method {
                "session/update" => {
                    if let Some(params) = msg.get("params") {
                        if let Some(update) = params.get("update") {
                            if let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
                                match session_update {
                                    "agent_message_chunk" => {
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
                                        // Log tool results to diagnose failures
                                        let title = update.get("title").and_then(|t| t.as_str()).unwrap_or("unknown");
                                        let is_error = update.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
                                        if is_error {
                                            let content = update.get("content").and_then(|c| {
                                                c.get("text").and_then(|t| t.as_str())
                                            }).unwrap_or("no error message");
                                            tracing::error!("❌ {} tool failed: {} - {}", agent_type, title, content);
                                        } else {
                                            tracing::debug!("✅ {} tool succeeded: {}", agent_type, title);
                                        }
                                    }
                                    "plan" => {
                                        if let Some(entries) = update.get("entries").and_then(|e| e.as_array()) {
                                            tracing::info!("📋 {} created plan with {} entries", agent_type, entries.len());
                                        }
                                    }
                                    _ => {
                                        // Ignore other session updates to reduce noise
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Ignore other methods to reduce noise
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
        tracing::info!("🔄 Decomposing task: {}", task);

        // 1. Spawn planner to create plan
        let planner_session = self.spawn_planner(task).await?;

        // 2. Wait for plan via ACP plan updates (with timeout)
        let plan = self.wait_for_plan(&planner_session).await.map_err(|e| {
            anyhow::anyhow!("{}", e)
        })?;

        let num_entries = plan.entries.len();
        tracing::info!("📋 Plan created with {} entries", num_entries);
        for (i, entry) in plan.entries.iter().enumerate() {
            tracing::info!("  {}. {}", i + 1, entry.content);
        }

        // 3. Push this plan level onto the stack for context tracking
        let tasks: Vec<String> = plan.entries.iter().map(|e| e.content.clone()).collect();
        self.plan_stack.push(PlanLevel {
            parent_task: task.to_string(),
            tasks: tasks.clone(),
            current_index: 0,
        });

        // 4. Spawn child orchestrator to execute the plan
        let tasks_list = tasks
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{}. {}", i + 1, t))
            .collect::<Vec<_>>()
            .join("\n");

        let child_prompt = format!(
            "You are an orchestrator. Complete these tasks:\n\n{}\n\nFor each task, decide if it's simple enough to implement() or needs to be decompose()d further. When all tasks are done, call complete(true).",
            tasks_list
        );

        let result = self.run_orchestrator(&child_prompt).await?;

        // 5. Pop this plan level when done
        self.plan_stack.pop();

        tracing::info!("✅ Decomposition complete: {:?}", result);

        Ok(format!(
            "Decomposed into {} subtasks and executed them. Result: {}",
            num_entries,
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
        tracing::info!("🔨 Implementing task: {}", task);

        // Spawn implementer
        let impl_session = self.spawn_implementer(task).await?;

        // Wait for implementer to finish by watching for session completion (with timeout)
        self.wait_for_session_complete(&impl_session).await.map_err(|e| {
            anyhow::anyhow!("{}", e)
        })?;

        // Advance the current index in the top plan level
        if let Some(level) = self.plan_stack.last_mut() {
            level.current_index += 1;
        }

        tracing::info!("✅ Implementation complete");

        Ok(format!("Task completed: {}", task))
    }

    /// Wait for a session to complete (agent finishes its turn) with timeout.
    ///
    /// Returns an error if the session does not complete within the configured timeout.
    async fn wait_for_session_complete(&mut self, session_id: &str) -> Result<(), OrchestratorError> {
        let timeout_duration = self.timeout_config.session_complete_timeout;

        match tokio::time::timeout(timeout_duration, self.wait_for_session_complete_inner(session_id)).await {
            Ok(result) => result.map_err(OrchestratorError::from),
            Err(_elapsed) => {
                tracing::error!(
                    "⏰ Timeout waiting for session completion after {:?} (session: {})",
                    timeout_duration,
                    session_id
                );
                Err(OrchestratorError::Timeout {
                    operation: TimeoutOperation::WaitForSessionComplete,
                    duration: timeout_duration,
                    context: Some(format!("session_id: {}", session_id)),
                })
            }
        }
    }

    /// Inner implementation of wait_for_session_complete without timeout.
    async fn wait_for_session_complete_inner(&mut self, session_id: &str) -> Result<()> {
        loop {
            let msg = self.acp_worker.recv().await?;

            // Check for session/update with completion signal
            if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                if method == "session/update" {
                    if let Some(params) = msg.get("params") {
                        if params.get("sessionId").and_then(|v| v.as_str()) == Some(session_id) {
                            if let Some(update) = params.get("update") {
                                if let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
                                    // Log progress for debugging
                                    match session_update {
                                        "tool_call" => {
                                            if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                                                tracing::info!("🔧 implementer tool call: {}", title);
                                            }
                                        }
                                        "tool_result" => {
                                            // Log tool results to diagnose failures
                                            let title = update.get("title").and_then(|t| t.as_str()).unwrap_or("unknown");
                                            let is_error = update.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
                                            if is_error {
                                                let content = update.get("content").and_then(|c| {
                                                    c.get("text").and_then(|t| t.as_str())
                                                }).unwrap_or("no error message");
                                                tracing::error!("❌ implementer tool failed: {} - {}", title, content);
                                            } else {
                                                tracing::debug!("✅ implementer tool succeeded: {}", title);
                                            }
                                        }
                                        "agent_message_chunk" => {
                                            // Stream agent messages to stdout
                                            if let Some(content) = update.get("content") {
                                                if let Some(text) = content.get("text").and_then(|t| t.as_str()) {
                                                    print!("{}", text);
                                                    std::io::Write::flush(&mut std::io::stdout()).ok();
                                                }
                                            }
                                        }
                                        "agent_turn_finished" | "session_finished" => {
                                            // Session is complete
                                            return Ok(());
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Spawn an orchestrator agent
    async fn spawn_orchestrator(&mut self, prompt: &str) -> Result<String> {
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
        let mcp_servers = vec![json!({
            "name": "orchestrator",
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
            "{}\n\nYour task:\n{}",
            ORCHESTRATOR_PROMPT.trim(),
            prompt
        );

        self.acp_orchestrator
            .session_prompt(&response.session_id, &full_prompt)
            .await?;

        Ok(response.session_id)
    }

    /// Spawn a planner agent
    async fn spawn_planner(&mut self, task: &str) -> Result<String> {
        let cwd = std::env::current_dir()?
            .to_string_lossy()
            .to_string();

        let response = self
            .acp_worker
            .session_new(self.model_config.planner_model.as_str(), vec![], &cwd)
            .await?;

        let prompt = PLANNER_PROMPT.replace("{task}", task);

        self.acp_worker.session_prompt(&response.session_id, &prompt).await?;

        Ok(response.session_id)
    }

    /// Spawn an implementer agent
    async fn spawn_implementer(&mut self, task: &str) -> Result<String> {
        let cwd = std::env::current_dir()?
            .to_string_lossy()
            .to_string();

        let response = self
            .acp_worker
            .session_new(self.model_config.implementer_model.as_str(), vec![], &cwd)
            .await?;

        // Build sliding window context: last 2 tasks + next 2 tasks from current plan level
        let context = self.build_implementer_context();

        let prompt = IMPLEMENTER_PROMPT
            .replace("{task}", task)
            .replace("{user_goal}", &self.original_goal)
            .replace("{context}", &context);

        self.acp_worker.session_prompt(&response.session_id, &prompt).await?;

        Ok(response.session_id)
    }

    /// Build context string for implementer showing nearby tasks in the plan
    fn build_implementer_context(&self) -> String {
        let Some(level) = self.plan_stack.last() else {
            return "No plan context available.".to_string();
        };

        let idx = level.current_index;
        let tasks = &level.tasks;

        let mut parts = Vec::new();

        // Last 2 completed tasks
        let start = idx.saturating_sub(2);
        if start < idx {
            let previous: Vec<String> = tasks[start..idx]
                .iter()
                .map(|t| format!("  ✓ {}", t))
                .collect();
            if !previous.is_empty() {
                parts.push(format!("Previous tasks:\n{}", previous.join("\n")));
            }
        }

        // Next 2 upcoming tasks (after current)
        let end = (idx + 3).min(tasks.len());
        if idx + 1 < end {
            let upcoming: Vec<String> = tasks[idx + 1..end]
                .iter()
                .map(|t| format!("  → {}", t))
                .collect();
            if !upcoming.is_empty() {
                parts.push(format!("Upcoming tasks:\n{}", upcoming.join("\n")));
            }
        }

        if parts.is_empty() {
            "This is a standalone task.".to_string()
        } else {
            parts.join("\n\n")
        }
    }

    /// Wait for a plan from a planner session with timeout.
    ///
    /// Returns an error if the plan is not received within the configured timeout.
    async fn wait_for_plan(&mut self, session_id: &str) -> Result<Plan, OrchestratorError> {
        let timeout_duration = self.timeout_config.plan_timeout;

        match tokio::time::timeout(timeout_duration, self.wait_for_plan_inner(session_id)).await {
            Ok(result) => result.map_err(OrchestratorError::from),
            Err(_elapsed) => {
                tracing::error!(
                    "⏰ Timeout waiting for plan after {:?} (session: {})",
                    timeout_duration,
                    session_id
                );
                Err(OrchestratorError::Timeout {
                    operation: TimeoutOperation::WaitForPlan,
                    duration: timeout_duration,
                    context: Some(format!("session_id: {}", session_id)),
                })
            }
        }
    }

    /// Inner implementation of wait_for_plan without timeout.
    async fn wait_for_plan_inner(&mut self, session_id: &str) -> Result<Plan> {
        loop {
            let msg = self.acp_worker.recv().await?;

            // Check for session/update with plan
            if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                if method == "session/update" {
                    if let Some(params) = msg.get("params") {
                        if params.get("sessionId").and_then(|v| v.as_str()) == Some(session_id) {
                            if let Some(update) = params.get("update") {
                                if update.get("sessionUpdate").and_then(|v| v.as_str())
                                    == Some("plan")
                                {
                                    let entries: Vec<PlanEntry> =
                                        serde_json::from_value(update["entries"].clone())?;
                                    return Ok(Plan { entries });
                                }
                            }
                        }
                    }
                }
            }
        }
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

    tracing::debug!("Received tool request from MCP server: {}", line.trim());

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
    tracing::debug!("Sending response to MCP server: {}", response_json);
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    Ok(())
}
