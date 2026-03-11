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
    let session_id = params.get("sessionId")?.as_str()?;

    if session_id != expected_session_id {
        return None;
    }

    let update = params.get("update")?;
    let session_update = update.get("sessionUpdate")?.as_str()?;
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
}
