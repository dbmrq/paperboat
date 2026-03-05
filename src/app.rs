//! Main orchestrator application

use crate::acp::AcpClient;
use crate::mcp_server::ToolCall;
use crate::types::{Plan, PlanEntry, TaskResult};
use anyhow::{Context, Result};
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

pub struct App {
    acp_orchestrator: AcpClient,
    acp_worker: AcpClient,
    socket_path: Option<PathBuf>,
    tool_rx: Option<mpsc::Receiver<ToolCall>>,
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

    pub async fn new() -> Result<Self> {
        // Set up orchestrator cache directory with removed tools
        let orchestrator_cache = Self::setup_orchestrator_cache()?;

        // Orchestrator uses a custom cache directory with editing tools removed
        let mut acp_orchestrator = AcpClient::spawn(Some(&orchestrator_cache)).await?;
        acp_orchestrator.initialize().await?;

        // Workers use the default cache directory with all tools available
        let mut acp_worker = AcpClient::spawn(None).await?;
        acp_worker.initialize().await?;

        Ok(Self {
            acp_orchestrator,
            acp_worker,
            socket_path: None,
            tool_rx: None,
        })
    }

    /// Run the orchestrator with a goal
    pub async fn run(&mut self, goal: &str) -> Result<TaskResult> {
        tracing::info!("Starting with goal: {}", goal);

        // Set up Unix socket for MCP server communication
        let socket_path = self.setup_socket().await?;

        // Spawn orchestrator with MCP tools
        let result = self.run_orchestrator(goal).await?;

        // Clean up socket
        if let Err(e) = std::fs::remove_file(&socket_path) {
            tracing::warn!("Failed to remove socket file: {}", e);
        }

        Ok(result)
    }

    /// Set up Unix socket for MCP server communication
    async fn setup_socket(&mut self) -> Result<PathBuf> {
        let socket_path = std::env::temp_dir().join(format!("villalobos-{}.sock", uuid::Uuid::new_v4()));

        // Remove socket file if it exists
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)
            .context("Failed to bind Unix socket")?;

        tracing::info!("Unix socket listening at: {:?}", socket_path);

        // Spawn task to accept connections and forward tool calls
        let (tool_tx, tool_rx) = mpsc::channel(100);

        tokio::spawn(async move {
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
                        tracing::error!("Failed to accept connection: {}", e);
                        break;
                    }
                }
            }
        });

        self.socket_path = Some(socket_path.clone());
        self.tool_rx = Some(tool_rx);

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
                // Handle tool calls from MCP server
                Some(tool_call) = tool_rx.recv() => {
                    tracing::info!("Received tool call: {:?}", tool_call);

                    match tool_call {
                        ToolCall::Decompose { task } => {
                            // Restore tool_rx before recursive call so child can use it
                            self.tool_rx = Some(tool_rx);
                            self.handle_decompose(&task).await?;
                            // Take it back after child returns
                            tool_rx = self.tool_rx.take()
                                .context("Tool receiver lost during decompose")?;
                        }
                        ToolCall::Implement { task } => {
                            self.handle_implement(&task).await?;
                        }
                        ToolCall::Complete { success, message } => {
                            break TaskResult { success, message };
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

    /// Handle decompose tool call
    async fn handle_decompose(&mut self, task: &str) -> Result<()> {
        tracing::info!("🔄 Decomposing task: {}", task);

        // 1. Spawn planner to create plan
        let planner_session = self.spawn_planner(task).await?;

        // 2. Wait for plan via ACP plan updates
        let plan = self.wait_for_plan(&planner_session).await?;

        tracing::info!("📋 Plan created with {} entries", plan.entries.len());
        for (i, entry) in plan.entries.iter().enumerate() {
            tracing::info!("  {}. {}", i + 1, entry.content);
        }

        // 3. Spawn child orchestrator to execute the plan
        let tasks_list = plan
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| format!("{}. {}", i + 1, e.content))
            .collect::<Vec<_>>()
            .join("\n");

        let child_prompt = format!(
            "You are an orchestrator. Complete these tasks:\n\n{}\n\nFor each task, decide if it's simple enough to implement() or needs to be decompose()d further. When all tasks are done, call complete(true).",
            tasks_list
        );

        let result = self.run_orchestrator(&child_prompt).await?;

        tracing::info!("✅ Decomposition complete: {:?}", result);

        Ok(())
    }

    /// Handle implement tool call
    async fn handle_implement(&mut self, task: &str) -> Result<()> {
        tracing::info!("🔨 Implementing task: {}", task);

        // Spawn implementer
        let impl_session = self.spawn_implementer(task).await?;

        // Wait for implementer to finish by watching for session completion
        self.wait_for_session_complete(&impl_session).await?;

        tracing::info!("✅ Implementation complete");

        Ok(())
    }

    /// Wait for a session to complete (agent finishes its turn)
    async fn wait_for_session_complete(&mut self, session_id: &str) -> Result<()> {
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
            .session_new("claude-3-5-sonnet-20241022", mcp_servers, &cwd)
            .await?;

        let full_prompt = format!(
            "You are an orchestrator agent. Your job is to coordinate the completion of tasks by delegating work to specialized agents.

CRITICAL RULES:
- You MUST use ONLY the MCP tools (decompose, implement, complete)
- You MUST NOT edit files, run commands, or write code directly
- You MUST delegate all implementation work via implement()
- You MUST delegate complex planning via decompose()
- You MUST call complete() when all work is delegated and done

Your task:
{}",
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
            .session_new("claude-3-5-sonnet-20241022", vec![], &cwd)
            .await?;

        let prompt = format!(
            "You are a task planner. Break down this task into concrete, implementable subtasks:

Task: {}

Create a detailed plan. Each subtask should be:
- Clear and actionable
- As small as reasonably possible
- Properly ordered

IMPORTANT: Work autonomously. Do NOT ask questions or wait for user input. Create the plan immediately using your built-in planning capabilities.",
            task
        );

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
            .session_new("claude-3-5-sonnet-20241022", vec![], &cwd)
            .await?;

        let prompt = format!(
            "You are an implementer. Implement this task:

Task: {}

Write the code, tests, and documentation needed to complete this task.

IMPORTANT: Work autonomously. Do NOT ask questions or wait for user input. Make reasonable decisions and implement the task immediately.",
            task
        );

        self.acp_worker.session_prompt(&response.session_id, &prompt).await?;

        Ok(response.session_id)
    }

    /// Wait for a plan from a planner session
    async fn wait_for_plan(&mut self, session_id: &str) -> Result<Plan> {
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

/// Handle an MCP connection from the MCP server
async fn handle_mcp_connection(
    stream: UnixStream,
    tool_tx: mpsc::Sender<ToolCall>,
) -> Result<()> {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        tracing::debug!("Received tool call from MCP server: {}", line);

        let tool_call: ToolCall = serde_json::from_str(&line)
            .context("Failed to parse tool call")?;

        tool_tx.send(tool_call).await
            .context("Failed to send tool call to app")?;
    }

    Ok(())
}
