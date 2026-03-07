# Villalobos

An agentic AI orchestration framework for autonomous task execution.

## Features

- **Multi-Agent Architecture**: Orchestrator, Planner, Implementer, and Verifier agents
- **Task Management**: Hierarchical task decomposition and dependency tracking
- **Real-time TUI**: Interactive terminal interface for monitoring agent activity
- **Comprehensive Logging**: Detailed logs for debugging and analysis

## Quick Start

```bash
# Build the project
cargo build --release

# Run with console output (default)
cargo run --release "your task description"

# Run with TUI mode
cargo run --release --features tui -- --tui "your task description"
```

## Terminal User Interface (TUI)

Villalobos includes an optional Terminal User Interface for real-time monitoring of agent activity.

### Enabling TUI Mode

The TUI requires building with the `tui` feature flag:

```bash
# Build with TUI support
cargo build --release --features tui

# Run with TUI enabled
cargo run --release --features tui -- --tui "your task description"
```

**Note**: The `--tui` flag requires an interactive terminal. It cannot be used with:
- Piped input/output
- Non-interactive environments
- `--mcp-server` mode (mutually exclusive)

### Layout Overview

```
┌─────────────┬────────────────────────┬───────────────┐
│ Agent Tree  │      Agent Output      │   Task List   │
│    (20%)    │         (50%)          │     (30%)     │
├─────────────┴────────────────────────┴───────────────┤
│                    App Logs                          │
│                     (30%)                            │
├──────────────────────────────────────────────────────┤
│ Status Bar                            Press ? for help│
└──────────────────────────────────────────────────────┘
```

**Panels:**
- **Agent Tree**: Navigate the agent hierarchy (orchestrator → child agents)
- **Agent Output**: Real-time streaming output from the selected agent
- **Task List**: Task status overview with progress indicators
- **App Logs**: Filterable application logs (tracing output)
- **Status Bar**: Agent count, task progress, and help hints

### Keyboard Shortcuts

Press `?` at any time to see the help overlay with all shortcuts.

#### Global Shortcuts

| Key | Action |
|-----|--------|
| `Tab` | Cycle focus between panels |
| `Shift+Tab` | Cycle focus in reverse |
| `q` | Quit TUI |
| `?` | Toggle help overlay |
| `Esc` | Close help overlay |

#### Agent Tree Panel

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate agents |
| `←` / `→` | Collapse/expand tree nodes |
| `Enter` | Select agent for detail view |
| `f` | Toggle auto-follow mode |

#### Agent Output Panel

| Key | Action |
|-----|--------|
| `PgUp` / `PgDn` | Scroll output by page |
| `Home` / `End` or `g` / `G` | Jump to top/bottom |
| `↑` / `↓` or `k` / `j` | Scroll by single line |

#### Task List Panel

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate tasks |
| `PgUp` / `PgDn` | Scroll list by page |

#### App Logs Panel

| Key | Action |
|-----|--------|
| `h` | Toggle target selector (show/hide log targets) |
| `←` / `→` | Filter by log level (decrease/increase minimum level) |
| `PgUp` / `PgDn` | Scroll logs |
| `↑` / `↓` | Navigate targets (when target selector visible) |
| `Space` | Toggle focus between target list and log view |

### Features

#### Auto-Follow Mode
When enabled (default), the TUI automatically selects newly spawned agents, keeping focus on the most recent activity. Press `f` in the Agent Tree panel to toggle this behavior.

#### Real-Time Streaming
Agent output is streamed in real-time, showing:
- Agent thinking and reasoning
- Tool calls with icons (🔧 calling, ✅ success, ❌ error)
- Subtask creation notifications
- Completion status

#### Log Filtering
The App Logs panel supports filtering by:
- Log level (trace, debug, info, warn, error)
- Log target (module/crate)

## Troubleshooting

### TUI Issues

**TUI won't start:**
```
Error: --tui requires an interactive terminal
```
- Ensure you're running in a real terminal, not piped
- Check that stdout is connected to a TTY

**Display issues / garbled output:**
- Try resizing your terminal
- Ensure terminal supports UTF-8 and 256 colors
- Minimum recommended size: 120×40 characters

**TUI crashes / terminal left in bad state:**
The TUI installs a panic hook to restore terminal state, but if it fails:
```bash
# Reset terminal
reset
# Or
stty sane
```

**Disabling TUI:**
Simply omit the `--tui` flag to use console output mode (default).

### Terminal Compatibility

Tested terminals:
- ✅ macOS Terminal.app
- ✅ iTerm2
- ✅ Alacritty
- ✅ Kitty
- ✅ VS Code integrated terminal
- ✅ Linux: GNOME Terminal, Konsole, xterm

## Architecture Notes

### Threading Model

The TUI runs on a dedicated OS thread separate from the async Tokio runtime:

```
┌─────────────────────────────────────────────────────────────┐
│                    Tokio Runtime                            │
│  ┌─────────────┐    ┌─────────────────────────────────────┐ │
│  │   Agents    │───>│  Broadcast Channel (LogEvent)       │ │
│  └─────────────┘    └──────────────┬──────────────────────┘ │
│                                    │                        │
│  ┌──────────────────────────────┐  │                        │
│  │     Event Bridge (async)     │<─┘                        │
│  │   broadcast → mpsc adapter   │                           │
│  └─────────────┬────────────────┘                           │
└────────────────│────────────────────────────────────────────┘
                 │ mpsc::sync_channel
                 ▼
┌────────────────────────────────────────────────────────────┐
│              TUI Thread (std::thread)                      │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Main Event Loop (60 FPS)               │   │
│  │  - Poll terminal events (keyboard, resize)          │   │
│  │  - Receive LogEvents from mpsc channel              │   │
│  │  - Update TuiState                                  │   │
│  │  - Render UI via ratatui                            │   │
│  └─────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────┘
```

### LogEvent Flow

1. **Generation**: Agents emit `LogEvent`s via `LogScope` during execution
2. **Broadcasting**: Events are published to a `tokio::sync::broadcast` channel
3. **Bridging**: An async task forwards events to a bounded `std::sync::mpsc` channel
4. **Consumption**: TUI thread polls the mpsc channel during each frame

### Performance Characteristics

- **Frame rate**: 60 FPS maximum (16ms per frame)
- **Event buffer**: 1000 events (broadcast) + 1000 events (mpsc)
- **Backpressure**: If TUI can't keep up, oldest events are dropped with a warning
- **Memory**: State is bounded; message buffers have configurable limits

## Command-Line Options

| Flag | Description |
|------|-------------|
| `--tui` | Enable Terminal User Interface (requires `tui` feature) |
| `--mcp-server` | Run as MCP server (mutually exclusive with `--tui`) |

## Logs

Logs are stored in the `logs/` directory with timestamped run folders:
```
logs/
└── 2026-03-07_14-30-45_abc123/
    ├── app.log           # Application logs
    ├── orchestrator.log  # Root orchestrator
    ├── planner.log       # Planner agent
    └── subtask-001/      # Child scope logs
        └── ...
```

## License

MIT

