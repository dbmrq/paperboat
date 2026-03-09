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
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CliOutputLine {
    /// Text content from the agent
    Text { content: String },
    /// Tool use request
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Tool result
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// Final result with session info
    Result {
        #[serde(default)]
        subtype: Option<String>,
        session_id: Option<String>,
        #[serde(default)]
        result: Option<String>,
    },
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
        Self {
            workspace: PathBuf::new(),
            model: String::new(),
            agent_type: agent_type.into(),
            socket_path: None,
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
            Ok(CliOutputLine::Text { content }) => Some(SessionUpdate::Text {
                session_id: session_id.to_string(),
                content,
            }),
            Ok(CliOutputLine::ToolUse { id, name, input }) => {
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
            }) => {
                let success = subtype.as_deref() == Some("success");
                Some(SessionUpdate::Completion {
                    session_id: new_session_id.unwrap_or_else(|| session_id.to_string()),
                    result,
                    success,
                })
            }
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

    /// Spawn the agent process and start reading output.
    async fn spawn_agent(&mut self, prompt: &str) -> Result<()> {
        // Configure MCP for this agent type before spawning
        // This ensures the agent only sees the tools it should use
        if let Some(socket_path) = &self.socket_path {
            super::mcp_config::enable_mcp_for_agent(&self.agent_type, socket_path)?;
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
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        tracing::debug!(
            "🚀 Spawning Cursor CLI: model={}, workspace={}, resume={:?}",
            self.model,
            self.workspace.display(),
            self.current_session_id
        );

        let mut child = cmd.spawn().context("Failed to spawn Cursor agent CLI")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;

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

                if let Some(update) = parser.parse_output_line(&line, &temp_session_id) {
                    if tx.send(update).await.is_err() {
                        tracing::debug!("Notification receiver dropped, stopping CLI reader");
                        break;
                    }
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
        // This mimics the ACP notification format
        let json = match update {
            SessionUpdate::Text {
                session_id,
                content,
            } => {
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "agent_message_chunk",
                            "data": content
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
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "tool_call",
                            "data": {
                                "tool_use_id": tool_use_id,
                                "tool_name": tool_name,
                                "input": input
                            }
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
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "tool_result",
                            "data": {
                                "tool_use_id": tool_use_id,
                                "content": content,
                                "is_success": is_success
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
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id,
                        "update": {
                            "type": "complete",
                            "data": {
                                "result": result,
                                "success": success
                            }
                        }
                    }
                })
            }
            SessionUpdate::Raw { session_id, data } => {
                // For raw updates, wrap them in session/update format
                json!({
                    "method": "session/update",
                    "params": {
                        "session_id": session_id.unwrap_or_default(),
                        "update": data
                    }
                })
            }
        };

        Ok(json)
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
}
