# TestHarness Tool Call Integration Analysis

## Executive Summary

This document analyzes the TestHarness tool call integration system in Villalobos, examining how tool calls flow through the system, identifying current implementation gaps, and recommending approaches for completing the integration.

---

## 1. Architecture Overview

### 1.1 System Components

| Component | Location | Status | Purpose |
|-----------|----------|--------|---------|
| `MockScenario` | `src/testing/scenario.rs` | ✅ Complete | Loads test scenarios from TOML files |
| `MockAcpClient` | `src/testing/mock_acp.rs` | ✅ Complete | Simulates ACP communication |
| `MockToolInterceptor` | `src/testing/harness.rs` | ✅ Complete | Captures and responds to tool calls |
| `TestHarness` | `src/testing/harness.rs` | ⚠️ Partial | Orchestrates test runs |
| `AcpClientTrait` | `src/acp.rs` | ✅ Complete | Trait abstraction for DI |
| `App::with_mock_clients` | `src/app.rs` | ✅ Complete | Enables mock injection |
| Builder Helpers | `src/testing/builders.rs` | ✅ Complete | Fluent APIs for test data |
| Mock Types | `src/testing/types.rs` | ✅ Complete | Core data structures |

### 1.2 Tool Call Types

The system supports four tool call types:

1. **`Decompose`** - Break a complex task into subtasks
2. **`Implement`** - Execute a specific task via an implementer agent
3. **`Complete`** - Signal agent completion
4. **`WritePlan`** - Store a structured plan (planner only)

---

## 2. Tool Call Flow Analysis

### 2.1 Production Flow (Real Execution)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            PRODUCTION FLOW                               │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌─────────┐    ┌─────────────┐    ┌───────────────┐    ┌────────────┐ │
│  │  Agent  │───>│ MCP Server  │───>│ Unix Socket   │───>│    App     │ │
│  │(auggie) │    │(villalobos) │    │ (real IPC)    │    │            │ │
│  └─────────┘    └─────────────┘    └───────────────┘    └────────────┘ │
│       │               │                    │                   │        │
│       │    tools/call │                    │                   │        │
│       │<──────────────│                    │                   │        │
│       │               │─────ToolRequest───>│                   │        │
│       │               │                    │───ToolMessage────>│        │
│       │               │                    │                   │        │
│       │               │                    │  (App processes)  │        │
│       │               │                    │                   │        │
│       │               │<───ToolResponse────│<───────────────────│        │
│       │<──────────────│                    │                   │        │
│       │    response   │                    │                   │        │
└─────────────────────────────────────────────────────────────────────────┘
```

**Key Files:**
- `src/mcp_server/handlers.rs` - Handles MCP protocol and tool dispatch
- `src/mcp_server/socket.rs` - Unix socket communication
- `src/app.rs:setup_socket()` - Sets up socket listener (lines 346-384)
- `src/app.rs:run_orchestrator_with_writer_impl()` - Main tool handling loop

### 2.2 Test Flow (Mock Execution) - CURRENT STATE

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              TEST FLOW                                   │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────┐                                       ┌────────────┐  │
│  │ MockAcpClient│──────(no real agent)─────────────────>│    App     │  │
│  └──────────────┘                                       └────────────┘  │
│         │                                                     │         │
│         │ Scripted session updates                            │         │
│         ▼                                                     │         │
│  ┌──────────────────────────────────────────────────────────┐ │         │
│  │                    ⚠️ GAP IDENTIFIED                      │ │         │
│  │  Tool calls via Unix socket are NOT intercepted          │ │         │
│  │  MockToolInterceptor exists but is not connected         │ │         │
│  └──────────────────────────────────────────────────────────┘ │         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Critical Implementation Gaps

### 3.1 Gap 1: Tool Calls Not Intercepted in Mock Mode

**Problem:** The `MockToolInterceptor` is created but never used. `App::run()` still tries to set up a **real Unix socket** even in mock mode. Since no MCP server process is spawned, **no tool calls ever arrive**.

**Evidence:** `src/app.rs:220` calls `setup_socket()` unconditionally.

### 3.2 Gap 2: Mock ACP Client Cannot Trigger Tool Calls

**Problem:** `MockAcpClient` returns scripted session updates but has no mechanism to simulate an agent calling an MCP tool or inject tool call requests into the `tool_rx` channel.

### 3.3 Gap 3: Agent Type Detection Heuristic is Fragile

**Problem:** `MockAcpClient::agent_type_from_model()` uses string matching on model names ("planner", "orchestrat"), which breaks with actual model names like "claude-3.5-sonnet".

### 3.4 Gap 4: TestRunResult Tool Calls are Empty

**Problem:** `harness.rs` collects `interceptor.captured_calls()` but since the interceptor is never connected, this is always empty.

### 3.5 Gap 5: Missing Orchestrator Session in Scenarios

**Problem:** `simple_implement.toml` defines planner and implementer sessions but NO orchestrator session. The actual flow requires: Planner → **Orchestrator** → Implementer.

---

## 4. Recommended Fix Approach

### Option A: Bypass Socket, Inject Tool Calls Directly (RECOMMENDED)

Allow `App` to receive tool calls via a different mechanism when in mock mode.

**Implementation Steps:**

1. Add optional `MockToolInterceptor` to `App`:
   ```rust
   pub struct App {
       // ... existing fields
       mock_tool_interceptor: Option<Arc<Mutex<MockToolInterceptor>>>,
   }
   ```

2. Modify `with_mock_clients()` to accept interceptor
3. Skip real socket setup when interceptor is present
4. Create in-memory channel for tool injection

**Pros:** Minimal changes, clear separation of concerns
**Cons:** Adds conditional logic to App

### Option B: Mock Socket Communication

Replace Unix socket with in-memory channels for testing. More complex but more realistic.

### Option C: Simulate via ACP Updates Only (Simplest)

Don't simulate MCP tool calls. Use `agent_turn_finished` to trigger completion. Lowest coverage but simplest to implement.

---

## 5. Implementation Plan

### Phase 1: Fix Agent Type Detection (1-2 hours)
- Pass agent type explicitly when creating mock sessions
- Track session creation order (planner → orchestrator → implementers)
- Remove model name heuristic

### Phase 2: Implement Tool Call Injection (4-6 hours)
- Create `MockToolInjector` in `src/testing/`
- Modify `App::with_mock_clients()` to accept `MockToolInterceptor`
- Create alternative path in orchestrator loop for mock mode

### Phase 3: Update Test Scenarios (2-3 hours)
- Add `orchestrator_sessions` to `simple_implement.toml`
- Add scenario-level tool call triggers
- Create `nested_decompose.toml` and `error_recovery.toml`

### Phase 4: Integration Test Implementation (2-3 hours)
- Create `tests/integration/` directory
- Add `harness_basic_test.rs`
- Add `orchestration_flow_test.rs`

---

## 6. Required Code Changes

### 6.1 App Modifications (`src/app.rs`)

```rust
// Add to struct
mock_tool_interceptor: Option<Arc<Mutex<MockToolInterceptor>>>,

// Modify with_mock_clients
pub fn with_mock_clients(
    orchestrator: Box<dyn AcpClientTrait + Send>,
    worker: Box<dyn AcpClientTrait + Send>,
    model_config: ModelConfig,
    log_manager: Arc<RunLogManager>,
    mock_tool_interceptor: Option<Arc<Mutex<MockToolInterceptor>>>,
) -> Self { ... }

// Skip socket setup in mock mode
async fn setup_socket(&mut self) -> Result<PathBuf> {
    if self.mock_tool_interceptor.is_some() {
        // Use in-memory channel instead
        let (tool_tx, tool_rx) = mpsc::channel::<ToolMessage>(100);
        self.tool_rx = Some(tool_rx);
        return Ok(PathBuf::from("/mock/socket"));
    }
    // ... existing socket setup
}
```

### 6.2 MockAcpClient Modifications (`src/testing/mock_acp.rs`)

```rust
// Add tool call triggering capability
pub fn queue_tool_call(&mut self, tool_call: ToolCall, request_id: String) {
    self.pending_tool_calls.push_back((tool_call, request_id));
}
```

### 6.3 Harness Modifications (`src/testing/harness.rs`)

```rust
// Connect interceptor to App
let mut app = App::with_mock_clients(
    Box::new(mock_orchestrator),
    Box::new(mock_worker),
    model_config,
    log_manager,
    Some(self.tool_interceptor.clone()), // Pass interceptor
);
```

---

## 7. Corrected Scenario Format

```toml
[scenario]
name = "simple_implement"
description = "Single task implemented directly via orchestrator"

# Planner session
[[planner_sessions]]
session_id = "planner-001"
[[planner_sessions.updates]]
session_update = "agent_message_chunk"
content = "Creating plan..."
[[planner_sessions.updates]]
session_update = "agent_turn_finished"

# Orchestrator session (REQUIRED - currently missing)
[[orchestrator_sessions]]
session_id = "orchestrator-001"
[[orchestrator_sessions.updates]]
session_update = "agent_message_chunk"
content = "Executing plan..."
[[orchestrator_sessions.updates]]
session_update = "agent_turn_finished"

# Implementer session
[[implementer_sessions]]
session_id = "implementer-001"
# ... updates

# Mock tool responses
[[mock_tool_responses]]
tool_type = "Implement"
[mock_tool_responses.response]
success = true
summary = "Completed"
```

---

## 8. Summary

| Aspect | Current Status |
|--------|----------------|
| Mock Types | ✅ Complete |
| Scenario Loading | ✅ Complete |
| MockAcpClient | ⚠️ Lacks tool triggering |
| MockToolInterceptor | ⚠️ Not connected |
| TestHarness | ⚠️ Incomplete |
| App Mock Support | ⚠️ Still uses real socket |
| Test Scenarios | ⚠️ Missing orchestrator |
| Integration Tests | ❌ None exist |

### Critical Path

1. Fix App mock mode - Skip real socket, use injected channels
2. Connect MockToolInterceptor - Pass to App, use in tool handling
3. Add tool call triggers - Let scenarios specify when tools are called
4. Complete scenarios - Add orchestrator sessions
5. Write integration tests - Validate full flows

### Estimated Effort: 10-14 hours total

