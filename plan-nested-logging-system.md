# Nested Logging System Implementation Plan

## 1. Overview

### Goals
- Create a hierarchical logging system that mirrors the agent hierarchy (Planner → Orchestrator → Implementer/nested decomposition)
- Separate log files per agent type within each hierarchy level
- Design for future streaming to both files and UI via channels/observers
- Maintain compatibility with existing `tracing` infrastructure for app-level logs

### Architecture Assumption
This plan assumes the **"Always Start with Planner"** architecture:
- Every run begins with a Planner that creates a plan from the user's goal
- An Orchestrator then executes the plan, calling `implement()` or `decompose()` for each task
- `decompose()` creates a nested level with its own Planner → Orchestrator → agents
- This provides consistent structure and predictable logging patterns

### Success Criteria
- Each app run creates its own timestamped folder
- Agent conversations are captured in separate, readable log files
- Nested decompose calls create subfolder hierarchies
- Concurrent implementers at the same level get distinct log files
- Clean API that integrates with existing `App` and agent spawning code

### Scope
- **Included**: File-based logging, directory structure, agent output capture, streaming-ready design
- **Excluded**: UI streaming implementation (design interface only), log aggregation/search

---

## 2. Directory Structure

### Example Structure for a Run
```
logs/
└── 2026-03-05_14-32-15_abc123/           # Run folder (timestamp + short UUID)
    ├── app.log                            # App-level tracing logs
    ├── planner.log                        # Root planner (always present)
    ├── orchestrator.log                   # Root orchestrator executing the plan
    ├── implementer-001.log                # Direct implementations at root
    ├── implementer-002.log                # Concurrent implementer
    ├── subtask-001/                       # First decompose call
    │   ├── planner.log                    # Planner for this subtask
    │   ├── orchestrator.log               # Orchestrator executing subtask plan
    │   ├── implementer-001.log            # Implementer
    │   └── subtask-001/                   # Nested decompose
    │       ├── planner.log
    │       ├── orchestrator.log
    │       └── implementer-001.log
    └── subtask-002/                       # Second decompose at root level
        ├── planner.log
        ├── orchestrator.log
        └── implementer-001.log
```

### Naming Conventions
- **Run folder**: `YYYY-MM-DD_HH-MM-SS_{short_uuid}` - human-readable with collision protection
- **Subtask subfolders**: `subtask-{NNN}` - sequential within parent, represents decomposed work
- **Agent logs**: `{agent_type}.log` or `{agent_type}-{NNN}.log` for concurrent agents

---

## 3. Core Types

### 3.1 Module Structure

```
src/
└── logging/
    ├── mod.rs              # Module exports
    ├── manager.rs          # RunLogManager - manages the run directory
    ├── scope.rs            # LogScope - represents a logging context/hierarchy level
    ├── writer.rs           # AgentWriter - writes to individual log files
    ├── stream.rs           # LogStream, LogEvent - for streaming/observation
    └── format.rs           # Formatting utilities for agent output
```

### 3.2 RunLogManager

The central coordinator for a single run's logging.

```rust
// src/logging/manager.rs

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages the log directory for a single application run.
/// 
/// Creates the run folder on initialization and provides factory methods
/// for creating scoped loggers for the agent hierarchy.
pub struct RunLogManager {
    /// Root directory for this run (e.g., logs/2026-03-05_14-32-15_abc123/)
    run_dir: PathBuf,
    /// Broadcast channel for streaming log events to observers (e.g., UI)
    event_tx: tokio::sync::broadcast::Sender<LogEvent>,
    /// Counter for orchestrator subfolders at root level
    root_orchestrator_count: Arc<RwLock<u32>>,
}

impl RunLogManager {
    /// Create a new run log manager.
    /// 
    /// Creates the run directory with format: `{base_dir}/{timestamp}_{short_uuid}/`
    pub fn new(base_dir: &str) -> std::io::Result<Self> {
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let short_uuid = &uuid::Uuid::new_v4().to_string()[..8];
        let run_dir = PathBuf::from(base_dir).join(format!("{}_{}", timestamp, short_uuid));
        
        std::fs::create_dir_all(&run_dir)?;
        
        let (event_tx, _) = tokio::sync::broadcast::channel(1000);
        
        Ok(Self {
            run_dir,
            event_tx,
            root_orchestrator_count: Arc::new(RwLock::new(0)),
        })
    }
    
    /// Get the run directory path.
    pub fn run_dir(&self) -> &PathBuf {
        &self.run_dir
    }
    
    /// Create the root LogScope for this run.
    /// 
    /// The root scope is where the app and root orchestrator logs live.
    pub fn root_scope(&self) -> LogScope {
        LogScope::new(
            self.run_dir.clone(),
            self.event_tx.clone(),
            0, // depth
        )
    }
    
    /// Subscribe to log events for streaming to UI.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LogEvent> {
        self.event_tx.subscribe()
    }
}
```

### 3.3 LogScope

Represents a logging context at a specific level of the hierarchy.

```rust
// src/logging/scope.rs

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// A logging scope representing one level in the agent hierarchy.
///
/// Each scope can create writers for agents at this level and
/// child scopes for nested decomposition (subtasks).
#[derive(Clone)]
pub struct LogScope {
    /// Directory for this scope's logs
    dir: PathBuf,
    /// Broadcast channel for log events
    event_tx: broadcast::Sender<LogEvent>,
    /// Depth in the hierarchy (0 = root)
    depth: u32,
    /// Counter for implementers at this level
    implementer_count: Arc<RwLock<u32>>,
    /// Counter for subtasks (decomposed work) at this level
    subtask_count: Arc<RwLock<u32>>,
}

impl LogScope {
    pub fn new(
        dir: PathBuf,
        event_tx: broadcast::Sender<LogEvent>,
        depth: u32,
    ) -> Self {
        Self {
            dir,
            event_tx,
            depth,
            implementer_count: Arc::new(RwLock::new(0)),
            subtask_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Create a writer for the planner at this scope level.
    /// In the "always planner first" architecture, this is called first.
    pub async fn planner_writer(&self) -> std::io::Result<AgentWriter> {
        AgentWriter::new(
            self.dir.join("planner.log"),
            AgentType::Planner,
            self.event_tx.clone(),
            self.depth,
        ).await
    }

    /// Create a writer for the orchestrator at this scope level.
    /// Called after planner creates the plan.
    pub async fn orchestrator_writer(&self) -> std::io::Result<AgentWriter> {
        AgentWriter::new(
            self.dir.join("orchestrator.log"),
            AgentType::Orchestrator,
            self.event_tx.clone(),
            self.depth,
        ).await
    }

    /// Create a writer for a new implementer at this scope level.
    /// Returns a numbered writer (implementer-001.log, etc.)
    pub async fn implementer_writer(&self) -> std::io::Result<AgentWriter> {
        let mut count = self.implementer_count.write().await;
        *count += 1;
        let filename = format!("implementer-{:03}.log", *count);

        AgentWriter::new(
            self.dir.join(&filename),
            AgentType::Implementer { index: *count },
            self.event_tx.clone(),
            self.depth,
        ).await
    }

    /// Create a child scope for a decompose operation (subtask).
    /// Creates a new subdirectory and returns a LogScope for it.
    pub async fn child_scope(&self) -> std::io::Result<LogScope> {
        let mut count = self.subtask_count.write().await;
        *count += 1;
        let subdir = self.dir.join(format!("subtask-{:03}", *count));

        std::fs::create_dir_all(&subdir)?;

        Ok(LogScope::new(
            subdir,
            self.event_tx.clone(),
            self.depth + 1,
        ))
    }

    /// Get the directory path for this scope.
    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    /// Get the depth of this scope in the hierarchy.
    pub fn depth(&self) -> u32 {
        self.depth
    }
}
```

### 3.4 AgentWriter

Handles writing to individual agent log files.

```rust
// src/logging/writer.rs

use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::broadcast;
use chrono::Local;

/// Type of agent for log identification.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentType {
    Orchestrator,
    Planner,
    Implementer { index: u32 },
}

impl AgentType {
    pub fn name(&self) -> String {
        match self {
            AgentType::Orchestrator => "orchestrator".to_string(),
            AgentType::Planner => "planner".to_string(),
            AgentType::Implementer { index } => format!("implementer-{:03}", index),
        }
    }
}

/// Writes agent output to a log file and broadcasts events.
pub struct AgentWriter {
    file: BufWriter<File>,
    path: PathBuf,
    agent_type: AgentType,
    event_tx: broadcast::Sender<LogEvent>,
    depth: u32,
    /// Session ID this writer is associated with (set after session creation)
    session_id: Option<String>,
}

impl AgentWriter {
    pub async fn new(
        path: PathBuf,
        agent_type: AgentType,
        event_tx: broadcast::Sender<LogEvent>,
        depth: u32,
    ) -> std::io::Result<Self> {
        let file = File::create(&path).await?;

        Ok(Self {
            file: BufWriter::new(file),
            path,
            agent_type,
            event_tx,
            depth,
            session_id: None,
        })
    }

    /// Associate this writer with a session ID.
    pub fn set_session_id(&mut self, session_id: String) {
        self.session_id = Some(session_id);
    }

    /// Write a header at the start of the log file.
    pub async fn write_header(&mut self, task: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let header = format!(
            "=== {} Log ===\nStarted: {}\nTask: {}\n{}\n\n",
            self.agent_type.name().to_uppercase(),
            timestamp,
            task,
            "=".repeat(60)
        );
        self.file.write_all(header.as_bytes()).await?;
        self.file.flush().await
    }

    /// Write an agent message chunk (streaming text).
    pub async fn write_message_chunk(&mut self, text: &str) -> std::io::Result<()> {
        self.file.write_all(text.as_bytes()).await?;

        // Broadcast event for UI streaming
        let _ = self.event_tx.send(LogEvent::AgentMessage {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            content: text.to_string(),
        });

        Ok(())
    }

    /// Write a tool call event.
    pub async fn write_tool_call(&mut self, tool_name: &str, args: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%H:%M:%S");
        let line = format!("\n[{}] 🔧 Tool: {} - {}\n", timestamp, tool_name, args);
        self.file.write_all(line.as_bytes()).await?;

        let _ = self.event_tx.send(LogEvent::ToolCall {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            tool_name: tool_name.to_string(),
        });

        Ok(())
    }

    /// Write a tool result.
    pub async fn write_tool_result(&mut self, tool_name: &str, is_error: bool, content: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%H:%M:%S");
        let icon = if is_error { "❌" } else { "✅" };
        let line = format!("[{}] {} Tool result: {}\n{}\n", timestamp, icon, tool_name, content);
        self.file.write_all(line.as_bytes()).await?;

        let _ = self.event_tx.send(LogEvent::ToolResult {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            tool_name: tool_name.to_string(),
            is_error,
        });

        Ok(())
    }

    /// Write completion marker and flush.
    pub async fn finalize(&mut self, success: bool) -> std::io::Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let status = if success { "SUCCESS" } else { "FAILURE" };
        let footer = format!(
            "\n{}\nCompleted: {} - {}\n",
            "=".repeat(60),
            timestamp,
            status
        );
        self.file.write_all(footer.as_bytes()).await?;
        self.file.flush().await?;

        let _ = self.event_tx.send(LogEvent::AgentComplete {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            success,
        });

        Ok(())
    }
}
```

### 3.5 LogEvent (for streaming)

```rust
// src/logging/stream.rs

use super::writer::AgentType;

/// Events broadcast for UI streaming and observation.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// Agent started a new session.
    AgentStarted {
        agent_type: AgentType,
        session_id: String,
        depth: u32,
        task: String,
    },

    /// Agent sent a message chunk (streaming text).
    AgentMessage {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        content: String,
    },

    /// Agent made a tool call.
    ToolCall {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        tool_name: String,
    },

    /// Tool call completed.
    ToolResult {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        tool_name: String,
        is_error: bool,
    },

    /// Agent completed its work.
    AgentComplete {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        success: bool,
    },

    /// A new child scope was created (subtask from decompose).
    SubtaskCreated {
        parent_depth: u32,
        new_depth: u32,
        path: String,
        task_description: String,
    },
}
```

---

## 4. Integration with Existing Code

**Note**: This section assumes the "Always Start with Planner" architecture change has been made.
The entry point now calls planner first, then orchestrator executes the resulting plan.

### 4.1 Changes to App struct

```rust
// src/app.rs - Updated App struct

pub struct App {
    acp_orchestrator: AcpClient,
    acp_worker: AcpClient,
    socket_path: Option<PathBuf>,
    tool_rx: Option<mpsc::Receiver<ToolMessage>>,
    model_config: ModelConfig,
    timeout_config: TimeoutConfig,
    original_goal: String,
    plan_stack: Vec<PlanLevel>,

    // NEW: Logging infrastructure
    /// Log manager for this run
    log_manager: Arc<RunLogManager>,
    /// Current logging scope (changes during decompose/subtasks)
    current_scope: LogScope,
    /// Writer for the current planner (set during planning phase)
    planner_writer: Option<AgentWriter>,
    /// Writer for the current orchestrator (set during execution phase)
    orchestrator_writer: Option<AgentWriter>,
}
```

### 4.2 Updated App::new()

```rust
impl App {
    pub async fn new(model_config: ModelConfig) -> Result<Self> {
        Self::with_timeout_config(model_config, TimeoutConfig::default()).await
    }

    pub async fn with_timeout_config(
        model_config: ModelConfig,
        timeout_config: TimeoutConfig,
    ) -> Result<Self> {
        let orchestrator_cache = Self::setup_orchestrator_cache()?;

        let mut acp_orchestrator = AcpClient::spawn(Some(&orchestrator_cache)).await?;
        acp_orchestrator.initialize().await?;

        let mut acp_worker = AcpClient::spawn(None).await?;
        acp_worker.initialize().await?;

        // NEW: Initialize logging
        let log_dir = std::env::var("VILLALOBOS_LOG_DIR").unwrap_or_else(|_| "logs".to_string());
        let log_manager = Arc::new(RunLogManager::new(&log_dir)?);
        let current_scope = log_manager.root_scope();

        tracing::info!("📁 Run logs: {:?}", log_manager.run_dir());

        Ok(Self {
            acp_orchestrator,
            acp_worker,
            socket_path: None,
            tool_rx: None,
            model_config,
            timeout_config,
            original_goal: String::new(),
            plan_stack: Vec::new(),
            log_manager,
            current_scope,
            orchestrator_writer: None,
        })
    }
}
```

### 4.3 Updated handle_acp_message()

Route ACP messages to the appropriate AgentWriter based on session ID:

```rust
/// Handle ACP messages and route to appropriate log writers
async fn handle_acp_message(
    &mut self,
    msg: &serde_json::Value,
    agent_type: &str,
    writer: &mut Option<AgentWriter>,
) {
    if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
        if method == "session/update" {
            if let Some(params) = msg.get("params") {
                if let Some(update) = params.get("update") {
                    if let Some(session_update) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
                        match session_update {
                            "agent_message_chunk" => {
                                if let Some(text) = update.get("content")
                                    .and_then(|c| c.get("text"))
                                    .and_then(|t| t.as_str())
                                {
                                    // Write to appropriate log file
                                    if let Some(w) = writer.as_mut() {
                                        let _ = w.write_message_chunk(text).await;
                                    }
                                    // Still stream to console
                                    print!("{}", text);
                                    std::io::Write::flush(&mut std::io::stdout()).ok();
                                }
                            }
                            "tool_call" => {
                                if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                                    if let Some(w) = writer.as_mut() {
                                        let _ = w.write_tool_call(title, "").await;
                                    }
                                    tracing::info!("🔧 {} tool call: {}", agent_type, title);
                                }
                            }
                            "tool_result" => {
                                let title = update.get("title").and_then(|t| t.as_str()).unwrap_or("unknown");
                                let is_error = update.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
                                let content = update.get("content")
                                    .and_then(|c| c.get("text"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("");

                                if let Some(w) = writer.as_mut() {
                                    let _ = w.write_tool_result(title, is_error, content).await;
                                }
                            }
                            "plan" => {
                                if let Some(entries) = update.get("entries").and_then(|e| e.as_array()) {
                                    tracing::info!("📋 {} created plan with {} entries", agent_type, entries.len());
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
```

### 4.4 Updated spawn_implementer()

Create a new implementer writer when spawning:

```rust
async fn spawn_implementer(&mut self, task: &str) -> Result<(String, AgentWriter)> {
    let cwd = std::env::current_dir()?.to_string_lossy().to_string();

    let response = self
        .acp_worker
        .session_new(self.model_config.implementer_model.as_str(), vec![], &cwd)
        .await?;

    // Create implementer-specific log writer
    let mut writer = self.current_scope.implementer_writer().await?;
    writer.set_session_id(response.session_id.clone());
    writer.write_header(task).await?;

    let context = self.build_implementer_context();
    let prompt = IMPLEMENTER_PROMPT
        .replace("{task}", task)
        .replace("{user_goal}", &self.original_goal)
        .replace("{context}", &context);

    self.acp_worker.session_prompt(&response.session_id, &prompt).await?;

    Ok((response.session_id, writer))
}
```

### 4.5 Updated App::run() - New Entry Point

With "Always Start with Planner" architecture, the run flow is:

```rust
/// Run the app with a goal - always starts with planner
pub async fn run(&mut self, goal: &str) -> Result<TaskResult> {
    tracing::info!("Starting with goal: {}", goal);
    self.original_goal = goal.to_string();

    // Set up Unix socket for MCP server communication
    let socket_path = self.setup_socket().await?;

    // 1. Create planner writer at root scope
    let mut planner_writer = self.current_scope.planner_writer().await?;

    // 2. Spawn planner to create initial plan
    let planner_session = self.spawn_planner(goal).await?;
    planner_writer.set_session_id(planner_session.clone());
    planner_writer.write_header(goal).await?;

    // 3. Wait for plan with logging
    let plan = self.wait_for_plan_with_logging(&planner_session, &mut planner_writer).await?;
    planner_writer.finalize(true).await?;

    tracing::info!("📋 Root plan created with {} entries", plan.entries.len());

    // 4. Create orchestrator writer
    let mut orch_writer = self.current_scope.orchestrator_writer().await?;

    // 5. Run orchestrator to execute the plan
    let result = self.run_orchestrator_with_logging(&plan, &mut orch_writer).await?;
    orch_writer.finalize(result.success).await?;

    // Clean up socket
    if let Err(e) = std::fs::remove_file(&socket_path) {
        tracing::warn!("Failed to remove socket file: {}", e);
    }

    Ok(result)
}
```

### 4.6 Updated handle_decompose_inner() - Creates Subtask Scope

When orchestrator calls decompose(), create a child scope (subtask):

```rust
async fn handle_decompose_inner(&mut self, task: &str) -> Result<String> {
    tracing::info!("🔄 Decomposing task into subtask: {}", task);

    // Create child scope (subtask folder) for this decomposition
    let child_scope = self.current_scope.child_scope().await?;
    let previous_scope = std::mem::replace(&mut self.current_scope, child_scope);

    // Subtask follows same pattern: planner first, then orchestrator
    // 1. Create planner writer for subtask
    let mut planner_writer = self.current_scope.planner_writer().await?;

    // 2. Spawn planner for subtask
    let planner_session = self.spawn_planner(task).await?;
    planner_writer.set_session_id(planner_session.clone());
    planner_writer.write_header(task).await?;

    // 3. Wait for subtask plan
    let plan = self.wait_for_plan_with_logging(&planner_session, &mut planner_writer).await?;
    planner_writer.finalize(true).await?;

    tracing::info!("📋 Subtask plan created with {} entries", plan.entries.len());

    // 4. Create orchestrator writer for subtask
    let mut orch_writer = self.current_scope.orchestrator_writer().await?;

    // 5. Run orchestrator to execute subtask plan
    let result = self.run_orchestrator_with_logging(&plan, &mut orch_writer).await?;
    orch_writer.finalize(result.success).await?;

    // Restore previous scope
    self.current_scope = previous_scope;

    Ok(format!(
        "Decomposed into {} subtasks and executed them. Result: {}",
        plan.entries.len(),
        if result.success { "success" } else { "failure" }
    ))
}
```

---

## 5. Prerequisites

### 5.1 New Dependencies

Add to `Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
chrono = "0.4"  # For timestamps in log filenames and entries
```

### 5.2 Module Registration

Add to `src/main.rs`:

```rust
mod logging;  // New module
```

---

## 6. Implementation Steps

### Step 1: Create logging module structure
- **Files**: `src/logging/mod.rs`, `src/logging/stream.rs`
- Create the `LogEvent` enum and module exports
- **Testing**: Unit test that `LogEvent` can be cloned and debug-printed

### Step 2: Implement AgentWriter
- **Files**: `src/logging/writer.rs`
- Implement `AgentType` enum and `AgentWriter` struct
- Support async file writing with `tokio::fs`
- **Testing**: Unit test writing to a temp file and verifying content

### Step 3: Implement LogScope
- **Files**: `src/logging/scope.rs`
- Implement scope creation, child scope generation, and counter management
- **Testing**: Test that child scopes create correct directory structure

### Step 4: Implement RunLogManager
- **Files**: `src/logging/manager.rs`
- Implement run directory creation with timestamp + UUID naming
- Integrate broadcast channel for event streaming
- **Testing**: Test directory creation and naming format

### Step 5: Integrate with App struct
- **Files**: `src/app.rs`
- Add `log_manager`, `current_scope`, `orchestrator_writer` fields
- Update `App::new()` to initialize logging
- **Testing**: Verify App creates log directory on startup

### Step 6: Update agent spawning
- **Files**: `src/app.rs`
- Modify `spawn_orchestrator()`, `spawn_planner()`, `spawn_implementer()` to create writers
- Pass writers to `handle_acp_message()` for routing
- **Testing**: Manual test that agent logs appear in correct files

### Step 7: Update decompose flow
- **Files**: `src/app.rs`
- Create child scope on decompose, restore parent scope on completion
- Pass scope to recursive orchestrator calls
- **Testing**: Test nested decompose creates correct folder hierarchy

### Step 8: Update ACP message handling
- **Files**: `src/app.rs`
- Route messages to correct writer based on session tracking
- Maintain session-to-writer mapping
- **Testing**: Verify concurrent implementers write to separate files

### Step 9: Migrate main.rs logging setup
- **Files**: `src/main.rs`
- Keep tracing for app-level logs, redirect to `app.log` in run directory
- Remove global file appender, use run-specific path
- **Testing**: End-to-end test of complete logging flow

### Step 10: Add streaming subscription API
- **Files**: `src/logging/mod.rs`
- Expose `subscribe()` method for future UI integration
- Document usage pattern
- **Testing**: Test that broadcast channel receives events

---

## 7. File Changes Summary

### New Files
| Path | Description |
|------|-------------|
| `src/logging/mod.rs` | Module exports and re-exports |
| `src/logging/manager.rs` | `RunLogManager` implementation |
| `src/logging/scope.rs` | `LogScope` implementation |
| `src/logging/writer.rs` | `AgentWriter` and `AgentType` |
| `src/logging/stream.rs` | `LogEvent` enum for streaming |

### Modified Files
| Path | Changes |
|------|---------|
| `src/main.rs` | Add `mod logging;`, update tracing setup for run-specific path |
| `src/app.rs` | Add logging fields, update agent spawning, ACP message routing |
| `Cargo.toml` | Add `chrono` dependency |

---

## 8. Testing Strategy

### Unit Tests

```rust
// src/logging/writer.rs
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_agent_writer_creates_file() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);

        let mut writer = AgentWriter::new(
            dir.path().join("test.log"),
            AgentType::Implementer { index: 1 },
            tx,
            0,
        ).await.unwrap();

        writer.write_header("Test task").await.unwrap();
        writer.write_message_chunk("Hello world").await.unwrap();
        writer.finalize(true).await.unwrap();

        let content = std::fs::read_to_string(dir.path().join("test.log")).unwrap();
        assert!(content.contains("IMPLEMENTER-001"));
        assert!(content.contains("Test task"));
        assert!(content.contains("Hello world"));
        assert!(content.contains("SUCCESS"));
    }

    #[tokio::test]
    async fn test_concurrent_implementers() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);
        let scope = LogScope::new(dir.path().to_path_buf(), tx, 0);

        let w1 = scope.implementer_writer().await.unwrap();
        let w2 = scope.implementer_writer().await.unwrap();

        assert!(dir.path().join("implementer-001.log").exists());
        assert!(dir.path().join("implementer-002.log").exists());
    }
}
```

### Integration Tests

```rust
// tests/logging_integration.rs
#[tokio::test]
async fn test_always_planner_first_structure() {
    let log_manager = RunLogManager::new("target/test-logs").unwrap();
    let root = log_manager.root_scope();

    // Root level: planner → orchestrator (always present)
    let _root_planner = root.planner_writer().await.unwrap();
    let _root_orch = root.orchestrator_writer().await.unwrap();
    let _impl1 = root.implementer_writer().await.unwrap();

    // Verify root structure
    let run_dir = log_manager.run_dir();
    assert!(run_dir.join("planner.log").exists());
    assert!(run_dir.join("orchestrator.log").exists());
    assert!(run_dir.join("implementer-001.log").exists());
}

#[tokio::test]
async fn test_nested_decompose_creates_subtask_hierarchy() {
    let log_manager = RunLogManager::new("target/test-logs").unwrap();
    let root = log_manager.root_scope();

    // Root level
    let _root_planner = root.planner_writer().await.unwrap();
    let _root_orch = root.orchestrator_writer().await.unwrap();

    // First subtask (decompose call)
    let subtask1 = root.child_scope().await.unwrap();
    let _sub1_planner = subtask1.planner_writer().await.unwrap();
    let _sub1_orch = subtask1.orchestrator_writer().await.unwrap();
    let _sub1_impl = subtask1.implementer_writer().await.unwrap();

    // Nested subtask within subtask1
    let nested = subtask1.child_scope().await.unwrap();
    let _nested_planner = nested.planner_writer().await.unwrap();
    let _nested_impl = nested.implementer_writer().await.unwrap();

    // Verify structure
    let run_dir = log_manager.run_dir();
    assert!(run_dir.join("planner.log").exists());
    assert!(run_dir.join("orchestrator.log").exists());
    assert!(run_dir.join("subtask-001/planner.log").exists());
    assert!(run_dir.join("subtask-001/orchestrator.log").exists());
    assert!(run_dir.join("subtask-001/implementer-001.log").exists());
    assert!(run_dir.join("subtask-001/subtask-001/planner.log").exists());
    assert!(run_dir.join("subtask-001/subtask-001/implementer-001.log").exists());
}
```

### Manual Testing Steps

1. Run `cargo run "Simple task"` and verify:
   - New timestamped folder created in `logs/`
   - `planner.log` contains planning conversation (always present)
   - `orchestrator.log` contains execution conversation
   - `implementer-001.log` contains implementation

2. Run `cargo run "Complex task that needs decomposition"` and verify:
   - Root has `planner.log` and `orchestrator.log`
   - Subfolder `subtask-001/` created for decomposed work
   - Subtask folder has its own `planner.log`, `orchestrator.log`, `implementer-NNN.log`

3. Run task with nested decomposition and verify:
   - `subtask-001/subtask-001/` hierarchy is created
   - Each level has complete planner → orchestrator → implementer logs

4. Run task with multiple implementers and verify separate log files

---

## 9. Rollback Plan

### If Issues Arise

1. **Feature flag approach**: Keep old logging path active
   ```rust
   let use_nested_logging = std::env::var("VILLALOBOS_NESTED_LOGS").is_ok();
   ```

2. **Revert steps**:
   - Remove `mod logging;` from `main.rs`
   - Remove logging fields from `App` struct
   - Restore original `handle_acp_message()` signature
   - Remove `chrono` from `Cargo.toml`

3. **Data preservation**: Old log format (`logs/villalobos.log.DATE`) remains available

---

## 10. Future UI Streaming Integration

The design supports future UI streaming via the broadcast channel:

```rust
// Example: UI server subscribing to log events
async fn start_ui_server(log_manager: Arc<RunLogManager>) {
    let mut receiver = log_manager.subscribe();

    loop {
        match receiver.recv().await {
            Ok(event) => {
                // Send to WebSocket clients, TUI, etc.
                match event {
                    LogEvent::AgentMessage { agent_type, content, depth, .. } => {
                        // Stream to UI with hierarchy info
                    }
                    LogEvent::ScopeCreated { new_depth, path, .. } => {
                        // Update UI tree structure
                    }
                    _ => {}
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("UI receiver lagged by {} events", n);
            }
            Err(_) => break,
        }
    }
}
```

---

## 11. Estimated Effort

| Step | Time Estimate | Complexity |
|------|---------------|------------|
| Steps 1-4 (Core types) | 2-3 hours | Medium |
| Steps 5-8 (Integration) | 3-4 hours | High |
| Step 9 (Migration) | 1 hour | Low |
| Step 10 (Streaming API) | 30 min | Low |
| Testing | 2 hours | Medium |
| **Total** | **8-10 hours** | **Medium-High** |

---

## 12. Open Questions / Decisions

1. **Log rotation within runs?** - Current design keeps all logs for a run in one folder. For very long runs, may want to rotate `orchestrator.log` files.

2. **Session-to-writer mapping**: Currently passing writers through function calls. Alternative: use a `HashMap<SessionId, AgentWriter>` for cleaner routing.

3. **Flush frequency**: Currently flush after each write. Consider batching for performance if needed.

4. **Log format**: Currently human-readable text. Could add JSON format option for machine parsing.

