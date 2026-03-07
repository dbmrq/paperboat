# TUI Plan

## Overview

Add a Terminal User Interface (TUI) to villalobos for real-time visibility into agent execution, task progress, and application logs.

## Requirements

1. **Agent Output Visibility**: Stream agent output (thinking, tool calls, results) in real-time
2. **Agent Hierarchy Navigation**: Display nested agent structure (orchestrators can spawn child orchestrators via decompose)
3. **Task Progress**: Show all tasks with status (NotStarted, InProgress, Complete, Failed)
4. **App Logs**: Display tracing logs with filtering capabilities
5. **Non-blocking**: TUI must not block agent execution
6. **Graceful Degradation**: Support `--tui` flag; default to current console output

## Layout: Master-Detail with Split Panes

```
┌─────────────────────┬─────────────────────────────────┬───────────┐
│  Agent Tree         │  Selected Agent Output           │   Tasks   │
│  (navigation)       │  (streaming content)             │   List    │
│                     │                                  │           │
│  ▼ Orch [00:05:32]  │  🔧 view src/auth.rs             │ [ ] task1 │
│    ├ Planner ✓      │  Looking at the auth module...   │ [/] task2 │
│   >├ impl-001 🔧    │  I see we need to add JWT...     │ [✓] task3 │
│    ├ impl-002       │  🔧 str-replace-editor auth.rs   │ [✗] task4 │
│    └ ▼ Orch [sub]   │  Adding authenticate() function  │           │
│        ├ impl-001   │  ...                             │           │
│        └ impl-002   │                                  │           │
├─────────────────────┴─────────────────────────────────┴───────────┤
│  App Logs (filterable by target/level)                            │
├───────────────────────────────────────────────────────────────────┤
│  Status: Running │ Agents: 4 │ Tasks: 2/4 │ ?=help                │
└───────────────────────────────────────────────────────────────────┘
```

### Panel Descriptions

| Panel | Purpose | Proportions |
|-------|---------|-------------|
| Agent Tree | Navigate agent hierarchy, select agent to view | ~20% width |
| Agent Output | Streaming output for selected agent | ~50% width |
| Task List | Task status overview | ~30% width |
| App Logs | Tracing output with filtering | ~30% height |
| Status Bar | Summary info, help hint | 1 line |

## Key Decisions

### 1. Use tui-rs-tree-widget for Agent Tree

**Decision**: Use `tui-tree-widget` crate (not `ratatui-explorer`)

**Rationale**:
- Generic tree widget for any hierarchical data
- Built-in expand/collapse and selection state
- `ratatui-explorer` is filesystem-specific, not suitable

### 2. Use tui-logger for App Logs

**Decision**: Use `tui-logger` with `tracing-support` feature

**Rationale**:
- Provides `TuiTracingSubscriberLayer` for seamless tracing integration
- Built-in target/level filtering with keyboard controls
- Scrollback and circular buffer handling included
- No need to build custom log widget

### 3. Separate Data Flows

**Decision**: Two independent data pipelines

| Data | Source | Destination |
|------|--------|-------------|
| Agent events | `broadcast::Sender<LogEvent>` | Custom `AgentTreeState` |
| App logs | `tracing!` macros | tui-logger internal buffer |

**Rationale**: Clean separation of concerns; each pipeline optimized for its use case

### 4. Threading Model

**Decision**: TUI runs on separate `std::thread`, not tokio task

**Rationale**:
- TUI event loop is blocking (crossterm polling)
- Avoids blocking tokio runtime
- Communication via `std::sync::mpsc` channel forwarding `LogEvent`s

### 5. Auto-Follow Active Agent

**Decision**: Default to following most recently active agent; allow manual lock

**Rationale**: Users typically want to see current activity, but need ability to inspect specific agents

## Data Sources (Existing)

The following already exist and will be consumed by the TUI:

- `LogEvent::AgentStarted` - Agent spawned
- `LogEvent::AgentMessage` - Streaming text chunks
- `LogEvent::ToolCall` - Tool invocation
- `LogEvent::ToolProgress` - Tool streaming output
- `LogEvent::ToolResult` - Tool completion
- `LogEvent::AgentComplete` - Agent finished
- `LogEvent::SubtaskCreated` - New decomposition scope
- `LogEvent::TaskCreated` - Task added to plan
- `LogEvent::TaskStateChanged` - Task status update

## Dependencies

```toml
ratatui = "0.29"
crossterm = "0.28"
tui-tree-widget = "0.24"
tui-logger = { version = "0.13", features = ["tracing-support"] }
```

## Keyboard Navigation

### Global
| Key | Action |
|-----|--------|
| `Tab` | Cycle focus between panels |
| `q` | Quit |
| `?` | Toggle help overlay |

### Agent Tree (focused)
| Key | Action |
|-----|--------|
| `↑/↓` | Navigate agents |
| `←/→` | Collapse/expand |
| `Enter` | Select agent for detail view |
| `f` | Toggle auto-follow |

### App Logs (focused)
Inherits tui-logger controls: `h` toggle selector, `←/→` filter level, `PgUp/PgDn` scroll

## Implementation Phases

| Phase | Scope | Estimate |
|-------|-------|----------|
| 1. Foundation | Terminal setup, layout, quit handling | 2-3 days |
| 2. Agent Tree | Tree widget, LogEvent processing, selection | 2-3 days |
| 3. Agent Output | Detail pane, message streaming, scrollback | 1-2 days |
| 4. Task List | Status display, real-time updates | 1 day |
| 5. App Logs | tui-logger integration, tracing layer | 1-2 days |
| 6. Polish | Status bar, help, `--tui` flag, testing | 1-2 days |

**Total: 8-13 days**

## Module Structure

```
src/tui/
├── mod.rs          # Public API, feature flag
├── app.rs          # Event loop, terminal management
├── state.rs        # TuiState, AgentTreeState, focus management
├── layout.rs       # Panel layout calculations
├── widgets/
│   ├── mod.rs
│   ├── agent_output.rs
│   ├── task_list.rs
│   └── status_bar.rs
└── events.rs       # Keyboard event handling
```

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| TUI blocks async runtime | Separate thread with channel communication |
| Too many events overwhelm TUI | Coalesce rapid message chunks; ring buffers |
| Terminal not restored on crash | Panic hook to restore terminal state |
| Conflicts with existing console output | `--tui` flag makes TUI opt-in |

## Out of Scope

- Web UI
- Log persistence beyond existing file logging
- Agent control (pause/resume/cancel) via TUI
- Configuration via TUI

