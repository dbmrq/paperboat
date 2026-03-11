//! Self-improvement runner module.
//!
//! This module handles spawning and running the self-improvement agent
//! after a successful paperboat run in its own repository.
//!
//! The self-improvement feature only runs when paperboat detects it is
//! in its own repository, using the `is_paperboat_repository()` check.
//! The agent has full implementer permissions to analyze logs and make
//! small, safe code changes.
//!
//! # Configuration
//!
//! Self-improvement is enabled by default (opt-out). Control via:
//! - `PAPERBOAT_SELF_IMPROVE=0` environment variable to disable
//! - `.paperboat/self-improve.toml` config file with `enabled = false`

use crate::acp::{AcpClient, AcpClientTrait, SessionMode};
use crate::agents::{get_prompt, get_tool_config};
use crate::app::retry::{retry_async, RetryConfig};
use crate::ipc::{IpcAddress, IpcListener, IpcStream};
use crate::logging::{AgentWriter, LogScope};
use crate::tasks::TaskManager;
use crate::types::TaskResult;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use super::context_builder::build_self_improvement_context;
use super::detection::is_paperboat_repository;
use super::is_self_improvement_enabled;

/// Result of a self-improvement run.
#[derive(Debug)]
pub struct SelfImprovementOutcome {
    /// Whether the self-improvement run was successful.
    pub success: bool,
    /// Message from the self-improver agent.
    pub message: Option<String>,
    /// Number of changes made (if any).
    #[allow(dead_code)]
    pub changes_made: usize,
}

/// Configuration for the self-improvement runner.
#[derive(Debug, Clone)]
pub struct SelfImprovementConfig {
    /// Timeout for the self-improvement session.
    pub session_timeout: Duration,
    /// Request timeout for ACP operations.
    pub request_timeout: Duration,
    /// Model to use for self-improvement.
    pub model: String,
}

impl Default for SelfImprovementConfig {
    fn default() -> Self {
        Self {
            session_timeout: Duration::from_secs(300), // 5 minutes
            request_timeout: Duration::from_secs(30),
            model: "claude-sonnet-4-20250514".to_string(),
        }
    }
}

/// Run self-improvement if conditions are met.
///
/// This is the main entry point called from main.rs after a successful run.
/// It checks all preconditions and spawns the self-improver agent if appropriate.
///
/// Self-improvement only runs when:
/// 1. The feature is enabled via `PAPERBOAT_SELF_IMPROVEMENT=1`
/// 2. The run was successful
/// 3. We're running in the paperboat repository
///
/// # Arguments
///
/// * `run_dir` - Path to the completed run's log directory
/// * `result` - The final task result from the main run
/// * `task_manager` - Reference to the `TaskManager` with final state
///
/// # Returns
///
/// * `Ok(Some(outcome))` - Self-improvement ran and completed
/// * `Ok(None)` - Self-improvement was skipped (disabled, not in paperboat repo, etc.)
/// * `Err(e)` - Self-improvement failed
///
/// # Errors
///
/// This function is designed to be resilient. Errors are logged but should
/// not cause the main application to fail.
pub async fn maybe_run_self_improvement(
    run_dir: &Path,
    result: &TaskResult,
    task_manager: &TaskManager,
) -> Result<Option<SelfImprovementOutcome>> {
    // Check if self-improvement is enabled
    if !is_self_improvement_enabled() {
        tracing::debug!("Self-improvement is disabled");
        return Ok(None);
    }

    // Check if the run was successful (only improve on success or partial success)
    if !result.success {
        tracing::debug!("Run failed, skipping self-improvement");
        return Ok(None);
    }

    // Check if we're in the paperboat repository
    if !is_paperboat_repository() {
        tracing::debug!("Not in paperboat repository, skipping self-improvement");
        return Ok(None);
    }

    tracing::info!("🔄 Starting self-improvement phase...");

    // Build context for the self-improver
    let context = build_self_improvement_context(run_dir, result, task_manager)
        .await
        .context("Failed to build self-improvement context")?;

    // Run the self-improvement agent with full implementer permissions
    let config = SelfImprovementConfig::default();
    let outcome = run_self_improver(run_dir, &context, &config).await?;

    Ok(Some(outcome))
}

/// Run the self-improvement agent with full editing permissions.
async fn run_self_improver(
    run_dir: &Path,
    context: &str,
    config: &SelfImprovementConfig,
) -> Result<SelfImprovementOutcome> {
    // Create the self-improvement scope for logging
    let (event_tx, _) = broadcast::channel(100);
    let self_improve_dir = run_dir.join("self-improve");
    std::fs::create_dir_all(&self_improve_dir)?;
    let scope = LogScope::new(self_improve_dir.clone(), event_tx, 0);

    // Create writer for the self-improver agent
    let mut writer = scope
        .self_improver_writer()
        .await
        .context("Failed to create self-improver writer")?;

    // Build the task description
    let task = build_self_improvement_task(run_dir);

    // Get the self-improver prompt template
    let prompt_template = get_prompt("selfimprover")
        .context("selfimprover prompt not found - ensure prompts/selfimprover.txt exists")?;

    // Build the full prompt
    let prompt = prompt_template
        .replace("{task}", &task)
        .replace("{context}", context);

    // Get removed tools for selfimprover (defaults to implementer config)
    let tool_config = get_tool_config("selfimprover");
    let removed_tools: Vec<String> = tool_config
        .removed_auggie_tools
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    // Log the prompt
    let _ = writer.write_header_with_prompt(&task, &prompt).await;

    // Spawn and run the agent with implementer-level tools
    let outcome =
        spawn_and_run_agent(&prompt, &removed_tools, "implementer", config, &mut writer).await?;

    // Finalize the log
    let _ = writer.finalize(outcome.success).await;

    Ok(outcome)
}

/// Build the task description for the self-improvement agent.
fn build_self_improvement_task(run_dir: &Path) -> String {
    format!(
        r"Analyze the completed paperboat run and implement improvements.

**Run Directory**: `{}`

Your mission is to:
1. Read the log files in the run directory
2. Identify patterns that could be improved
3. Make small, safe, incremental changes
4. Run `cargo check` and `cargo test` to verify changes

Focus on:
- Error messages that could be clearer
- Prompts that confused agents
- Edge cases that weren't handled well
- Documentation gaps

Remember: Make only small, safe changes. No core refactors.",
        run_dir.display()
    )
}

/// Message sent from socket handler to signal completion.
struct CompletionSignal {
    success: bool,
    message: Option<String>,
}

/// Spawn the ACP client and run the self-improver agent.
///
/// # Arguments
///
/// * `prompt` - The prompt to send to the agent
/// * `removed_tools` - List of tools to remove from the agent
/// * `agent_type` - The agent type for MCP tool configuration ("implementer" or "explorer")
/// * `config` - Self-improvement configuration
/// * `writer` - Log writer for this agent
async fn spawn_and_run_agent(
    prompt: &str,
    removed_tools: &[String],
    agent_type: &str,
    config: &SelfImprovementConfig,
    writer: &mut AgentWriter,
) -> Result<SelfImprovementOutcome> {
    let cwd = std::env::current_dir()?.to_string_lossy().to_string();

    // Get the path to the current binary for MCP server
    let binary_path = std::env::current_exe().context("Failed to get current executable path")?;

    // Create a unique IPC address for the self-improver
    let socket_address = IpcAddress::generate("selfimprove");

    // Set up socket listener BEFORE spawning MCP server
    // The MCP server will connect to this socket to send tool calls
    let (completion_tx, completion_rx) = mpsc::channel::<CompletionSignal>(1);
    let socket_handle = setup_selfimprover_socket(&socket_address, completion_tx).await?;

    // Configure MCP server with the appropriate agent type
    let mcp_servers = vec![json!({
        "name": format!("paperboat-selfimprover-{agent_type}"),
        "command": binary_path.to_string_lossy(),
        "args": ["--mcp-server", "--socket", socket_address.as_str()],
        "env": [{
            "name": "PAPERBOAT_AGENT_TYPE",
            "value": agent_type
        }, {
            "name": "PAPERBOAT_REMOVED_TOOLS",
            "value": removed_tools.join(",")
        }]
    })];

    // Spawn and initialize the ACP client with retry
    let retry_config = RetryConfig::from_env();
    let (mut acp, session) = retry_async(&retry_config, "self-improver ACP spawn", || {
        let mcp_servers = mcp_servers.clone();
        let model = config.model.clone();
        let cwd = cwd.clone();

        async move {
            let mut acp = AcpClient::spawn_with_timeout(None, config.request_timeout)
                .await
                .context("Failed to spawn ACP client for self-improver")?;

            acp.initialize()
                .await
                .context("Failed to initialize ACP for self-improver")?;

            // Self-improver needs full tool access to make changes
            let session = acp
                .session_new(&model, mcp_servers, &cwd, SessionMode::Agent)
                .await
                .context("Failed to create self-improver session")?;

            Ok((acp, session))
        }
    })
    .await?;

    tracing::info!(
        "🤖 Self-improver agent started (session_id={}, mode={})",
        &session.session_id,
        agent_type
    );

    // Send the prompt
    acp.session_prompt(&session.session_id, prompt)
        .await
        .context("Failed to send prompt to self-improver")?;

    // Drain messages and wait for completion
    // completion_rx signals when the agent calls the complete tool
    let outcome =
        drain_self_improver_session(&mut acp, &session.session_id, writer, config, completion_rx)
            .await;

    // Clean up socket resources
    socket_handle.cleanup();
    let _ = acp.shutdown().await;

    outcome
}

/// Handle for the self-improver socket, for cleanup.
struct SelfImproverSocketHandle {
    socket_address: IpcAddress,
    listener_task: JoinHandle<()>,
}

impl SelfImproverSocketHandle {
    fn cleanup(self) {
        self.listener_task.abort();
        self.socket_address.cleanup();
    }
}

/// Set up a socket listener for the self-improver.
///
/// This handles MCP tool calls from the self-improver agent.
/// The main tool we care about is `complete`, which signals the agent is done.
async fn setup_selfimprover_socket(
    socket_address: &IpcAddress,
    completion_tx: mpsc::Sender<CompletionSignal>,
) -> Result<SelfImproverSocketHandle> {
    let listener = IpcListener::bind(socket_address)
        .await
        .with_context(|| format!("Failed to bind self-improver socket at {socket_address}"))?;

    tracing::debug!("Self-improver socket listening at: {}", socket_address);

    let socket_address_for_handle = socket_address.clone();

    let listener_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    let completion_tx = completion_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_selfimprover_tool_call(stream, completion_tx).await {
                            tracing::warn!("Self-improver tool call error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::debug!("Self-improver socket listener stopped: {}", e);
                    break;
                }
            }
        }
    });

    // Yield to let the listener task start
    tokio::task::yield_now().await;

    Ok(SelfImproverSocketHandle {
        socket_address: socket_address_for_handle,
        listener_task,
    })
}

/// Handle a single tool call from the self-improver agent.
///
/// This parses the MCP tool request (in JSON-RPC format), handles it,
/// and sends the response back. The main tool we care about is `complete`.
async fn handle_selfimprover_tool_call(
    stream: IpcStream,
    completion_tx: mpsc::Sender<CompletionSignal>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read the tool request (JSON-RPC format from MCP server)
    if reader.read_line(&mut line).await? == 0 {
        return Ok(()); // Connection closed
    }

    let request: Value = serde_json::from_str(&line).context("Failed to parse tool request")?;

    // Extract tool name and arguments from the request
    // The MCP server sends requests with request_id and tool_call containing tool info
    let request_id = request["request_id"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let tool_call = &request["tool_call"];

    // Tool call contains the tool type as a key with its arguments
    let (tool_name, arguments) = if let Some(obj) = tool_call.as_object() {
        // Find the first key (tool type) and its value (arguments)
        obj.iter()
            .next()
            .map_or(("", Value::Null), |(k, v)| (k.as_str(), v.clone()))
    } else {
        ("", Value::Null)
    };

    tracing::debug!("Self-improver tool call received: {}", tool_name);

    // Handle the tool call and build response
    let response =
        handle_selfimprover_request(&request_id, tool_name, &arguments, &completion_tx).await;

    // Send response back
    let response_json = serde_json::to_string(&response)?;
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    Ok(())
}

/// Handle a self-improver MCP tool request.
///
/// The self-improver has access to implementer tools (view, edit, etc.) which
/// are handled by Augment, but the `complete` tool is handled by us.
async fn handle_selfimprover_request(
    request_id: &str,
    tool_name: &str,
    arguments: &Value,
    completion_tx: &mpsc::Sender<CompletionSignal>,
) -> Value {
    // Check if this is the complete tool
    if tool_name == "complete" || tool_name == "Complete" {
        // Extract success and message from arguments
        let success = arguments
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let message = arguments
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string);

        // Send completion signal
        let _ = completion_tx
            .send(CompletionSignal { success, message })
            .await;

        return json!({
            "request_id": request_id,
            "success": true,
            "summary": "Self-improvement analysis complete.",
            "files_modified": []
        });
    }

    // For other tools, return an error - they should be handled by Augment
    json!({
        "request_id": request_id,
        "success": false,
        "summary": format!("Tool '{}' is not handled by the self-improver MCP server.", tool_name),
        "files_modified": [],
        "error": format!("Unknown tool: {tool_name}")
    })
}

/// Drain messages from the self-improver session until completion.
///
/// Completion can happen in two ways:
/// 1. The agent calls the `complete` MCP tool (signaled via `completion_rx`)
/// 2. The ACP session finishes (`session_finished`/`agent_turn_finished`)
async fn drain_self_improver_session(
    acp: &mut AcpClient,
    session_id: &str,
    writer: &mut AgentWriter,
    config: &SelfImprovementConfig,
    mut completion_rx: mpsc::Receiver<CompletionSignal>,
) -> Result<SelfImprovementOutcome> {
    let start = std::time::Instant::now();
    let mut final_message: Option<String> = None;
    let mut success = false;
    let mut changes_made: usize = 0;

    loop {
        // Check timeout
        if start.elapsed() > config.session_timeout {
            tracing::warn!("Self-improvement session timed out");
            return Ok(SelfImprovementOutcome {
                success: false,
                message: Some("Session timed out".to_string()),
                changes_made,
            });
        }

        // Use tokio::select to wait for either:
        // 1. ACP message from the agent
        // 2. Completion signal from the socket handler (when complete tool is called)
        tokio::select! {
            // Check for completion signal from socket handler
            Some(signal) = completion_rx.recv() => {
                tracing::info!("✅ Self-improver signaled completion via MCP tool");
                return Ok(SelfImprovementOutcome {
                    success: signal.success,
                    message: signal.message,
                    changes_made,
                });
            }

            // Check for ACP messages
            result = tokio::time::timeout(Duration::from_secs(5), acp.recv()) => {
                match result {
                    Ok(Ok(msg)) => {
                        if let Some(update_type) = extract_session_update(&msg, session_id) {
                            match update_type.as_str() {
                                "session_finished" | "agent_turn_finished" => {
                                    tracing::info!("✅ Self-improver session finished");
                                    success = true;
                                    break;
                                }
                                "agent_message_chunk" | "agent_thought_chunk" => {
                                    if let Some(text) = extract_message_text(&msg) {
                                        let _ = writer.write_message_chunk(&text).await;
                                        // Accumulate for final message
                                        if let Some(ref mut m) = final_message {
                                            m.push_str(&text);
                                        } else {
                                            final_message = Some(text);
                                        }
                                    }
                                }
                                "tool_call" => {
                                    if let Some(title) = extract_tool_title(&msg) {
                                        let _ = writer.write_tool_call(&title).await;
                                        tracing::debug!("🔧 Self-improver tool call: {}", title);
                                        // Count file-editing tool calls as changes
                                        if is_editing_tool(&title) {
                                            changes_made += 1;
                                        }
                                    }
                                }
                                // "tool_result" and other message types don't need processing
                                _ => {}
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Error receiving from self-improver: {}", e);
                        break;
                    }
                    Err(_) => {
                        // Timeout on recv, continue loop (will check completion_rx next iteration)
                    }
                }
            }
        }
    }

    Ok(SelfImprovementOutcome {
        success,
        message: final_message,
        changes_made,
    })
}

/// Extract the session update type from a message.
fn extract_session_update(msg: &Value, expected_session_id: &str) -> Option<String> {
    let params = msg.get("params")?;
    // Support both ACP format (sessionId) and CLI format (session_id)
    let session_id = params
        .get("sessionId")
        .or_else(|| params.get("session_id"))?
        .as_str()?;

    if session_id != expected_session_id {
        return None;
    }

    let update = params.get("update")?;
    // Support both ACP format (sessionUpdate) and CLI format (type)
    let session_update = update
        .get("sessionUpdate")
        .or_else(|| update.get("type"))?
        .as_str()?;
    Some(session_update.to_string())
}

/// Extract message text from an `agent_message_chunk` or `agent_thought_chunk`.
fn extract_message_text(msg: &Value) -> Option<String> {
    let content = msg.get("params")?.get("update")?.get("content")?;
    let text = content.get("text")?.as_str()?;
    Some(text.to_string())
}

/// Extract tool title from a `tool_call` message.
fn extract_tool_title(msg: &Value) -> Option<String> {
    let title = msg.get("params")?.get("update")?.get("title")?.as_str()?;
    // Strip MCP server prefix if present
    let title = title
        .strip_prefix("mcp__paperboat-selfimprover__")
        .unwrap_or(title);
    Some(title.to_string())
}

/// Check if a tool name represents a file-editing operation.
///
/// Returns true for tools that modify files in the codebase:
/// - `str-replace-editor`: Edit existing files
/// - `save-file`: Create new files
/// - `remove-files`: Delete files
fn is_editing_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "str-replace-editor" | "save-file" | "remove-files"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::Mutex;
    use tempfile::tempdir;

    // Mutex to serialize tests that modify environment variables
    static ENV_VAR_MUTEX: Mutex<()> = Mutex::new(());

    // ========================================================================
    // Configuration Tests
    // ========================================================================

    #[test]
    fn test_self_improvement_config_default_timeout() {
        let config = SelfImprovementConfig::default();
        assert_eq!(config.session_timeout, Duration::from_secs(300));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_self_improvement_config_default_model() {
        let config = SelfImprovementConfig::default();
        assert!(!config.model.is_empty());
        assert!(config.model.contains("claude"));
    }

    #[test]
    fn test_self_improvement_config_clone() {
        let config = SelfImprovementConfig {
            session_timeout: Duration::from_secs(600),
            request_timeout: Duration::from_secs(60),
            model: "test-model".to_string(),
        };
        let cloned = config.clone();
        assert_eq!(cloned.session_timeout, Duration::from_secs(600));
        assert_eq!(cloned.request_timeout, Duration::from_secs(60));
        assert_eq!(cloned.model, "test-model");
    }

    #[test]
    fn test_self_improvement_config_debug() {
        let config = SelfImprovementConfig::default();
        let debug_str = format!("{config:?}");
        assert!(debug_str.contains("SelfImprovementConfig"));
        assert!(debug_str.contains("session_timeout"));
        assert!(debug_str.contains("request_timeout"));
        assert!(debug_str.contains("model"));
    }

    // ========================================================================
    // Task Builder Tests
    // ========================================================================

    #[test]
    fn test_build_self_improvement_task_contains_run_dir() {
        let run_dir = std::path::PathBuf::from("/tmp/test-run");
        let task = build_self_improvement_task(&run_dir);

        assert!(task.contains("/tmp/test-run"));
        assert!(task.contains("implement improvements"));
        assert!(task.contains("cargo check"));
    }

    #[test]
    fn test_build_self_improvement_task_instructions() {
        let run_dir = tempdir().unwrap();
        let task = build_self_improvement_task(run_dir.path());

        // Verify all required instructions are present
        assert!(task.contains("Read the log files"));
        assert!(task.contains("Identify patterns"));
        assert!(task.contains("Make small, safe"));
        assert!(task.contains("cargo test"));
        assert!(task.contains("Error messages"));
        assert!(task.contains("Documentation gaps"));
        assert!(task.contains("small, safe changes"));
    }

    #[test]
    fn test_build_self_improvement_task_with_special_path() {
        let run_dir = std::path::PathBuf::from("/path/with spaces/and-dashes/run_123");
        let task = build_self_improvement_task(&run_dir);
        assert!(task.contains("/path/with spaces/and-dashes/run_123"));
    }

    // ========================================================================
    // SelfImprovementOutcome Tests
    // ========================================================================

    #[test]
    fn test_outcome_default_values() {
        let outcome = SelfImprovementOutcome {
            success: false,
            message: None,
            changes_made: 0,
        };

        assert!(!outcome.success);
        assert!(outcome.message.is_none());
        assert_eq!(outcome.changes_made, 0);
    }

    #[test]
    fn test_outcome_with_message() {
        let outcome = SelfImprovementOutcome {
            success: true,
            message: Some("Changes made".to_string()),
            changes_made: 3,
        };

        assert!(outcome.success);
        assert_eq!(outcome.message.as_deref(), Some("Changes made"));
        assert_eq!(outcome.changes_made, 3);
    }

    #[test]
    fn test_outcome_debug_format() {
        let outcome = SelfImprovementOutcome {
            success: true,
            message: Some("Test message".to_string()),
            changes_made: 5,
        };
        let debug_str = format!("{outcome:?}");
        assert!(debug_str.contains("SelfImprovementOutcome"));
        assert!(debug_str.contains("success: true"));
        assert!(debug_str.contains("Test message"));
        assert!(debug_str.contains("5"));
    }

    // ========================================================================
    // Helper Function Tests
    // ========================================================================

    #[test]
    fn test_ipc_address_generate_unique() {
        let addr1 = IpcAddress::generate("selfimprove");
        let addr2 = IpcAddress::generate("selfimprove");

        // Each generated address should be unique
        assert_ne!(addr1.to_string(), addr2.to_string());

        // Platform-specific format checks
        #[cfg(unix)]
        {
            // On Unix, should be a .sock file in temp directory
            let addr_str = addr1.to_string();
            assert!(addr_str.contains("vl-"));
            assert!(addr_str.ends_with(".sock"));
        }

        #[cfg(windows)]
        {
            // On Windows, should be a named pipe path
            let addr_str = addr1.to_string();
            assert!(addr_str.starts_with(r"\\.\pipe\vl-"));
        }
    }

    #[test]
    fn test_extract_session_update_valid() {
        let msg = serde_json::json!({
            "params": {
                "sessionId": "session-123",
                "update": {
                    "sessionUpdate": "agent_turn_finished"
                }
            }
        });

        let result = extract_session_update(&msg, "session-123");
        assert_eq!(result, Some("agent_turn_finished".to_string()));
    }

    #[test]
    fn test_extract_session_update_wrong_session() {
        let msg = serde_json::json!({
            "params": {
                "sessionId": "session-123",
                "update": {
                    "sessionUpdate": "agent_turn_finished"
                }
            }
        });

        let result = extract_session_update(&msg, "different-session");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_session_update_cli_format() {
        // Test CLI format with session_id and type
        let msg = serde_json::json!({
            "params": {
                "session_id": "cli-session-456",
                "update": {
                    "type": "session_finished"
                }
            }
        });

        let result = extract_session_update(&msg, "cli-session-456");
        assert_eq!(result, Some("session_finished".to_string()));
    }

    #[test]
    fn test_extract_session_update_missing_params() {
        let msg = serde_json::json!({
            "method": "session/update"
        });
        let result = extract_session_update(&msg, "any-session");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_session_update_missing_session_id() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "sessionUpdate": "agent_turn_finished"
                }
            }
        });
        let result = extract_session_update(&msg, "session-123");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_session_update_missing_update() {
        let msg = serde_json::json!({
            "params": {
                "sessionId": "session-123"
            }
        });
        let result = extract_session_update(&msg, "session-123");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_message_text_valid() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "content": {
                        "text": "Hello, world!"
                    }
                }
            }
        });

        let result = extract_message_text(&msg);
        assert_eq!(result, Some("Hello, world!".to_string()));
    }

    #[test]
    fn test_extract_message_text_missing_content() {
        let msg = serde_json::json!({
            "params": {
                "update": {}
            }
        });
        let result = extract_message_text(&msg);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_message_text_missing_text() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "content": {
                        "type": "image"
                    }
                }
            }
        });
        let result = extract_message_text(&msg);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_message_text_empty_text() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "content": {
                        "text": ""
                    }
                }
            }
        });
        let result = extract_message_text(&msg);
        assert_eq!(result, Some(String::new()));
    }

    #[test]
    fn test_extract_tool_title_strips_prefix() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "title": "mcp__paperboat-selfimprover__view"
                }
            }
        });

        let result = extract_tool_title(&msg);
        assert_eq!(result, Some("view".to_string()));
    }

    #[test]
    fn test_extract_tool_title_no_prefix() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "title": "simple_tool"
                }
            }
        });

        let result = extract_tool_title(&msg);
        assert_eq!(result, Some("simple_tool".to_string()));
    }

    #[test]
    fn test_extract_tool_title_complex_prefix() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "title": "mcp__paperboat-selfimprover__str-replace-editor"
                }
            }
        });
        let result = extract_tool_title(&msg);
        assert_eq!(result, Some("str-replace-editor".to_string()));
    }

    #[test]
    fn test_extract_tool_title_missing_update() {
        let msg = serde_json::json!({
            "params": {}
        });
        let result = extract_tool_title(&msg);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_tool_title_missing_title() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "content": "some content"
                }
            }
        });
        let result = extract_tool_title(&msg);
        assert!(result.is_none());
    }

    #[test]
    fn test_is_editing_tool_true_cases() {
        assert!(is_editing_tool("str-replace-editor"));
        assert!(is_editing_tool("save-file"));
        assert!(is_editing_tool("remove-files"));
    }

    #[test]
    fn test_is_editing_tool_false_cases() {
        assert!(!is_editing_tool("view"));
        assert!(!is_editing_tool("codebase-retrieval"));
        assert!(!is_editing_tool("launch-process"));
        assert!(!is_editing_tool("complete"));
        assert!(!is_editing_tool("web-search"));
    }

    #[test]
    fn test_is_editing_tool_case_sensitive() {
        // Tool names are case-sensitive
        assert!(!is_editing_tool("Str-Replace-Editor"));
        assert!(!is_editing_tool("SAVE-FILE"));
        assert!(!is_editing_tool("Remove-Files"));
    }

    #[test]
    fn test_is_editing_tool_empty_string() {
        assert!(!is_editing_tool(""));
    }

    // ========================================================================
    // handle_selfimprover_request Tests
    // ========================================================================

    #[tokio::test]
    async fn test_handle_selfimprover_request_complete_tool() {
        let (tx, mut rx) = mpsc::channel::<CompletionSignal>(1);

        let arguments = json!({
            "success": true,
            "message": "Self-improvement complete"
        });

        let response = handle_selfimprover_request("req-001", "complete", &arguments, &tx).await;

        // Verify response format
        assert_eq!(response["request_id"], "req-001");
        assert!(response["success"].as_bool().unwrap());
        assert_eq!(
            response["summary"],
            "Self-improvement analysis complete."
        );
        assert!(response["files_modified"].as_array().unwrap().is_empty());

        // Verify completion signal was sent
        let signal = rx.recv().await.unwrap();
        assert!(signal.success);
        assert_eq!(signal.message, Some("Self-improvement complete".to_string()));
    }

    #[tokio::test]
    async fn test_handle_selfimprover_request_complete_uppercase() {
        let (tx, mut rx) = mpsc::channel::<CompletionSignal>(1);

        let arguments = json!({
            "success": false,
            "message": "Failed to complete"
        });

        let response = handle_selfimprover_request("req-002", "Complete", &arguments, &tx).await;

        assert!(response["success"].as_bool().unwrap());

        let signal = rx.recv().await.unwrap();
        assert!(!signal.success);
        assert_eq!(signal.message, Some("Failed to complete".to_string()));
    }

    #[tokio::test]
    async fn test_handle_selfimprover_request_complete_default_success() {
        let (tx, mut rx) = mpsc::channel::<CompletionSignal>(1);

        // No success field - should default to true
        let arguments = json!({
            "message": "Done"
        });

        let _ = handle_selfimprover_request("req-003", "complete", &arguments, &tx).await;

        let signal = rx.recv().await.unwrap();
        assert!(signal.success); // Default to true
    }

    #[tokio::test]
    async fn test_handle_selfimprover_request_complete_no_message() {
        let (tx, mut rx) = mpsc::channel::<CompletionSignal>(1);

        let arguments = json!({
            "success": true
        });

        let _ = handle_selfimprover_request("req-004", "complete", &arguments, &tx).await;

        let signal = rx.recv().await.unwrap();
        assert!(signal.success);
        assert!(signal.message.is_none());
    }

    #[tokio::test]
    async fn test_handle_selfimprover_request_unknown_tool() {
        let (tx, _rx) = mpsc::channel::<CompletionSignal>(1);

        let arguments = json!({});

        let response = handle_selfimprover_request("req-005", "unknown_tool", &arguments, &tx).await;

        // Verify error response
        assert_eq!(response["request_id"], "req-005");
        assert!(!response["success"].as_bool().unwrap());
        assert!(response["error"].as_str().unwrap().contains("unknown_tool"));
        assert!(response["summary"]
            .as_str()
            .unwrap()
            .contains("not handled"));
    }

    #[tokio::test]
    async fn test_handle_selfimprover_request_view_tool_not_handled() {
        let (tx, _rx) = mpsc::channel::<CompletionSignal>(1);

        let arguments = json!({
            "path": "src/main.rs"
        });

        let response = handle_selfimprover_request("req-006", "view", &arguments, &tx).await;

        assert!(!response["success"].as_bool().unwrap());
        assert!(response["error"].as_str().unwrap().contains("view"));
    }

    // ========================================================================
    // Socket Handler Tests (using real IPC sockets)
    // ========================================================================

    #[tokio::test]
    async fn test_setup_selfimprover_socket_binds_successfully() {
        let socket_address = IpcAddress::generate("test-selfimprove-setup");
        let (completion_tx, _completion_rx) = mpsc::channel::<CompletionSignal>(1);

        let handle = setup_selfimprover_socket(&socket_address, completion_tx)
            .await
            .expect("Should bind socket successfully");

        // Socket should exist on Unix
        #[cfg(unix)]
        assert!(socket_address.exists(), "Socket file should exist");

        // Clean up
        handle.cleanup();
    }

    #[tokio::test]
    async fn test_selfimprover_socket_handle_cleanup() {
        let socket_address = IpcAddress::generate("test-selfimprove-cleanup");
        let (completion_tx, _completion_rx) = mpsc::channel::<CompletionSignal>(1);

        let handle = setup_selfimprover_socket(&socket_address, completion_tx)
            .await
            .expect("Should bind socket");

        let addr_clone = handle.socket_address.clone();
        handle.cleanup();

        // After cleanup, socket should not exist (on Unix)
        #[cfg(unix)]
        {
            // Give a moment for cleanup to complete
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert!(!addr_clone.exists(), "Socket file should be cleaned up");
        }
    }

    #[tokio::test]
    async fn test_selfimprover_socket_receives_complete_tool() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let socket_address = IpcAddress::generate("test-selfimprove-complete");
        let (completion_tx, mut completion_rx) = mpsc::channel::<CompletionSignal>(1);

        let handle = setup_selfimprover_socket(&socket_address, completion_tx)
            .await
            .expect("Should bind socket");

        // Give the listener task time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect as a client and send a complete tool call
        let stream = IpcStream::connect(&socket_address)
            .await
            .expect("Should connect to socket");

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send complete tool call in the expected format
        let request = json!({
            "request_id": "test-req-001",
            "tool_call": {
                "complete": {
                    "success": true,
                    "message": "Socket test complete"
                }
            }
        });

        writer
            .write_all(serde_json::to_string(&request).unwrap().as_bytes())
            .await
            .unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        // Read response
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await.unwrap();
        let response: Value = serde_json::from_str(&response_line).unwrap();

        assert!(response["success"].as_bool().unwrap());

        // Verify completion signal was received
        let signal = tokio::time::timeout(Duration::from_secs(1), completion_rx.recv())
            .await
            .expect("Should receive signal within timeout")
            .expect("Should have signal");

        assert!(signal.success);
        assert_eq!(signal.message, Some("Socket test complete".to_string()));

        handle.cleanup();
    }

    #[tokio::test]
    async fn test_selfimprover_socket_handles_invalid_json() {
        let socket_address = IpcAddress::generate("test-selfimprove-invalid");
        let (completion_tx, _completion_rx) = mpsc::channel::<CompletionSignal>(1);

        let handle = setup_selfimprover_socket(&socket_address, completion_tx)
            .await
            .expect("Should bind socket");

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect and send invalid JSON
        let stream = IpcStream::connect(&socket_address)
            .await
            .expect("Should connect");

        let (_reader, mut writer) = stream.into_split();

        // Send invalid JSON
        writer.write_all(b"not valid json\n").await.unwrap();
        writer.flush().await.unwrap();

        // The connection should be closed without panicking
        tokio::time::sleep(Duration::from_millis(100)).await;

        handle.cleanup();
    }

    #[tokio::test]
    async fn test_selfimprover_socket_handles_connection_close() {
        let socket_address = IpcAddress::generate("test-selfimprove-close");
        let (completion_tx, _completion_rx) = mpsc::channel::<CompletionSignal>(1);

        let handle = setup_selfimprover_socket(&socket_address, completion_tx)
            .await
            .expect("Should bind socket");

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect and immediately close
        let stream = IpcStream::connect(&socket_address)
            .await
            .expect("Should connect");
        drop(stream);

        // Should not crash
        tokio::time::sleep(Duration::from_millis(100)).await;

        handle.cleanup();
    }

    // ========================================================================
    // Skip Path Tests (checking conditions for maybe_run_self_improvement)
    // ========================================================================

    #[tokio::test]
    async fn test_skip_when_disabled_via_env() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Disable via env var
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "0");

        let run_dir = tempdir().unwrap();
        let result = TaskResult {
            success: true,
            message: Some("Test".to_string()),
        };
        let (event_tx, _) = broadcast::channel(10);
        let task_manager = TaskManager::new(event_tx);

        let outcome = maybe_run_self_improvement(run_dir.path(), &result, &task_manager).await;

        // Should return Ok(None) - skipped because disabled
        assert!(outcome.is_ok());
        assert!(outcome.unwrap().is_none());

        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");
    }

    #[tokio::test]
    async fn test_skip_when_primary_run_failed() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Enable self-improvement
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "1");

        let run_dir = tempdir().unwrap();
        let result = TaskResult {
            success: false, // Failed run
            message: Some("Primary run failed".to_string()),
        };
        let (event_tx, _) = broadcast::channel(10);
        let task_manager = TaskManager::new(event_tx);

        let outcome = maybe_run_self_improvement(run_dir.path(), &result, &task_manager).await;

        // Should return Ok(None) - skipped because run failed
        assert!(outcome.is_ok());
        assert!(outcome.unwrap().is_none());

        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");
    }

    #[tokio::test]
    #[serial]
    async fn test_skip_when_not_paperboat_repo() {
        let _guard = ENV_VAR_MUTEX.lock().unwrap();

        // Enable self-improvement
        std::env::set_var("PAPERBOAT_SELF_IMPROVE", "1");

        // Create a temp directory that is NOT the paperboat repo
        let temp_repo = tempdir().unwrap();
        std::fs::write(
            temp_repo.path().join("Cargo.toml"),
            r#"[package]
name = "other-project"
version = "0.1.0"
"#,
        )
        .unwrap();

        // Save current dir and change to temp repo
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_repo.path()).unwrap();

        let run_dir = tempdir().unwrap();
        let result = TaskResult {
            success: true,
            message: Some("Test".to_string()),
        };
        let (event_tx, _) = broadcast::channel(10);
        let task_manager = TaskManager::new(event_tx);

        let outcome = maybe_run_self_improvement(run_dir.path(), &result, &task_manager).await;

        // Restore original directory
        std::env::set_current_dir(original_dir).unwrap();
        std::env::remove_var("PAPERBOAT_SELF_IMPROVE");

        // Should return Ok(None) - skipped because not in paperboat repo
        assert!(outcome.is_ok());
        assert!(outcome.unwrap().is_none());
    }

    // ========================================================================
    // Failure Isolation Tests
    // ========================================================================

    #[test]
    fn test_self_improvement_outcome_type_supports_failure_isolation() {
        // This test verifies the Result<Option<Outcome>> pattern
        // that allows self-improvement failures to be isolated

        // Successful self-improvement
        let success: Result<Option<SelfImprovementOutcome>> = Ok(Some(SelfImprovementOutcome {
            success: true,
            message: Some("Improved".to_string()),
            changes_made: 1,
        }));
        assert!(success.is_ok());

        // Skipped self-improvement (not in paperboat repo, etc.)
        let skipped: Result<Option<SelfImprovementOutcome>> = Ok(None);
        assert!(skipped.is_ok());
        assert!(skipped.unwrap().is_none());

        // Failed self-improvement (error that can be logged but doesn't crash main run)
        let failed: Result<Option<SelfImprovementOutcome>> =
            Err(anyhow::anyhow!("Self-improvement error"));
        assert!(failed.is_err());

        // The caller can handle the error without affecting the main result
        let main_result_success = true;
        let self_improve_result: Result<Option<SelfImprovementOutcome>> =
            Err(anyhow::anyhow!("Error"));

        // Main result should remain unaffected
        if let Err(e) = self_improve_result {
            // Log the error but don't flip the main result
            let _ = format!("Self-improvement failed (non-fatal): {e}");
        }
        assert!(main_result_success);
    }

    #[test]
    fn test_outcome_failure_does_not_affect_success_field() {
        // Even a failed self-improvement can have success: false in outcome
        // without affecting the main run result
        let outcome = SelfImprovementOutcome {
            success: false, // Self-improvement failed
            message: Some("Something went wrong".to_string()),
            changes_made: 0,
        };

        // The outcome itself indicates failure...
        assert!(!outcome.success);

        // ...but this doesn't mean the main run failed
        // (that's a separate concern handled by the caller)
    }

    // ========================================================================
    // Completion Signal Tests
    // ========================================================================

    #[test]
    fn test_completion_signal_structure() {
        let signal = CompletionSignal {
            success: true,
            message: Some("Completed".to_string()),
        };
        assert!(signal.success);
        assert_eq!(signal.message, Some("Completed".to_string()));

        let signal_no_message = CompletionSignal {
            success: false,
            message: None,
        };
        assert!(!signal_no_message.success);
        assert!(signal_no_message.message.is_none());
    }

    #[tokio::test]
    async fn test_completion_signal_channel_behavior() {
        let (tx, mut rx) = mpsc::channel::<CompletionSignal>(1);

        // Send a signal
        tx.send(CompletionSignal {
            success: true,
            message: Some("Test signal".to_string()),
        })
        .await
        .unwrap();

        // Receive the signal
        let signal = rx.recv().await.unwrap();
        assert!(signal.success);
        assert_eq!(signal.message, Some("Test signal".to_string()));
    }

    #[tokio::test]
    async fn test_completion_signal_channel_closed() {
        let (tx, mut rx) = mpsc::channel::<CompletionSignal>(1);

        // Drop sender
        drop(tx);

        // Receive should return None
        let result = rx.recv().await;
        assert!(result.is_none());
    }

    // ========================================================================
    // Socket Handle Tests
    // ========================================================================

    #[test]
    fn test_socket_handle_has_cleanup() {
        // This is a compile-time test - verifying the cleanup method exists
        // and the struct has the expected fields
        fn _check_handle_fields(handle: SelfImproverSocketHandle) {
            let _: IpcAddress = handle.socket_address;
            let _: JoinHandle<()> = handle.listener_task;
        }
    }

    // ========================================================================
    // Prompt Content Tests
    // ========================================================================

    #[test]
    fn test_prompt_includes_required_elements() {
        let run_dir = std::path::PathBuf::from("/test/run/dir");
        let task = build_self_improvement_task(&run_dir);

        // The task description should include key elements
        assert!(task.contains("Run Directory"));
        assert!(task.contains("/test/run/dir"));

        // Mission items
        assert!(task.contains("1."));
        assert!(task.contains("2."));
        assert!(task.contains("3."));
        assert!(task.contains("4."));

        // Focus areas
        assert!(task.contains("Error messages"));
        assert!(task.contains("Prompts that confused"));
        assert!(task.contains("Edge cases"));
        assert!(task.contains("Documentation"));

        // Safety reminder
        assert!(task.contains("small, safe"));
        assert!(task.contains("No core refactors"));
    }

    // ========================================================================
    // Additional Edge Case Tests
    // ========================================================================

    #[test]
    fn test_extract_session_update_all_update_types() {
        // Test various session update types
        let update_types = [
            "session_finished",
            "agent_turn_finished",
            "agent_message_chunk",
            "agent_thought_chunk",
            "tool_call",
            "tool_result",
        ];

        for update_type in update_types {
            let msg = serde_json::json!({
                "params": {
                    "sessionId": "test-session",
                    "update": {
                        "sessionUpdate": update_type
                    }
                }
            });

            let result = extract_session_update(&msg, "test-session");
            assert_eq!(result, Some(update_type.to_string()));
        }
    }

    #[test]
    fn test_extract_message_text_multiline() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "content": {
                        "text": "Line 1\nLine 2\nLine 3"
                    }
                }
            }
        });

        let result = extract_message_text(&msg);
        assert_eq!(result, Some("Line 1\nLine 2\nLine 3".to_string()));
    }

    #[test]
    fn test_extract_message_text_with_special_chars() {
        let msg = serde_json::json!({
            "params": {
                "update": {
                    "content": {
                        "text": "Special: 🔧 ✅ ❌ \"quotes\" and 'apostrophes'"
                    }
                }
            }
        });

        let result = extract_message_text(&msg);
        assert_eq!(
            result,
            Some("Special: 🔧 ✅ ❌ \"quotes\" and 'apostrophes'".to_string())
        );
    }

    // ========================================================================
    // Integration Test with Tempdir for Run Directory
    // ========================================================================

    #[tokio::test]
    async fn test_run_directory_structure_creation() {
        let run_dir = tempdir().unwrap();
        let self_improve_dir = run_dir.path().join("self-improve");

        // Create directory structure as run_self_improver would
        std::fs::create_dir_all(&self_improve_dir).unwrap();

        assert!(self_improve_dir.exists());
        assert!(self_improve_dir.is_dir());
    }

    #[tokio::test]
    async fn test_log_scope_creates_self_improver_writer() {
        use crate::logging::LogScope;

        let run_dir = tempdir().unwrap();
        let self_improve_dir = run_dir.path().join("self-improve");
        std::fs::create_dir_all(&self_improve_dir).unwrap();

        let (event_tx, _) = broadcast::channel(100);
        let scope = LogScope::new(self_improve_dir.clone(), event_tx, 0);

        let writer = scope.self_improver_writer().await.unwrap();

        // Verify log file was created
        let log_path = self_improve_dir.join("self-improver.log");
        assert!(log_path.exists());
        assert_eq!(writer.agent_name(), "self-improver");
    }
}
