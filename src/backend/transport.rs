// Allow some clippy lints for this new module - can be cleaned up later
#![allow(clippy::doc_markdown)]
#![allow(clippy::len_zero)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::use_self)]

//! Transport abstraction for agent communication.
//!
//! This module provides the core transport layer that separates communication
//! protocol from backend vendor. It enables support for multiple connection modes
//! (ACP, CLI, future protocols) while maintaining a unified interface.
//!
//! # Architecture
//!
//! The transport layer sits between the application and backend-specific implementations:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Application                            │
//! │  (src/app/orchestrator.rs, session.rs, agent_handler.rs)   │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    AgentTransport Trait                     │
//! │  (protocol-agnostic interface for agent communication)      │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!           ┌──────────────────┼──────────────────┐
//!           ▼                  ▼                  ▼
//!   ┌───────────────┐  ┌───────────────┐  ┌───────────────┐
//!   │ AuggieAcp     │  │ CursorAcp     │  │ CursorCli     │
//!   │ Transport     │  │ Transport     │  │ Transport     │
//!   └───────────────┘  └───────────────┘  └───────────────┘
//! ```
//!
//! # Separation of Concerns
//!
//! - **Backend** (`trait.rs`): Vendor-specific configuration (auth, models, MCP setup)
//! - **Transport** (this module): Communication protocol (how to talk to the agent)
//! - **AgentType**: Permission control (what tools each agent type can use)
//!
//! # Transport Kinds
//!
//! - **ACP** (Agent Communication Protocol): JSON-RPC over stdin/stdout, request/response model
//! - **CLI**: Non-interactive CLI mode with streaming JSON output (`agent --print`)
//!
//! # Usage Example
//!
//! ```ignore
//! // Create a transport via the backend
//! let backend = BackendKind::Cursor.create();
//! let transport = backend.create_transport(
//!     TransportKind::Cli,
//!     AgentType::Implementer,
//!     config,
//! ).await?;
//!
//! // Initialize and create session
//! transport.initialize().await?;
//! let session = transport.create_session(SessionConfig {
//!     model: "sonnet-4.6".into(),
//!     workspace: "/path/to/workspace".into(),
//!     mcp_servers: vec![],
//!     mode: SessionMode::Agent,
//! }).await?;
//!
//! // Send prompts and receive updates
//! transport.send_prompt(&session.session_id, "Hello").await?;
//! let updates = transport.take_notifications();
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::acp::SessionMode;

// ============================================================================
// Transport Kind
// ============================================================================

/// Available transport protocols for agent communication.
///
/// Each transport kind represents a different communication mechanism
/// with its own trade-offs:
///
/// - **ACP**: Full duplex, supports tool permission requests, but Cursor's
///   implementation has broken MCP tool support.
/// - **CLI**: Streaming JSON output, properly loads MCP servers from config,
///   but runs as separate processes per prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransportKind {
    /// ACP protocol (Agent Communication Protocol).
    ///
    /// JSON-RPC 2.0 over stdin/stdout with bidirectional communication.
    /// Supports request/response patterns and server-initiated notifications.
    ///
    /// Used by: `auggie --acp`, `agent acp`
    #[default]
    Acp,

    /// Non-interactive CLI mode.
    ///
    /// Spawns the agent CLI with `--print` flag for each prompt, receiving
    /// streaming JSON output. Better MCP support but less interactive.
    ///
    /// Used by: `agent --print --output-format stream-json`
    Cli,
}

impl TransportKind {
    /// Returns the string identifier for this transport kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Cli => "cli",
        }
    }
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Agent Type (for permission control)
// ============================================================================

/// Type of agent for permission and capability control.
///
/// Different agent types have different tool access permissions:
///
/// | Agent Type   | File Editing | MCP Tools                        |
/// |-------------|--------------|----------------------------------|
/// | Orchestrator | ❌ Denied    | spawn_agents, decompose, skip_tasks, list_tasks, complete |
/// | Planner      | ❌ Denied    | set_goal, create_task, complete  |
/// | Implementer  | ✅ Allowed   | complete only                    |
///
/// This enum is used by transports to configure permission policies
/// that control which tools are allowed or denied for each agent type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentType {
    /// Orchestrator agent that coordinates task execution.
    ///
    /// Has access to spawn_agents, decompose, skip_tasks, list_tasks.
    /// Cannot edit files directly.
    Orchestrator,

    /// Planner agent that creates task plans.
    ///
    /// Has access to set_goal, create_task.
    /// Cannot edit files directly.
    Planner,

    /// Implementer agent that executes tasks.
    ///
    /// Full file system access.
    /// Only has the complete MCP tool.
    Implementer,
}

impl AgentType {
    /// Returns the string identifier for this agent type.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Planner => "planner",
            Self::Implementer => "implementer",
        }
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Session Configuration and Info
// ============================================================================

/// Configuration for creating a new agent session.
///
/// Contains all parameters needed to initialize a session with the agent,
/// including model selection, workspace path, MCP servers, and session mode.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Model identifier (e.g., "sonnet-4.6", "opus-4.6").
    ///
    /// This should be the backend-specific model ID returned by
    /// `Backend::resolve_tier()`.
    pub model: String,

    /// Working directory for the session.
    ///
    /// The agent will operate relative to this directory.
    pub workspace: String,

    /// MCP server configurations to enable for this session.
    ///
    /// Each entry is a JSON object with server configuration
    /// (typically name, command, args, env).
    pub mcp_servers: Vec<Value>,

    /// Session mode controlling agent capabilities.
    ///
    /// See [`SessionMode`] for details on each mode's restrictions.
    pub mode: SessionMode,
}

impl SessionConfig {
    /// Create a new session configuration with required parameters.
    pub fn new(model: impl Into<String>, workspace: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            workspace: workspace.into(),
            mcp_servers: Vec::new(),
            mode: SessionMode::default(),
        }
    }

    /// Add MCP servers to the configuration.
    #[must_use]
    pub fn with_mcp_servers(mut self, servers: Vec<Value>) -> Self {
        self.mcp_servers = servers;
        self
    }

    /// Set the session mode.
    #[must_use]
    pub fn with_mode(mut self, mode: SessionMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Information returned when a session is created.
///
/// Contains the session ID and any other metadata from session creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique identifier for the session.
    ///
    /// Used to send prompts and identify notifications for this session.
    pub session_id: String,
}

impl SessionInfo {
    /// Create a new session info with the given ID.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
        }
    }
}

// ============================================================================
// Session Updates (streaming responses)
// ============================================================================

/// Streaming updates from an agent session.
///
/// These represent the different types of messages that can be received
/// from an agent during execution. Both ACP notifications and CLI streaming
/// JSON are normalized to these variants.
///
/// # ACP Mapping
///
/// ACP `session/update` notifications with different `type` fields map to:
/// - `"text"` → `SessionUpdate::Text`
/// - `"tool_use"` → `SessionUpdate::ToolUse`
/// - `"tool_result"` → `SessionUpdate::ToolResult`
/// - `"completion"` / `"end"` → `SessionUpdate::Completion`
///
/// # CLI Mapping
///
/// CLI `--output-format stream-json` lines map similarly:
/// - `{"type":"text",...}` → `SessionUpdate::Text`
/// - `{"type":"tool_use",...}` → `SessionUpdate::ToolUse`
/// - `{"type":"tool_result",...}` → `SessionUpdate::ToolResult`
/// - `{"type":"result",...}` → `SessionUpdate::Completion`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionUpdate {
    /// Text content from the agent.
    ///
    /// This is the agent's natural language output, delivered incrementally
    /// as the agent generates it.
    Text {
        /// The session this update belongs to.
        session_id: String,
        /// The text content.
        content: String,
    },

    /// Agent is calling a tool.
    ///
    /// Indicates the agent wants to use a tool. For MCP tools, the
    /// application should execute the tool and respond with `respond_to_tool()`.
    ToolUse {
        /// The session this update belongs to.
        session_id: String,
        /// Unique identifier for this tool use (for responding).
        tool_use_id: String,
        /// Name of the tool being called.
        tool_name: String,
        /// Tool input parameters as JSON.
        input: Value,
    },

    /// Result from a tool call.
    ///
    /// Contains the output from a tool that was executed. This is typically
    /// sent by the agent/backend after it processes a tool call internally,
    /// or after we respond to an MCP tool call.
    ToolResult {
        /// The session this update belongs to.
        session_id: String,
        /// ID of the tool use this result corresponds to.
        tool_use_id: String,
        /// The tool output content.
        content: String,
        /// Whether the tool execution was successful.
        #[serde(default = "default_true")]
        is_success: bool,
    },

    /// Agent has finished processing.
    ///
    /// Indicates the agent completed its turn. May include a final result
    /// or summary message.
    Completion {
        /// The session this update belongs to.
        session_id: String,
        /// Final result or summary (optional).
        result: Option<String>,
        /// Whether the completion was successful.
        #[serde(default = "default_true")]
        success: bool,
    },

    /// Raw/unknown update type (for forward compatibility).
    ///
    /// Used for update types not yet supported by this enum.
    /// Contains the raw JSON for inspection.
    Raw {
        /// The session this update belongs to (if available).
        session_id: Option<String>,
        /// The raw JSON data.
        data: Value,
    },
}

/// Default value function for serde.
const fn default_true() -> bool {
    true
}

impl SessionUpdate {
    /// Get the session ID if available.
    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Text { session_id, .. }
            | Self::ToolUse { session_id, .. }
            | Self::ToolResult { session_id, .. }
            | Self::Completion { session_id, .. } => Some(session_id),
            Self::Raw { session_id, .. } => session_id.as_deref(),
        }
    }

    /// Check if this is a completion update.
    #[allow(dead_code)]
    pub const fn is_completion(&self) -> bool {
        matches!(self, Self::Completion { .. })
    }

    /// Check if this is a tool use update.
    #[allow(dead_code)]
    pub const fn is_tool_use(&self) -> bool {
        matches!(self, Self::ToolUse { .. })
    }
}

// ============================================================================
// Tool Result
// ============================================================================

/// Result of executing a tool call.
///
/// Used to respond to tool use requests from the agent. This is sent back
/// via `AgentTransport::respond_to_tool()` after processing an MCP tool call.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// The output content from the tool.
    pub content: String,

    /// Whether the tool execution was successful.
    ///
    /// If `false`, the content should describe the error.
    pub is_success: bool,
}

#[allow(dead_code)]
impl ToolResult {
    /// Create a successful tool result.
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_success: true,
        }
    }

    /// Create a failed tool result.
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            content: error.into(),
            is_success: false,
        }
    }
}

// ============================================================================
// Agent Transport Trait
// ============================================================================

/// Trait defining the transport interface for agent communication.
///
/// This trait abstracts the communication protocol (ACP, CLI, etc.) from
/// the application logic, enabling different backends to use different
/// transport mechanisms while presenting a unified interface.
///
/// # Lifecycle
///
/// A typical transport lifecycle:
///
/// 1. **Create**: Backend creates transport via `create_transport()`
/// 2. **Initialize**: Call `initialize()` to establish connection
/// 3. **Create Session**: Call `create_session()` to start a session
/// 4. **Take Notifications**: Call `take_notifications()` to get the update receiver
/// 5. **Send Prompts**: Call `send_prompt()` to send messages
/// 6. **Handle Tool Calls**: Call `respond_to_tool()` for MCP tool results
/// 7. **Shutdown**: Call `shutdown()` when done
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to support async contexts
/// and potential sharing across tasks.
///
/// # Error Handling
///
/// Methods return `anyhow::Result` for flexibility in error types.
/// Implementations should provide descriptive error messages.
#[async_trait]
pub trait AgentTransport: Send + Sync {
    /// Initialize the transport connection.
    ///
    /// This establishes the connection to the agent process.
    /// For ACP, this sends the initialize JSON-RPC request.
    /// For CLI, this may be a no-op or verify the CLI is available.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be established,
    /// authentication fails, or the agent process cannot be started.
    async fn initialize(&mut self) -> Result<()>;

    /// Create a new agent session.
    ///
    /// Initializes a session with the given configuration. The returned
    /// `SessionInfo` contains the session ID needed for subsequent calls.
    ///
    /// # Arguments
    ///
    /// * `config` - Session configuration including model, workspace, etc.
    ///
    /// # Returns
    ///
    /// Session information including the session ID.
    ///
    /// # Errors
    ///
    /// Returns an error if session creation fails (invalid model,
    /// workspace access denied, etc.).
    async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo>;

    /// Send a prompt to the agent session.
    ///
    /// Sends a message to the specified session. Responses come via
    /// the notification channel obtained from `take_notifications()`.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session to send to (from `SessionInfo`)
    /// * `prompt` - The prompt text to send
    ///
    /// # Errors
    ///
    /// Returns an error if the session doesn't exist or if sending fails.
    async fn send_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()>;

    /// Take the notification receiver for streaming updates.
    ///
    /// Returns the receiver end of the channel that receives `SessionUpdate`
    /// messages. This can only be called once; subsequent calls return `None`.
    ///
    /// The receiver should be consumed by a background task that processes
    /// updates (text, tool calls, completions) as they arrive.
    ///
    /// # Returns
    ///
    /// `Some(receiver)` on first call, `None` on subsequent calls.
    #[allow(dead_code)]
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>>;

    /// Respond to a tool call with the result.
    ///
    /// After receiving a `SessionUpdate::ToolUse`, the application should
    /// execute the tool and send the result back via this method.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session the tool call belongs to
    /// * `tool_use_id` - The tool use ID from `SessionUpdate::ToolUse`
    /// * `result` - The tool execution result
    ///
    /// # Errors
    ///
    /// Returns an error if the session doesn't exist, the tool use ID is
    /// invalid, or if sending the response fails.
    ///
    /// # Note
    ///
    /// Not all transports support this. CLI transport may ignore this call
    /// since tool results are handled differently in non-interactive mode.
    #[allow(dead_code)]
    async fn respond_to_tool(
        &mut self,
        session_id: &str,
        tool_use_id: &str,
        result: ToolResult,
    ) -> Result<()>;

    /// Gracefully shutdown the transport.
    ///
    /// Cleans up resources, terminates child processes, and closes channels.
    /// This should be called when the session is complete.
    ///
    /// # Errors
    ///
    /// Returns an error if shutdown fails (e.g., process won't terminate).
    async fn shutdown(&mut self) -> Result<()>;

    // ========================================================================
    // Legacy compatibility methods (for gradual migration from AcpClientTrait)
    // ========================================================================

    /// Receive the next raw notification from the transport.
    ///
    /// This is a legacy compatibility method that enables gradual migration
    /// from `AcpClientTrait::recv()`. New code should use `take_notifications()`
    /// instead.
    ///
    /// The default implementation returns an error indicating this transport
    /// doesn't support polling. Transports that wrap ACP clients can override
    /// this to provide backward compatibility.
    ///
    /// # Errors
    ///
    /// Returns an error if the transport doesn't support polling or if no
    /// more notifications are available.
    async fn recv(&mut self) -> Result<Value> {
        Err(anyhow::anyhow!(
            "This transport does not support polling via recv(). Use take_notifications() instead."
        ))
    }

    /// Take the raw notification receiver for legacy routing.
    ///
    /// This is a legacy compatibility method that enables gradual migration
    /// from `AcpClientTrait::take_notification_rx()`. New code should use
    /// `take_notifications()` instead.
    ///
    /// The default implementation returns `None`. Transports that wrap ACP
    /// clients can override this to provide backward compatibility.
    ///
    /// # Returns
    ///
    /// `Some(receiver)` if the transport supports raw notifications,
    /// `None` otherwise.
    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        None
    }

    /// Returns the transport kind for this transport.
    ///
    /// This is used to detect which transport type is being used, allowing
    /// the application to make transport-specific decisions (e.g., using
    /// unique sockets for CLI transports to prevent MCP server caching).
    ///
    /// # Returns
    ///
    /// The `TransportKind` for this transport. Defaults to `Acp` for
    /// backward compatibility.
    fn kind(&self) -> TransportKind {
        TransportKind::Acp
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // TransportKind Tests
    // ========================================================================

    #[test]
    fn test_transport_kind_as_str() {
        assert_eq!(TransportKind::Acp.as_str(), "acp");
        assert_eq!(TransportKind::Cli.as_str(), "cli");
    }

    #[test]
    fn test_transport_kind_display() {
        assert_eq!(format!("{}", TransportKind::Acp), "acp");
        assert_eq!(format!("{}", TransportKind::Cli), "cli");
    }

    #[test]
    fn test_transport_kind_default() {
        assert_eq!(TransportKind::default(), TransportKind::Acp);
    }

    #[test]
    fn test_transport_kind_equality() {
        assert_eq!(TransportKind::Acp, TransportKind::Acp);
        assert_eq!(TransportKind::Cli, TransportKind::Cli);
        assert_ne!(TransportKind::Acp, TransportKind::Cli);
    }

    #[test]
    fn test_transport_kind_debug() {
        let acp = TransportKind::Acp;
        let cli = TransportKind::Cli;
        assert_eq!(format!("{:?}", acp), "Acp");
        assert_eq!(format!("{:?}", cli), "Cli");
    }

    #[test]
    fn test_transport_kind_clone() {
        let original = TransportKind::Cli;
        let cloned = original;
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_transport_kind_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(TransportKind::Acp);
        set.insert(TransportKind::Cli);
        assert_eq!(set.len(), 2);
        assert!(set.contains(&TransportKind::Acp));
        assert!(set.contains(&TransportKind::Cli));
    }

    // ========================================================================
    // AgentType Tests
    // ========================================================================

    #[test]
    fn test_agent_type_as_str() {
        assert_eq!(AgentType::Orchestrator.as_str(), "orchestrator");
        assert_eq!(AgentType::Planner.as_str(), "planner");
        assert_eq!(AgentType::Implementer.as_str(), "implementer");
    }

    #[test]
    fn test_agent_type_display() {
        assert_eq!(format!("{}", AgentType::Orchestrator), "orchestrator");
        assert_eq!(format!("{}", AgentType::Planner), "planner");
        assert_eq!(format!("{}", AgentType::Implementer), "implementer");
    }

    #[test]
    fn test_agent_type_equality() {
        assert_eq!(AgentType::Orchestrator, AgentType::Orchestrator);
        assert_eq!(AgentType::Planner, AgentType::Planner);
        assert_eq!(AgentType::Implementer, AgentType::Implementer);
        assert_ne!(AgentType::Orchestrator, AgentType::Planner);
        assert_ne!(AgentType::Planner, AgentType::Implementer);
    }

    #[test]
    fn test_agent_type_debug() {
        assert_eq!(format!("{:?}", AgentType::Orchestrator), "Orchestrator");
        assert_eq!(format!("{:?}", AgentType::Planner), "Planner");
        assert_eq!(format!("{:?}", AgentType::Implementer), "Implementer");
    }

    #[test]
    fn test_agent_type_clone() {
        let original = AgentType::Implementer;
        let cloned = original;
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_agent_type_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(AgentType::Orchestrator);
        set.insert(AgentType::Planner);
        set.insert(AgentType::Implementer);
        assert_eq!(set.len(), 3);
    }

    // ========================================================================
    // SessionConfig Tests
    // ========================================================================

    #[test]
    fn test_session_config_builder() {
        let config = SessionConfig::new("sonnet-4.6", "/workspace").with_mode(SessionMode::Plan);

        assert_eq!(config.model, "sonnet-4.6");
        assert_eq!(config.workspace, "/workspace");
        assert_eq!(config.mode, SessionMode::Plan);
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_session_config_with_mcp_servers() {
        let servers = vec![
            json!({"name": "server1", "command": "cmd1"}),
            json!({"name": "server2", "command": "cmd2"}),
        ];

        let config = SessionConfig::new("opus-4.6", "/workspace").with_mcp_servers(servers);

        assert_eq!(config.mcp_servers.len(), 2);
        assert_eq!(config.mcp_servers[0]["name"], "server1");
        assert_eq!(config.mcp_servers[1]["name"], "server2");
    }

    #[test]
    fn test_session_config_default_mode() {
        let config = SessionConfig::new("model", "/workspace");
        assert_eq!(config.mode, SessionMode::default());
    }

    #[test]
    fn test_session_config_chaining() {
        let config = SessionConfig::new("model", "/workspace")
            .with_mcp_servers(vec![json!({"name": "test"})])
            .with_mode(SessionMode::Agent);

        assert_eq!(config.model, "model");
        assert_eq!(config.workspace, "/workspace");
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mode, SessionMode::Agent);
    }

    #[test]
    fn test_session_config_from_string() {
        let model = String::from("model-name");
        let workspace = String::from("/path/to/workspace");
        let config = SessionConfig::new(model, workspace);

        assert_eq!(config.model, "model-name");
        assert_eq!(config.workspace, "/path/to/workspace");
    }

    // ========================================================================
    // SessionInfo Tests
    // ========================================================================

    #[test]
    fn test_session_info() {
        let info = SessionInfo::new("session-123");
        assert_eq!(info.session_id, "session-123");
    }

    #[test]
    fn test_session_info_from_string() {
        let id = String::from("session-456");
        let info = SessionInfo::new(id);
        assert_eq!(info.session_id, "session-456");
    }

    #[test]
    fn test_session_info_serialization() {
        let info = SessionInfo::new("sess-789");
        let serialized = serde_json::to_string(&info).unwrap();
        assert!(serialized.contains("sess-789"));
    }

    #[test]
    fn test_session_info_deserialization() {
        let json = r#"{"session_id":"sess-abc"}"#;
        let info: SessionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.session_id, "sess-abc");
    }

    #[test]
    fn test_session_info_roundtrip() {
        let original = SessionInfo::new("roundtrip-test");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: SessionInfo = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original.session_id, deserialized.session_id);
    }

    // ========================================================================
    // ToolResult Tests
    // ========================================================================

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success("Task created");
        assert!(result.is_success);
        assert_eq!(result.content, "Task created");
    }

    #[test]
    fn test_tool_result_failure() {
        let result = ToolResult::failure("Task not found");
        assert!(!result.is_success);
        assert_eq!(result.content, "Task not found");
    }

    #[test]
    fn test_tool_result_success_empty_content() {
        let result = ToolResult::success("");
        assert!(result.is_success);
        assert_eq!(result.content, "");
    }

    #[test]
    fn test_tool_result_from_string() {
        let content = String::from("Result content");
        let result = ToolResult::success(content);
        assert_eq!(result.content, "Result content");
    }

    #[test]
    fn test_tool_result_serialization() {
        let result = ToolResult::success("test");
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(serialized.contains("test"));
        assert!(serialized.contains("is_success"));
    }

    #[test]
    fn test_tool_result_deserialization() {
        let json = r#"{"content":"result data","is_success":true}"#;
        let result: ToolResult = serde_json::from_str(json).unwrap();
        assert!(result.is_success);
        assert_eq!(result.content, "result data");
    }

    // ========================================================================
    // SessionUpdate Tests
    // ========================================================================

    #[test]
    fn test_session_update_session_id() {
        let text = SessionUpdate::Text {
            session_id: "sess-1".into(),
            content: "Hello".into(),
        };
        assert_eq!(text.session_id(), Some("sess-1"));

        let raw = SessionUpdate::Raw {
            session_id: None,
            data: json!({}),
        };
        assert_eq!(raw.session_id(), None);
    }

    #[test]
    fn test_session_update_session_id_all_variants() {
        // Text
        let text = SessionUpdate::Text {
            session_id: "s1".into(),
            content: String::new(),
        };
        assert_eq!(text.session_id(), Some("s1"));

        // ToolUse
        let tool_use = SessionUpdate::ToolUse {
            session_id: "s2".into(),
            tool_use_id: "t1".into(),
            tool_name: "test".into(),
            input: json!({}),
        };
        assert_eq!(tool_use.session_id(), Some("s2"));

        // ToolResult
        let tool_result = SessionUpdate::ToolResult {
            session_id: "s3".into(),
            tool_use_id: "t1".into(),
            content: String::new(),
            is_success: true,
        };
        assert_eq!(tool_result.session_id(), Some("s3"));

        // Completion
        let completion = SessionUpdate::Completion {
            session_id: "s4".into(),
            result: None,
            success: true,
        };
        assert_eq!(completion.session_id(), Some("s4"));

        // Raw with session_id
        let raw_with_id = SessionUpdate::Raw {
            session_id: Some("s5".into()),
            data: json!({}),
        };
        assert_eq!(raw_with_id.session_id(), Some("s5"));
    }

    #[test]
    fn test_session_update_is_completion() {
        let completion = SessionUpdate::Completion {
            session_id: "sess-1".into(),
            result: None,
            success: true,
        };
        assert!(completion.is_completion());

        let text = SessionUpdate::Text {
            session_id: "sess-1".into(),
            content: "Hello".into(),
        };
        assert!(!text.is_completion());
    }

    #[test]
    fn test_session_update_is_tool_use() {
        let tool_use = SessionUpdate::ToolUse {
            session_id: "sess-1".into(),
            tool_use_id: "call-1".into(),
            tool_name: "complete".into(),
            input: json!({}),
        };
        assert!(tool_use.is_tool_use());

        let text = SessionUpdate::Text {
            session_id: "sess-1".into(),
            content: "Hello".into(),
        };
        assert!(!text.is_tool_use());
    }

    #[test]
    fn test_session_update_is_completion_all_variants() {
        // Only Completion should return true
        let completion = SessionUpdate::Completion {
            session_id: String::new(),
            result: None,
            success: true,
        };
        assert!(completion.is_completion());

        let text = SessionUpdate::Text {
            session_id: String::new(),
            content: String::new(),
        };
        assert!(!text.is_completion());

        let tool_use = SessionUpdate::ToolUse {
            session_id: String::new(),
            tool_use_id: String::new(),
            tool_name: String::new(),
            input: json!({}),
        };
        assert!(!tool_use.is_completion());

        let tool_result = SessionUpdate::ToolResult {
            session_id: String::new(),
            tool_use_id: String::new(),
            content: String::new(),
            is_success: true,
        };
        assert!(!tool_result.is_completion());

        let raw = SessionUpdate::Raw {
            session_id: None,
            data: json!({}),
        };
        assert!(!raw.is_completion());
    }

    #[test]
    fn test_session_update_is_tool_use_all_variants() {
        // Only ToolUse should return true
        let tool_use = SessionUpdate::ToolUse {
            session_id: String::new(),
            tool_use_id: String::new(),
            tool_name: String::new(),
            input: json!({}),
        };
        assert!(tool_use.is_tool_use());

        let text = SessionUpdate::Text {
            session_id: String::new(),
            content: String::new(),
        };
        assert!(!text.is_tool_use());

        let completion = SessionUpdate::Completion {
            session_id: String::new(),
            result: None,
            success: true,
        };
        assert!(!completion.is_tool_use());

        let tool_result = SessionUpdate::ToolResult {
            session_id: String::new(),
            tool_use_id: String::new(),
            content: String::new(),
            is_success: true,
        };
        assert!(!tool_result.is_tool_use());

        let raw = SessionUpdate::Raw {
            session_id: None,
            data: json!({}),
        };
        assert!(!raw.is_tool_use());
    }

    // ========================================================================
    // SessionUpdate Serialization Tests
    // ========================================================================

    #[test]
    fn test_session_update_text_serialization() {
        let update = SessionUpdate::Text {
            session_id: "sess-1".into(),
            content: "Hello, world!".into(),
        };
        let serialized = serde_json::to_string(&update).unwrap();
        assert!(serialized.contains(r#""type":"text""#));
        assert!(serialized.contains("sess-1"));
        assert!(serialized.contains("Hello, world!"));
    }

    #[test]
    fn test_session_update_text_deserialization() {
        let json = r#"{"type":"text","session_id":"s1","content":"test content"}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::Text {
            session_id,
            content,
        } = update
        {
            assert_eq!(session_id, "s1");
            assert_eq!(content, "test content");
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn test_session_update_tool_use_serialization() {
        let update = SessionUpdate::ToolUse {
            session_id: "sess-1".into(),
            tool_use_id: "call-1".into(),
            tool_name: "complete".into(),
            input: json!({"success": true}),
        };
        let serialized = serde_json::to_string(&update).unwrap();
        assert!(serialized.contains(r#""type":"tool_use""#));
        assert!(serialized.contains("complete"));
    }

    #[test]
    fn test_session_update_tool_use_deserialization() {
        let json = r#"{"type":"tool_use","session_id":"s1","tool_use_id":"t1","tool_name":"test","input":{"key":"value"}}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::ToolUse {
            session_id,
            tool_use_id,
            tool_name,
            input,
        } = update
        {
            assert_eq!(session_id, "s1");
            assert_eq!(tool_use_id, "t1");
            assert_eq!(tool_name, "test");
            assert_eq!(input["key"], "value");
        } else {
            panic!("Expected ToolUse variant");
        }
    }

    #[test]
    fn test_session_update_tool_result_serialization() {
        let update = SessionUpdate::ToolResult {
            session_id: "sess-1".into(),
            tool_use_id: "call-1".into(),
            content: "File saved".into(),
            is_success: true,
        };
        let serialized = serde_json::to_string(&update).unwrap();
        assert!(serialized.contains(r#""type":"tool_result""#));
        assert!(serialized.contains("File saved"));
    }

    #[test]
    fn test_session_update_tool_result_deserialization() {
        let json = r#"{"type":"tool_result","session_id":"s1","tool_use_id":"t1","content":"done","is_success":false}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::ToolResult {
            session_id,
            tool_use_id,
            content,
            is_success,
        } = update
        {
            assert_eq!(session_id, "s1");
            assert_eq!(tool_use_id, "t1");
            assert_eq!(content, "done");
            assert!(!is_success);
        } else {
            panic!("Expected ToolResult variant");
        }
    }

    #[test]
    fn test_session_update_tool_result_default_is_success() {
        // When is_success is missing, it should default to true
        let json =
            r#"{"type":"tool_result","session_id":"s1","tool_use_id":"t1","content":"done"}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::ToolResult { is_success, .. } = update {
            assert!(is_success);
        } else {
            panic!("Expected ToolResult variant");
        }
    }

    #[test]
    fn test_session_update_completion_serialization() {
        let update = SessionUpdate::Completion {
            session_id: "sess-1".into(),
            result: Some("Task completed".into()),
            success: true,
        };
        let serialized = serde_json::to_string(&update).unwrap();
        assert!(serialized.contains(r#""type":"completion""#));
        assert!(serialized.contains("Task completed"));
    }

    #[test]
    fn test_session_update_completion_deserialization() {
        let json = r#"{"type":"completion","session_id":"s1","result":"all done","success":true}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::Completion {
            session_id,
            result,
            success,
        } = update
        {
            assert_eq!(session_id, "s1");
            assert_eq!(result, Some("all done".into()));
            assert!(success);
        } else {
            panic!("Expected Completion variant");
        }
    }

    #[test]
    fn test_session_update_completion_default_success() {
        // When success is missing, it should default to true
        let json = r#"{"type":"completion","session_id":"s1"}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::Completion { success, .. } = update {
            assert!(success);
        } else {
            panic!("Expected Completion variant");
        }
    }

    #[test]
    fn test_session_update_raw_serialization() {
        let update = SessionUpdate::Raw {
            session_id: Some("sess-1".into()),
            data: json!({"custom": "data"}),
        };
        let serialized = serde_json::to_string(&update).unwrap();
        assert!(serialized.contains(r#""type":"raw""#));
        assert!(serialized.contains("custom"));
    }

    #[test]
    fn test_session_update_raw_deserialization() {
        let json = r#"{"type":"raw","session_id":"s1","data":{"foo":"bar"}}"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        if let SessionUpdate::Raw { session_id, data } = update {
            assert_eq!(session_id, Some("s1".into()));
            assert_eq!(data["foo"], "bar");
        } else {
            panic!("Expected Raw variant");
        }
    }

    #[test]
    fn test_session_update_roundtrip_all_variants() {
        let updates = vec![
            SessionUpdate::Text {
                session_id: "s1".into(),
                content: "hello".into(),
            },
            SessionUpdate::ToolUse {
                session_id: "s2".into(),
                tool_use_id: "t1".into(),
                tool_name: "test".into(),
                input: json!({"a": 1}),
            },
            SessionUpdate::ToolResult {
                session_id: "s3".into(),
                tool_use_id: "t1".into(),
                content: "result".into(),
                is_success: true,
            },
            SessionUpdate::Completion {
                session_id: "s4".into(),
                result: Some("done".into()),
                success: true,
            },
            SessionUpdate::Raw {
                session_id: Some("s5".into()),
                data: json!({"x": "y"}),
            },
        ];

        for update in updates {
            let serialized = serde_json::to_string(&update).unwrap();
            let deserialized: SessionUpdate = serde_json::from_str(&serialized).unwrap();

            // Check session IDs match
            assert_eq!(update.session_id(), deserialized.session_id());
        }
    }
}
