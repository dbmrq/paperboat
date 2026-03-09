# Backend Transport Architecture

This document describes Paperboat's backend transport architecture, which separates vendor configuration (backends) from communication protocols (transports).

## Status

| Component | Status | Notes |
|-----------|--------|-------|
| Backend × Transport design | ✅ Complete | See `src/backend/mod.rs` |
| `TransportKind` enum | ✅ Complete | ACP, CLI variants |
| `BackendConfig` parsing | ✅ Complete | `cursor:cli` syntax supported |
| CLI argument parsing | ✅ Complete | `--backend cursor:cli` works |
| Config file parsing | ✅ Complete | `backend = "cursor:cli"` works |
| Environment variable | ✅ Complete | `PAPERBOAT_BACKEND=cursor:cli` works |
| `AgentTransport` trait | ✅ Complete | See `src/backend/transport.rs` |
| `CursorCliTransport` | ✅ Complete | See `src/backend/cursor/cli.rs` |
| App migration | 🔄 In Progress | Use `AgentTransport` instead of `AcpClientTrait` |

## Context

Paperboat is a multi-agent orchestration system that can use different AI backends (Auggie, Cursor). We support multiple transport protocols per backend because:

1. **Cursor's ACP mode doesn't support MCP tools** (known Cursor bug with no ETA for fix)
2. Cursor's CLI mode (`agent --print`) DOES properly load MCP servers from `~/.cursor/mcp.json`

## Configuration

### Backend:Transport Syntax

```bash
# CLI flag
--backend cursor        # Cursor with CLI (default)
--backend cursor:cli    # Cursor with CLI (explicit)
--backend cursor:acp    # Cursor with ACP (MCP broken)
--backend auggie        # Auggie with ACP (default)
--backend auggie:acp    # Auggie with ACP (explicit)

# Environment variable
PAPERBOAT_BACKEND=cursor:cli cargo run -- "task"

# Config file (.paperboat/config.toml)
backend = "cursor:cli"
```

### Default Transports

| Backend | Default Transport | Reason |
|---------|-------------------|--------|
| Auggie  | ACP              | Only supported transport |
| Cursor  | CLI              | Better MCP tool support |

## Current Architecture (to understand, not preserve exactly)

```
src/backend/
├── trait.rs          # Backend trait: auth, models, cache, create_client
├── auggie/
│   └── mod.rs        # Auggie backend implementation
└── cursor/
    ├── mod.rs        # Cursor backend implementation
    ├── acp.rs        # CursorAcpClient (ACP mode - MCP broken)
    ├── mcp_config.rs # Write to ~/.cursor/mcp.json
    └── cache.rs      # Cache directory setup

src/acp.rs            # AcpClientTrait + AcpClient (Auggie ACP implementation)
```

Key types:
- `Backend` trait: Vendor-specific setup (auth, models, cache)
- `AcpClientTrait`: Communication interface (initialize, session_new, session_prompt, etc.)
- `AcpClient`: Auggie's ACP implementation
- `CursorAcpClient`: Cursor's ACP implementation (MCP broken)

## Proposed Architecture

### 1. Separate Concerns: Backend vs Transport vs Client

```
Backend (vendor)     × Transport (protocol)  = Client (implementation)
─────────────────────────────────────────────────────────────────────
Auggie               × ACP                   = AuggieAcpClient ✓
Cursor               × ACP                   = CursorAcpClient (MCP broken)
Cursor               × CLI                   = CursorCliClient (NEW - MCP works)
```

### 2. New Directory Structure

```
src/backend/
├── mod.rs            # Re-exports, BackendKind enum
├── trait.rs          # Backend trait (vendor config)
├── transport.rs      # Transport trait (communication)
├── auggie/
│   ├── mod.rs        # AuggieBackend
│   └── acp.rs        # AuggieAcpTransport
└── cursor/
    ├── mod.rs        # CursorBackend
    ├── acp.rs        # CursorAcpTransport (keep for when Cursor fixes MCP)
    ├── cli.rs        # CursorCliTransport (NEW)
    ├── mcp_config.rs # MCP config management
    └── permission.rs # Permission policy (extracted from acp.rs)
```

### 3. Core Traits

```rust
/// Backend: Vendor-specific configuration and capabilities
pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn check_auth(&self) -> Result<()>;
    fn discover_models(&self) -> impl Future<Output = Result<ModelTiers>>;
    fn setup_mcp(&self, socket_path: &str) -> impl Future<Output = Result<()>>;
    fn cleanup_mcp(&self) -> Result<()>;

    /// Which transport modes this backend supports
    fn supported_transports(&self) -> Vec<TransportKind>;

    /// Create a transport client for the given mode and agent type
    fn create_transport(
        &self,
        kind: TransportKind,
        agent_type: AgentType,
        config: TransportConfig,
    ) -> impl Future<Output = Result<Box<dyn AgentTransport>>>;
}

/// Transport kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// ACP protocol (JSON-RPC over stdin/stdout)
    Acp,
    /// Non-interactive CLI (`agent --print`)
    Cli,
}

/// Agent types for permission control
#[derive(Debug, Clone, Copy)]
pub enum AgentType {
    Orchestrator,  // No file editing, has spawn_agents/decompose
    Planner,       // No file editing, has set_goal/create_task
    Implementer,   // Full access, has complete
}

/// Transport: Communication with an agent process
#[async_trait]
pub trait AgentTransport: Send + Sync {
    /// Initialize the connection
    async fn initialize(&mut self) -> Result<()>;

    /// Create a new session
    async fn create_session(&mut self, config: SessionConfig) -> Result<SessionInfo>;

    /// Send a prompt and receive responses
    async fn send_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()>;

    /// Take the notification receiver (for streaming responses)
    fn take_notifications(&mut self) -> Option<mpsc::Receiver<SessionUpdate>>;

    /// Respond to a tool call (for transports that support it)
    async fn respond_to_tool(
        &mut self,
        session_id: &str,
        tool_use_id: &str,
        result: ToolResult,
    ) -> Result<()>;

    /// Shutdown the transport
    async fn shutdown(&mut self) -> Result<()>;
}
```

### 4. CursorCliTransport Implementation

The new CLI transport for Cursor:

```rust
/// Cursor CLI transport using `agent --print` mode
pub struct CursorCliTransport {
    workspace: PathBuf,
    model: String,
    permission_policy: PermissionPolicy,
    current_session_id: Option<String>,
    notification_tx: mpsc::Sender<SessionUpdate>,
    notification_rx: Option<mpsc::Receiver<SessionUpdate>>,
}

impl CursorCliTransport {
    /// Spawn `agent --print` with the given prompt
    async fn run_agent(
        &mut self,
        prompt: &str,
        resume_session: Option<&str>,
    ) -> Result<AgentOutput> {
        let mut cmd = Command::new("agent");
        cmd.args(["--print", "--force", "--approve-mcps", "--trust"]);
        cmd.args(["--output-format", "stream-json"]);
        cmd.args(["--model", &self.model]);
        cmd.arg("--workspace").arg(&self.workspace);

        if let Some(session_id) = resume_session {
            cmd.arg("--resume").arg(session_id);
        }

        cmd.arg("--").arg(prompt);

        // Spawn and stream output...
    }
}
```

Key CLI flags:
- `--print` - Non-interactive mode
- `--force` - Allow all tools (including MCP)
- `--approve-mcps` - Auto-approve MCP servers
- `--trust` - Trust workspace without prompting
- `--output-format stream-json` - Structured streaming output
- `--resume <session_id>` - Continue previous session

### 5. Configuration

Add transport configuration to support easy switching:

```rust
/// Configuration for backend and transport selection
pub struct BackendConfig {
    pub kind: BackendKind,
    pub transport: TransportKind,
    // ... other config
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: BackendKind::Auggie,
            transport: TransportKind::Acp,
        }
    }
}

// CLI flag: --backend cursor:cli or --backend cursor:acp or --backend auggie
```

## Files to Modify/Create

**Create:**
- `src/backend/transport.rs` - `AgentTransport` trait, `TransportKind`, `SessionUpdate` types
- `src/backend/cursor/cli.rs` - `CursorCliTransport` implementation
- `src/backend/cursor/permission.rs` - Extract `PermissionPolicy` from `acp.rs`

**Modify:**
- `src/backend/trait.rs` - Add `supported_transports()`, `create_transport()` to `Backend` trait
- `src/backend/mod.rs` - Re-export new types, update `BackendKind`
- `src/backend/auggie/mod.rs` - Implement new trait methods
- `src/backend/cursor/mod.rs` - Implement new trait methods, support both ACP and CLI
- `src/app/mod.rs` - Use new `AgentTransport` trait instead of `AcpClientTrait`
- `src/main.rs` / CLI parsing - Support `--backend cursor:cli` syntax

**Preserve (rename/adapt):**
- `src/acp.rs` → Keep as `AuggieAcpTransport` or move to `src/backend/auggie/acp.rs`
- `src/backend/cursor/acp.rs` → Keep for when Cursor fixes MCP support

## CLI Output Format

The `agent --print --output-format stream-json` outputs newline-delimited JSON:

```json
{"type":"text","content":"I'll help you..."}
{"type":"tool_use","id":"call_123","name":"paperboat-create_task","input":{...}}
{"type":"tool_result","tool_use_id":"call_123","content":"Task created"}
{"type":"result","subtype":"success","result":"...","session_id":"abc-123",...}
```

Parse these to emit `SessionUpdate` events that match what the ACP transport produces.

## Key Design Principles

1. **Trait-based polymorphism** - Use traits (`Backend`, `AgentTransport`) so implementations are swappable
2. **Composition over inheritance** - Backend + Transport = Client behavior
3. **Single responsibility** - Backend handles vendor stuff, Transport handles communication
4. **Feature flags for optional transports** - Could add `#[cfg(feature = "cursor-cli")]` if needed
5. **Backwards compatibility** - Existing tests using `AcpClientTrait` should still work
6. **Unified session updates** - Both ACP and CLI transports emit the same `SessionUpdate` type

## Testing Strategy

1. **Unit tests** for `CursorCliTransport` output parsing
2. **Integration tests** using mock `agent` command (shell script that emits expected JSON)
3. **Keep existing tests** - They use mock transports, should continue to work

## Migration Path

1. Add new traits and `CursorCliTransport` alongside existing code
2. Update `CursorBackend` to support both transports
3. Make CLI the default for Cursor (since ACP MCP is broken)
4. Gradually migrate App to use `AgentTransport` instead of `AcpClientTrait`
5. Eventually rename/consolidate the traits

## Success Criteria

1. `./paperboat --backend cursor "Hello"` works with MCP tools (uses CLI transport)
2. `./paperboat --backend cursor:acp "Hello"` uses ACP (for future when Cursor fixes MCP)
3. `./paperboat --backend auggie "Hello"` continues to work (ACP)
4. Adding a new backend (e.g., `claude-code`) requires:
   - One new directory under `src/backend/`
   - Implementing `Backend` trait
   - Implementing one or more `AgentTransport` variants
   - No changes to `src/app/` code
