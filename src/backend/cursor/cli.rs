//! Cursor CLI transport implementation.
//!
//! This module provides `CursorCliTransport`, a transport that uses Cursor's
//! non-interactive CLI mode (`agent --print`) instead of ACP. This is the
//! preferred transport for Cursor because ACP mode has broken MCP support.
//!
//! # CLI Mode vs ACP Mode
//!
//! - **CLI Mode** (`agent --print`): Properly loads MCP servers from `~/.cursor/mcp.json`
//! - **ACP Mode** (`agent acp`): MCP tools don't work (Cursor bug, no ETA for fix)
//!
//! # Output Format
//!
//! The CLI outputs newline-delimited JSON when using `--output-format stream-json`:
//! ```json
//! {"type":"text","content":"I'll help you..."}
//! {"type":"tool_use","id":"call_123","name":"paperboat-create_task","input":{...}}
//! {"type":"tool_result","tool_use_id":"call_123","content":"Task created"}
//! {"type":"result","subtype":"success","session_id":"abc-123",...}
//! ```
//!
//! # Session Resumption
//!
//! Multi-turn conversations use the `--resume <session_id>` flag to continue
//! a previous session. The session_id comes from the `result` message at the
//! end of each turn.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::permission::PermissionPolicy;
use crate::backend::transport::{
    AgentTransport, SessionConfig, SessionInfo, SessionUpdate, ToolResult,
};

/// Cursor CLI transport using `agent --print` mode.
///
/// This transport spawns separate `agent` processes for each prompt, using
/// the `--resume` flag for multi-turn conversations. It properly loads MCP
/// servers from `~/.cursor/mcp.json`, unlike the ACP transport.
///
/// # Lifecycle
///
/// 1. `initialize()` - No-op (CLI doesn't need initialization)
/// 2. `create_session()` - Stores config, returns placeholder session
/// 3. `send_prompt()` - Spawns `agent --print` process, streams output
/// 4. `take_notifications()` - Returns receiver for streaming updates
/// 5. `shutdown()` - Kills any running process
pub struct CursorCliTransport {
    /// Workspace directory for the agent
    workspace: PathBuf,
    /// Model to use for prompts
    model: String,
    /// Agent type for MCP server selection
    agent_type: String,
    /// Socket path for MCP communication (set from SessionConfig)
    socket_path: Option<String>,
    /// Unique ID for this session to prevent MCP server caching.
    /// Each session gets a unique ID that's used in the MCP server name.
    session_unique_id: String,
    /// Permission policy for tool filtering
    permission_policy: PermissionPolicy,
    /// Current session ID (from last successful prompt)
    current_session_id: Option<String>,
    /// Sender for streaming updates
    notification_tx: mpsc::Sender<SessionUpdate>,
    /// Receiver for streaming updates.
    /// Used by `recv()` for legacy polling, or can be taken once by `take_notifications()`.
    notification_rx: Option<mpsc::Receiver<SessionUpdate>>,
    /// Currently running agent process (if any)
    current_process: Option<Child>,
    /// Background task reading stdout
    reader_task: Option<JoinHandle<()>>,
}

/// Parsed output line from CLI stream-json format.
///
/// Cursor CLI outputs various message types:
/// - `thinking` with `subtype: "delta"` for streaming thought tokens
/// - `thinking` with `subtype: "completed"` when thinking is done
/// - `assistant` with the final response message
/// - `tool_use` for tool invocations
/// - `tool_result` for tool outputs
/// - `result` for completion status
/// - `system`, `user` for metadata (ignored)
///
/// Note: We use `#[serde(flatten)]` to capture any extra fields as `Value` since the
/// Cursor CLI may add new fields like `timestamp_ms`, `apiKeySource`, etc.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CliOutputLine {
    /// Text content from the agent (legacy format, may not be used by Cursor)
    Text {
        content: String,
        #[serde(flatten)]
        _extra: Value,
    },
    /// Thinking/reasoning tokens (streaming delta or completed)
    Thinking {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(flatten)]
        _extra: Value,
    },
    /// Final assistant response message
    Assistant {
        message: AssistantMessage,
        #[serde(flatten)]
        _extra: Value,
    },
    /// Tool use request
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(flatten)]
        _extra: Value,
    },
    /// Tool result
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
        #[serde(flatten)]
        _extra: Value,
    },
    /// Final result with session info
    Result {
        #[serde(default)]
        subtype: Option<String>,
        session_id: Option<String>,
        #[serde(default)]
        result: Option<String>,
        #[serde(flatten)]
        _extra: Value,
    },
    /// System initialization message (ignored)
    System {
        #[serde(flatten)]
        _extra: Value,
    },
    /// User message echo (ignored)
    User {
        #[serde(flatten)]
        _extra: Value,
    },
}

/// Assistant message structure from Cursor CLI
#[derive(Debug, Deserialize)]
struct AssistantMessage {
    #[allow(dead_code)]
    role: String,
    content: Vec<ContentBlock>,
}

/// Content block within a message
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    // Tool use blocks are handled separately via ToolUse type
    #[serde(other)]
    Other,
}

impl CursorCliTransport {
    /// Create a new CLI transport with the given agent type and permission policy.
    ///
    /// # Arguments
    ///
    /// * `agent_type` - The type of agent ("planner", "orchestrator", "implementer")
    /// * `permission_policy` - Policy for filtering tool calls
    #[must_use]
    pub fn new(agent_type: impl Into<String>, permission_policy: PermissionPolicy) -> Self {
        let (tx, rx) = mpsc::channel(100);
        // Generate a unique ID for this transport instance to prevent MCP server caching.
        // Use first 8 chars of UUID for readability while maintaining sufficient uniqueness.
        let session_unique_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        Self {
            workspace: PathBuf::new(),
            model: String::new(),
            agent_type: agent_type.into(),
            socket_path: None,
            session_unique_id,
            permission_policy,
            current_session_id: None,
            notification_tx: tx,
            notification_rx: Some(rx),
            current_process: None,
            reader_task: None,
        }
    }

    /// Create a transport for orchestrator agents.
    #[must_use]
    #[allow(dead_code)]
    pub fn for_orchestrator() -> Self {
        Self::new("orchestrator", PermissionPolicy::for_orchestrator())
    }

    /// Create a transport for planner agents.
    #[must_use]
    #[allow(dead_code)]
    pub fn for_planner() -> Self {
        Self::new("planner", PermissionPolicy::for_planner())
    }

    /// Create a transport for implementer agents.
    #[must_use]
    #[allow(dead_code)]
    pub fn for_implementer() -> Self {
        Self::new("implementer", PermissionPolicy::for_implementer())
    }

    /// Parse a line of stream-json output and convert to SessionUpdate.
    fn parse_output_line(&self, line: &str, session_id: &str) -> Option<SessionUpdate> {
        let parsed: Result<CliOutputLine, _> = serde_json::from_str(line);

        match parsed {
            Ok(CliOutputLine::Text { content, .. }) => Some(SessionUpdate::Text {
                session_id: session_id.to_string(),
                content,
            }),
            Ok(CliOutputLine::Thinking { subtype, text, .. }) => {
                // Only emit text for delta subtypes (streaming tokens)
                // Ignore "completed" subtypes
                if subtype.as_deref() == Some("delta") {
                    if let Some(text) = text {
                        return Some(SessionUpdate::Text {
                            session_id: session_id.to_string(),
                            content: text,
                        });
                    }
                }
                None
            }
            Ok(CliOutputLine::Assistant { message, .. }) => {
                // Extract text from the message content blocks
                let text: String = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        ContentBlock::Other => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");

                if !text.is_empty() {
                    Some(SessionUpdate::Text {
                        session_id: session_id.to_string(),
                        content: format!("\n\n**Response:**\n{}", text),
                    })
                } else {
                    None
                }
            }
            Ok(CliOutputLine::ToolUse {
                id, name, input, ..
            }) => {
                // Apply permission policy filtering
                if !self.permission_policy.should_allow(&name) {
                    tracing::warn!(
                        "🚫 CLI transport filtering tool call: {} (denied by policy)",
                        name
                    );
                    return None;
                }
                Some(SessionUpdate::ToolUse {
                    session_id: session_id.to_string(),
                    tool_use_id: id,
                    tool_name: name,
                    input,
                })
            }
            Ok(CliOutputLine::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            }) => Some(SessionUpdate::ToolResult {
                session_id: session_id.to_string(),
                tool_use_id,
                content,
                is_success: !is_error,
            }),
            Ok(CliOutputLine::Result {
                subtype,
                session_id: new_session_id,
                result,
                ..
            }) => {
                let success = subtype.as_deref() == Some("success");
                Some(SessionUpdate::Completion {
                    session_id: new_session_id.unwrap_or_else(|| session_id.to_string()),
                    result,
                    success,
                })
            }
            // Ignore system and user message echoes
            Ok(CliOutputLine::System { .. }) | Ok(CliOutputLine::User { .. }) => None,
            Err(e) => {
                // Log parse errors for debugging but don't fail
                tracing::trace!("Failed to parse CLI output line: {} - {}", line, e);
                // Return raw data for forward compatibility
                if let Ok(data) = serde_json::from_str::<Value>(line) {
                    Some(SessionUpdate::Raw {
                        session_id: Some(session_id.to_string()),
                        data,
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Convert a SessionUpdate to the JSON format expected by the legacy notification router.
    ///
    /// This mimics the ACP notification format so the existing routing code can handle
    /// messages from CLI transport the same way it handles ACP transport messages.
    ///
    /// The session handler (`handle_worker_session_message`) expects:
    /// - Text: `update.content.text` for the message content
    /// - ToolCall: `update.title` for the tool name
    /// - ToolResult: `update.title`, `update.isError`, `update.content.text`
    fn session_update_to_json(update: SessionUpdate) -> Value {
        match update {
            SessionUpdate::Text {
                session_id,
                content,
            } => {
                // Session handler expects: update.content.text
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "agent_message_chunk",
                            "content": {
                                "text": content
                            }
                        }
                    }
                })
            }
            SessionUpdate::ToolUse {
                session_id,
                tool_use_id,
                tool_name,
                input,
            } => {
                // Session handler expects: update.title for the tool name
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "tool_call",
                            "title": tool_name,
                            "tool_use_id": tool_use_id,
                            "input": input
                        }
                    }
                })
            }
            SessionUpdate::ToolResult {
                session_id,
                tool_use_id,
                content,
                is_success,
            } => {
                // Session handler expects: update.title, update.isError, update.content.text
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "tool_result",
                            "title": tool_use_id,
                            "isError": !is_success,
                            "content": {
                                "text": content
                            }
                        }
                    }
                })
            }
            SessionUpdate::Completion {
                session_id,
                result,
                success,
            } => {
                // Session handler expects: type "complete" to signal session end
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "complete",
                            "result": result,
                            "success": success
                        }
                    }
                })
            }
            SessionUpdate::Raw { session_id, data } => {
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id.unwrap_or_default(),
                        "update": data
                    }
                })
            }
        }
    }

    /// Spawn the agent process and start reading output.
    async fn spawn_agent(&mut self, prompt: &str) -> Result<()> {
        // Configure MCP for this agent type before spawning
        // This ensures the agent only sees the tools it should use
        // We use a unique suffix to prevent Cursor from caching MCP server processes
        // across different agent types/sessions
        if let Some(socket_path) = &self.socket_path {
            super::mcp_config::enable_mcp_for_agent(
                &self.agent_type,
                socket_path,
                Some(&self.session_unique_id),
            )?;
        } else {
            tracing::warn!(
                "No socket path configured for CLI transport - MCP tools may not work correctly"
            );
        }

        // Build command with all required flags
        let mut cmd = Command::new("agent");
        cmd.args([
            "--print",
            "--force",
            "--approve-mcps",
            "--trust",
            "--output-format",
            "stream-json",
        ]);

        // Add model if specified
        if !self.model.is_empty() {
            cmd.args(["--model", &self.model]);
        }

        // Add workspace if specified
        if self.workspace.as_os_str().len() > 0 {
            cmd.arg("--workspace").arg(&self.workspace);
        }

        // Add resume flag for multi-turn conversations
        if let Some(session_id) = &self.current_session_id {
            cmd.args(["--resume", session_id]);
        }

        // Add prompt after separator
        cmd.arg("--").arg(prompt);

        // Configure process
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // Capture stderr for logging
            .kill_on_drop(true);

        tracing::info!(
            "🚀 Spawning Cursor CLI: model={}, workspace={}, resume={:?}",
            self.model,
            self.workspace.display(),
            self.current_session_id
        );

        let mut child = cmd.spawn().context("Failed to spawn Cursor agent CLI")?;

        // Log the process ID for debugging
        if let Some(pid) = child.id() {
            tracing::info!("🚀 Cursor CLI started with PID: {}", pid);
        }

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take();

        // Spawn background task to log stderr
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::warn!("📛 CLI stderr: {}", line);
                    }
                }
            });
        }

        // Generate a temporary session ID for this prompt
        // (will be updated when we receive the result message)
        let temp_session_id = self
            .current_session_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let tx = self.notification_tx.clone();
        let permission_policy = self.permission_policy.clone();
        let agent_type = self.agent_type.clone();

        // Spawn background task to read stdout and emit SessionUpdates
        let reader_task = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            // Create a temporary transport for parsing (shares the permission policy)
            let parser = CursorCliTransport {
                workspace: PathBuf::new(),
                model: String::new(),
                agent_type,
                socket_path: None,
                session_unique_id: String::new(), // Not used for parsing
                permission_policy,
                current_session_id: None,
                notification_tx: tx.clone(),
                notification_rx: None,
                current_process: None,
                reader_task: None,
            };

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }

                // Log raw CLI output for debugging (at debug level to avoid noise in normal runs)
                tracing::debug!("📤 CLI output: {}", line);

                if let Some(update) = parser.parse_output_line(&line, &temp_session_id) {
                    // Also log the parsed update type for easier debugging
                    match &update {
                        crate::backend::transport::SessionUpdate::Text { content, .. } => {
                            tracing::trace!(
                                "  → Text: {}...",
                                &content.chars().take(50).collect::<String>()
                            );
                        }
                        crate::backend::transport::SessionUpdate::ToolUse { tool_name, .. } => {
                            tracing::debug!("  → Tool call: {}", tool_name);
                        }
                        crate::backend::transport::SessionUpdate::ToolResult {
                            tool_use_id,
                            is_success,
                            ..
                        } => {
                            tracing::debug!(
                                "  → Tool result: {} (success={})",
                                tool_use_id,
                                is_success
                            );
                        }
                        crate::backend::transport::SessionUpdate::Completion {
                            success, ..
                        } => {
                            tracing::info!("  → Completion: success={}", success);
                        }
                        crate::backend::transport::SessionUpdate::Raw { data, .. } => {
                            tracing::trace!("  → Raw: {:?}", data);
                        }
                    }
                    if tx.send(update).await.is_err() {
                        tracing::debug!("Notification receiver dropped, stopping CLI reader");
                        break;
                    }
                } else {
                    // Log lines that couldn't be parsed (might indicate issues)
                    tracing::warn!("📤 CLI unparseable output: {}", line);
                }
            }

            tracing::debug!("CLI reader task completed");
        });

        self.current_process = Some(child);
        self.reader_task = Some(reader_task);

        Ok(())
    }

    /// Kill the current process if running.
    async fn kill_process(&mut self) {
        // Abort reader task first
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }

        // Kill the process
        if let Some(mut child) = self.current_process.take() {
            if let Err(e) = child.kill().await {
                tracing::warn!("Failed to kill Cursor CLI process: {}", e);
            }
        }
    }
}

#[async_trait]
impl AgentTransport for CursorCliTransport {
    /// Initialize the transport.
    ///
    /// For CLI transport, this is a no-op since there's no persistent connection
    /// to establish. The actual agent process is spawned per-prompt.
    async fn initialize(&mut self) -> Result<()> {
        tracing::debug!("CLI transport initialized (no-op)");
        Ok(())
    }

    /// Create a new session with the given configuration.
    ///
    /// For CLI transport, this stores the configuration for later use and
    /// returns a placeholder session ID. The actual session is created
    /// when the first prompt is sent.
    async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo> {
        self.model = config.model;
        self.workspace = PathBuf::from(&config.workspace);

        // Extract socket path from MCP server config
        // The MCP servers config contains: {"args": ["--mcp-server", "--socket", "<path>"], ...}
        for server in &config.mcp_servers {
            if let Some(args) = server.get("args").and_then(|a| a.as_array()) {
                for (i, arg) in args.iter().enumerate() {
                    if arg.as_str() == Some("--socket") {
                        if let Some(socket) = args.get(i + 1).and_then(|s| s.as_str()) {
                            self.socket_path = Some(socket.to_string());
                            tracing::debug!("📦 Extracted socket path from MCP config: {}", socket);
                            break;
                        }
                    }
                }
            }
        }

        // Generate an initial session ID (may be replaced by CLI's session ID)
        let session_id = uuid::Uuid::new_v4().to_string();
        self.current_session_id = Some(session_id.clone());

        tracing::info!(
            "📋 CLI session configured: model={}, workspace={}",
            self.model,
            self.workspace.display()
        );

        Ok(SessionInfo::new(session_id))
    }

    /// Send a prompt to the agent.
    ///
    /// For CLI transport, this spawns a new `agent --print` process with
    /// the prompt. For subsequent prompts in the same session, the
    /// `--resume` flag is used to continue the conversation.
    async fn send_prompt(&mut self, _session_id: &str, prompt: &str) -> Result<()> {
        // Kill any existing process before starting a new one
        self.kill_process().await;

        // Spawn new process
        self.spawn_agent(prompt).await?;

        Ok(())
    }

    /// Take the notification receiver for streaming updates.
    ///
    /// Returns the receiver end of the channel that receives `SessionUpdate`
    /// messages. This can only be called once; subsequent calls return `None`.
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>> {
        self.notification_rx.take()
    }

    /// Respond to a tool call with the result.
    ///
    /// For CLI transport, this is a no-op because the CLI handles tool
    /// execution internally (MCP tools are invoked by the agent process
    /// itself, not by us).
    async fn respond_to_tool(
        &mut self,
        _session_id: &str,
        _tool_use_id: &str,
        _result: ToolResult,
    ) -> Result<()> {
        // CLI transport doesn't support responding to tool calls since the
        // agent process handles tool execution internally
        tracing::trace!("respond_to_tool is no-op for CLI transport");
        Ok(())
    }

    /// Shutdown the transport.
    ///
    /// Kills any running agent process and cleans up resources.
    async fn shutdown(&mut self) -> Result<()> {
        tracing::debug!("Shutting down Cursor CLI transport");
        self.kill_process().await;
        Ok(())
    }

    /// Receive the next raw notification (legacy compatibility).
    ///
    /// Receives a `SessionUpdate` from the notification channel and converts
    /// it to the raw JSON format expected by the legacy polling code.
    async fn recv(&mut self) -> Result<Value> {
        // Get the receiver - if it's been taken, return an error
        let rx = self.notification_rx.as_mut().ok_or_else(|| {
            anyhow::anyhow!(
                "Notification receiver has been taken. Use take_notifications() only once."
            )
        })?;

        // Wait for the next update
        let update = rx.recv().await.ok_or_else(|| {
            anyhow::anyhow!("CLI transport channel closed - agent process may have finished")
        })?;

        // Convert SessionUpdate to the JSON format expected by legacy code
        Ok(Self::session_update_to_json(update))
    }

    /// Returns the transport kind for this transport.
    ///
    /// CLI transport returns `TransportKind::Cli` to enable transport-specific
    /// behavior like creating unique sockets to prevent MCP server caching.
    fn kind(&self) -> crate::backend::transport::TransportKind {
        crate::backend::transport::TransportKind::Cli
    }

    /// Take the raw notification receiver (legacy compatibility).
    ///
    /// This converts the typed `SessionUpdate` channel to a raw `Value` channel
    /// for compatibility with the existing notification routing system.
    ///
    /// A bridge task is spawned that converts each `SessionUpdate` to the JSON
    /// format expected by the legacy code and forwards it to a new channel.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        // Take the typed receiver
        let typed_rx = self.notification_rx.take()?;

        // Create a new channel for raw JSON values
        let (tx, rx) = mpsc::channel::<Value>(100);

        // Spawn a bridge task that converts SessionUpdate to Value
        tokio::spawn(async move {
            let mut typed_rx = typed_rx;
            while let Some(update) = typed_rx.recv().await {
                // Convert SessionUpdate to the JSON format expected by legacy code
                let json = Self::session_update_to_json(update);
                if tx.send(json).await.is_err() {
                    // Receiver dropped, stop bridging
                    break;
                }
            }
            tracing::debug!("CLI transport notification bridge task ended");
        });

        Some(rx)
    }
}

impl Drop for CursorCliTransport {
    fn drop(&mut self) {
        // Abort reader task to prevent it from running after drop
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }

        // Initiate process termination (can't wait in Drop since it's not async)
        if let Some(ref mut child) = self.current_process {
            if let Err(e) = child.start_kill() {
                tracing::warn!("Failed to initiate kill of Cursor CLI process: {}", e);
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

    // ========================================================================
    // JSON Output Parsing Tests
    // ========================================================================

    #[test]
    fn test_parse_text_output() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"text","content":"Hello, world!"}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Text {
            session_id,
            content,
        }) = update
        {
            assert_eq!(session_id, "test-session");
            assert_eq!(content, "Hello, world!");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_parse_text_output_with_empty_content() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"text","content":""}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Text { content, .. }) = update {
            assert_eq!(content, "");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_parse_text_output_with_special_characters() {
        let transport = CursorCliTransport::for_implementer();
        // Test with escape sequences and unicode
        let line = r#"{"type":"text","content":"Hello\n\"World\"\t🚀"}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Text { content, .. }) = update {
            assert!(content.contains("Hello"));
            assert!(content.contains("World"));
            assert!(content.contains("🚀"));
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_parse_tool_use_output() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_use","id":"call_123","name":"str-replace-editor","input":{"path":"test.rs"}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::ToolUse {
            session_id,
            tool_use_id,
            tool_name,
            input,
        }) = update
        {
            assert_eq!(session_id, "test-session");
            assert_eq!(tool_use_id, "call_123");
            assert_eq!(tool_name, "str-replace-editor");
            assert_eq!(input["path"], "test.rs");
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_parse_tool_use_with_complex_input() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_use","id":"call_456","name":"launch-process","input":{"command":"echo test","cwd":"/tmp","wait":true}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::ToolUse { input, .. }) = update {
            assert_eq!(input["command"], "echo test");
            assert_eq!(input["cwd"], "/tmp");
            assert_eq!(input["wait"], true);
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_parse_tool_use_with_nested_input() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_use","id":"call_789","name":"save-file","input":{"path":"test.rs","content":"fn main() {}","options":{"overwrite":true}}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::ToolUse { input, .. }) = update {
            assert_eq!(input["path"], "test.rs");
            assert_eq!(input["options"]["overwrite"], true);
        } else {
            panic!("Expected ToolUse update");
        }
    }

    #[test]
    fn test_parse_tool_result_output() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_result","tool_use_id":"call_123","content":"File saved"}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::ToolResult {
            session_id,
            tool_use_id,
            content,
            is_success,
        }) = update
        {
            assert_eq!(session_id, "test-session");
            assert_eq!(tool_use_id, "call_123");
            assert_eq!(content, "File saved");
            assert!(is_success);
        } else {
            panic!("Expected ToolResult update");
        }
    }

    #[test]
    fn test_parse_tool_result_with_error() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_result","tool_use_id":"call_123","content":"File not found","is_error":true}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::ToolResult {
            content,
            is_success,
            ..
        }) = update
        {
            assert_eq!(content, "File not found");
            assert!(!is_success);
        } else {
            panic!("Expected ToolResult update");
        }
    }

    #[test]
    fn test_parse_tool_result_without_is_error_defaults_to_success() {
        let transport = CursorCliTransport::for_implementer();
        // No is_error field should default to success
        let line = r#"{"type":"tool_result","tool_use_id":"call_123","content":"Done"}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::ToolResult { is_success, .. }) = update {
            assert!(is_success);
        } else {
            panic!("Expected ToolResult update");
        }
    }

    #[test]
    fn test_parse_result_output() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"result","subtype":"success","session_id":"real-session-123","result":"Done"}"#;
        let update = transport.parse_output_line(line, "temp-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Completion {
            session_id,
            result,
            success,
        }) = update
        {
            assert_eq!(session_id, "real-session-123");
            assert_eq!(result, Some("Done".to_string()));
            assert!(success);
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_parse_result_output_failure() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"result","subtype":"error","session_id":"session-456","result":"Task failed"}"#;
        let update = transport.parse_output_line(line, "temp-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Completion {
            session_id,
            result,
            success,
        }) = update
        {
            assert_eq!(session_id, "session-456");
            assert_eq!(result, Some("Task failed".to_string()));
            assert!(!success);
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_parse_result_output_without_session_id_uses_temp() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"result","subtype":"success","result":"Done"}"#;
        let update = transport.parse_output_line(line, "temp-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Completion { session_id, .. }) = update {
            // Should fall back to temp session
            assert_eq!(session_id, "temp-session");
        } else {
            panic!("Expected Completion update");
        }
    }

    #[test]
    fn test_parse_result_output_without_result_field() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"result","subtype":"success","session_id":"session-123"}"#;
        let update = transport.parse_output_line(line, "temp-session");

        assert!(update.is_some());
        if let Some(SessionUpdate::Completion { result, .. }) = update {
            assert!(result.is_none());
        } else {
            panic!("Expected Completion update");
        }
    }

    // ========================================================================
    // Permission Policy Tests
    // ========================================================================

    #[test]
    fn test_permission_filtering_denies_tool() {
        // Orchestrator policy denies file editing tools
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"str-replace-editor","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Should be filtered out (returns None)
        assert!(update.is_none());
    }

    #[test]
    fn test_permission_filtering_allows_mcp_tool() {
        // Orchestrator policy allows spawn_agents MCP tool
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"spawn_agents","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Should be allowed
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_implementer_allows_file_editing() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_use","id":"call_123","name":"str-replace-editor","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Implementer should be allowed
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_planner_denies_file_editing() {
        let transport = CursorCliTransport::for_planner();
        let line = r#"{"type":"tool_use","id":"call_123","name":"str-replace-editor","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Planner should NOT be allowed to edit files
        assert!(update.is_none());
    }

    #[test]
    fn test_planner_allows_set_goal() {
        let transport = CursorCliTransport::for_planner();
        let line = r#"{"type":"tool_use","id":"call_123","name":"set_goal","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Planner should be allowed to use set_goal
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_planner_allows_create_task() {
        let transport = CursorCliTransport::for_planner();
        let line = r#"{"type":"tool_use","id":"call_123","name":"create_task","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_orchestrator_allows_decompose() {
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"decompose","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_orchestrator_allows_skip_tasks() {
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"skip_tasks","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_orchestrator_allows_list_tasks() {
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"list_tasks","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_implementer_allows_complete() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"tool_use","id":"call_123","name":"complete","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::ToolUse { .. })));
    }

    #[test]
    fn test_orchestrator_denies_save_file() {
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"save-file","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_none());
    }

    #[test]
    fn test_orchestrator_denies_remove_files() {
        let transport = CursorCliTransport::for_orchestrator();
        let line = r#"{"type":"tool_use","id":"call_123","name":"remove-files","input":{}}"#;
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_none());
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[test]
    fn test_parse_unknown_json_returns_raw() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"unknown_type","data":"something"}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Unknown types should return Raw
        assert!(update.is_some());
        if let Some(SessionUpdate::Raw { session_id, data }) = update {
            assert_eq!(session_id, Some("test-session".to_string()));
            assert_eq!(data["type"], "unknown_type");
        } else {
            panic!("Expected Raw update");
        }
    }

    #[test]
    fn test_parse_invalid_json_returns_none() {
        let transport = CursorCliTransport::for_implementer();
        let line = "not valid json";
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_none());
    }

    #[test]
    fn test_parse_empty_line_returns_none() {
        let transport = CursorCliTransport::for_implementer();
        let update = transport.parse_output_line("", "test-session");

        assert!(update.is_none());
    }

    #[test]
    fn test_parse_whitespace_only_returns_none() {
        let transport = CursorCliTransport::for_implementer();
        let update = transport.parse_output_line("   \t  \n  ", "test-session");

        assert!(update.is_none());
    }

    #[test]
    fn test_parse_json_without_type_returns_raw() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"data":"something","foo":"bar"}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Should return Raw for valid JSON without recognized type
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Raw { .. })));
    }

    #[test]
    fn test_parse_truncated_json_returns_none() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"text","content":"Hello"#; // Missing closing brace
        let update = transport.parse_output_line(line, "test-session");

        assert!(update.is_none());
    }

    #[test]
    fn test_parse_json_array_returns_raw() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"[{"type":"text","content":"Hello"}]"#;
        let update = transport.parse_output_line(line, "test-session");

        // Arrays are valid JSON but don't match expected structure, so return Raw
        assert!(update.is_some());
        assert!(matches!(update, Some(SessionUpdate::Raw { .. })));
    }

    #[test]
    fn test_parse_thinking_delta_output() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"thinking","subtype":"delta","text":"reasoning...","session_id":"abc"}"#;
        let update = transport.parse_output_line(line, "test-session");

        if let Some(SessionUpdate::Text { session_id, content }) = update {
            assert_eq!(session_id, "test-session");
            assert_eq!(content, "reasoning...");
        } else {
            panic!("Expected Text update for thinking delta");
        }
    }

    #[test]
    fn test_parse_thinking_completed_is_ignored() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"thinking","subtype":"completed","session_id":"abc"}"#;
        let update = transport.parse_output_line(line, "test-session");

        // Completed thinking events should be ignored
        assert!(update.is_none());
    }

    #[test]
    fn test_parse_assistant_message() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello!"}]},"session_id":"abc"}"#;
        let update = transport.parse_output_line(line, "test-session");

        if let Some(SessionUpdate::Text { session_id, content }) = update {
            assert_eq!(session_id, "test-session");
            assert!(content.contains("Hello!"));
        } else {
            panic!("Expected Text update for assistant message");
        }
    }

    #[test]
    fn test_parse_system_message_is_ignored() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"system","subtype":"init","session_id":"abc"}"#;
        let update = transport.parse_output_line(line, "test-session");

        // System messages should be ignored
        assert!(update.is_none());
    }

    #[test]
    fn test_parse_user_message_is_ignored() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"test"}]},"session_id":"abc"}"#;
        let update = transport.parse_output_line(line, "test-session");

        // User message echoes should be ignored
        assert!(update.is_none());
    }

    // ========================================================================
    // Constructor and Factory Method Tests
    // ========================================================================

    #[test]
    fn test_cli_transport_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CursorCliTransport>();
    }

    #[test]
    fn test_new_creates_channel() {
        let transport = CursorCliTransport::new("test", PermissionPolicy::allow_all());
        assert!(transport.notification_rx.is_some());
        assert!(transport.current_session_id.is_none());
        assert!(transport.current_process.is_none());
    }

    #[test]
    fn test_for_orchestrator_creates_transport() {
        let transport = CursorCliTransport::for_orchestrator();
        assert!(transport.notification_rx.is_some());
    }

    #[test]
    fn test_for_planner_creates_transport() {
        let transport = CursorCliTransport::for_planner();
        assert!(transport.notification_rx.is_some());
    }

    #[test]
    fn test_for_implementer_creates_transport() {
        let transport = CursorCliTransport::for_implementer();
        assert!(transport.notification_rx.is_some());
    }

    #[test]
    fn test_new_initializes_empty_workspace() {
        let transport = CursorCliTransport::new("test", PermissionPolicy::allow_all());
        assert!(transport.workspace.as_os_str().is_empty());
    }

    #[test]
    fn test_new_initializes_empty_model() {
        let transport = CursorCliTransport::new("test", PermissionPolicy::allow_all());
        assert!(transport.model.is_empty());
    }

    // ========================================================================
    // AgentTransport Trait Implementation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_initialize_succeeds() {
        let mut transport = CursorCliTransport::for_implementer();
        // Initialize is a no-op for CLI transport
        let result = transport.initialize().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_session_stores_config() {
        let mut transport = CursorCliTransport::for_implementer();
        transport.initialize().await.unwrap();

        let config = SessionConfig::new("test-model", "/test/workspace");
        let session = transport.create_session(config).await.unwrap();

        // Session ID should be generated
        assert!(!session.session_id.is_empty());
        // Config should be stored
        assert_eq!(transport.model, "test-model");
        assert_eq!(transport.workspace.to_string_lossy(), "/test/workspace");
    }

    #[tokio::test]
    async fn test_create_session_generates_uuid() {
        let mut transport = CursorCliTransport::for_implementer();
        transport.initialize().await.unwrap();

        let config = SessionConfig::new("test-model", "/test/workspace");
        let session = transport.create_session(config).await.unwrap();

        // Should be a valid UUID format (basic check)
        assert!(session.session_id.contains('-'));
        assert_eq!(session.session_id.len(), 36); // UUID format: 8-4-4-4-12
    }

    #[tokio::test]
    async fn test_create_session_extracts_socket_path_from_mcp_config() {
        let mut transport = CursorCliTransport::for_implementer();
        let config = SessionConfig::new("test-model", "/test/workspace").with_mcp_servers(vec![
            json!({
                "name": "paperboat",
                "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
            }),
            json!({
                "name": "other",
                "args": ["--flag", "value"]
            }),
        ]);

        transport.create_session(config).await.unwrap();

        assert_eq!(
            transport.socket_path.as_deref(),
            Some("/tmp/paperboat.sock")
        );
    }

    #[tokio::test]
    async fn test_take_notifications_returns_once() {
        let mut transport = CursorCliTransport::for_implementer();

        // First call should return Some
        let rx1 = transport.take_notifications();
        assert!(rx1.is_some());

        // Second call should return None
        let rx2 = transport.take_notifications();
        assert!(rx2.is_none());
    }

    #[tokio::test]
    async fn test_respond_to_tool_is_noop() {
        let mut transport = CursorCliTransport::for_implementer();

        // CLI transport doesn't support responding to tool calls
        let result = transport
            .respond_to_tool(
                "session-id",
                "tool-use-id",
                ToolResult::success("test result"),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_shutdown_succeeds() {
        let mut transport = CursorCliTransport::for_implementer();
        let result = transport.shutdown().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_recv_fails_after_take_notifications() {
        let mut transport = CursorCliTransport::for_implementer();
        assert!(transport.take_notifications().is_some());

        let err = transport.recv().await.unwrap_err();
        assert!(err
            .to_string()
            .contains("Notification receiver has been taken"));
    }

    #[tokio::test]
    async fn test_take_notification_rx_bridges_session_updates() {
        let mut transport = CursorCliTransport::for_implementer();
        let mut rx = transport.take_notification_rx().unwrap();

        transport
            .notification_tx
            .send(SessionUpdate::ToolResult {
                session_id: "session-123".to_string(),
                tool_use_id: "tool-1".to_string(),
                content: "done".to_string(),
                is_success: false,
            })
            .await
            .unwrap();

        let bridged = rx.recv().await.unwrap();
        assert_eq!(bridged["method"], "session/update");
        assert_eq!(bridged["params"]["session_id"], "session-123");
        assert_eq!(bridged["params"]["update"]["type"], "tool_result");
        assert_eq!(bridged["params"]["update"]["isError"], true);
        assert_eq!(bridged["params"]["update"]["content"]["text"], "done");
    }

    // ========================================================================
    // Session Update Conversion Tests
    // ========================================================================

    #[test]
    fn test_session_update_text_conversion_preserves_session_id() {
        let transport = CursorCliTransport::for_implementer();
        let line = r#"{"type":"text","content":"test"}"#;
        let update = transport.parse_output_line(line, "my-custom-session");

        if let Some(SessionUpdate::Text { session_id, .. }) = update {
            assert_eq!(session_id, "my-custom-session");
        } else {
            panic!("Expected Text update");
        }
    }

    #[test]
    fn test_multiple_outputs_parsed_correctly() {
        let transport = CursorCliTransport::for_implementer();

        let lines = vec![
            r#"{"type":"text","content":"Starting..."}"#,
            r#"{"type":"tool_use","id":"call_1","name":"view","input":{"path":"test.rs"}}"#,
            r#"{"type":"tool_result","tool_use_id":"call_1","content":"file content"}"#,
            r#"{"type":"text","content":"Done!"}"#,
            r#"{"type":"result","subtype":"success","session_id":"sess-123"}"#,
        ];

        let updates: Vec<_> = lines
            .iter()
            .filter_map(|line| transport.parse_output_line(line, "test-session"))
            .collect();

        assert_eq!(updates.len(), 5);
        assert!(matches!(updates[0], SessionUpdate::Text { .. }));
        assert!(matches!(updates[1], SessionUpdate::ToolUse { .. }));
        assert!(matches!(updates[2], SessionUpdate::ToolResult { .. }));
        assert!(matches!(updates[3], SessionUpdate::Text { .. }));
        assert!(matches!(updates[4], SessionUpdate::Completion { .. }));
    }

    #[tokio::test]
    #[serial]
    async fn test_send_prompt_propagates_agent_spawn_failure_without_socket_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _path = EnvGuard::set("PATH", temp_dir.path().to_str().unwrap());

        let mut transport = CursorCliTransport::for_implementer();
        transport
            .create_session(SessionConfig::new("test-model", "/tmp/workspace"))
            .await
            .unwrap();

        let err = transport
            .send_prompt("session-id", "hello")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to spawn Cursor agent CLI"));
    }

    #[tokio::test]
    #[serial]
    async fn test_send_prompt_propagates_mcp_enable_failure_with_socket_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp_dir.path().to_str().unwrap());
        let _path = EnvGuard::set("PATH", temp_dir.path().to_str().unwrap());

        let mut transport = CursorCliTransport::for_implementer();
        transport
            .create_session(
                SessionConfig::new("test-model", "/tmp/workspace").with_mcp_servers(vec![json!({
                    "name": "paperboat",
                    "args": ["--mcp-server", "--socket", "/tmp/paperboat.sock"]
                })]),
            )
            .await
            .unwrap();

        let err = transport
            .send_prompt("session-id", "hello")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to run 'agent mcp enable'"));
    }
}
