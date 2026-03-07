# Paperboat

[![CI](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml)
[![Security](https://github.com/dbmrq/paperboat/actions/workflows/security.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/dbmrq/paperboat/graph/badge.svg)](https://codecov.io/gh/dbmrq/paperboat)

"Any abyss can be sailed using tiny paper boats." — João Guimarães Rosa

1. AI agents can perform most small tasks
2. AI agents can be used to break large tasks into smaller tasks
3. AI agents can be used to spawn new AI agents

Paperboat uses these three concepts to execute any task.

## Features

- **Multi-Agent Architecture**: Orchestrator, Planner, Implementer, Verifier, and Explorer agents
- **Task Management**: Hierarchical task decomposition and dependency tracking
- **Real-time TUI**: Interactive terminal interface for monitoring agent activity (enabled by default)
- **Comprehensive Logging**: Detailed logs with optional JSON format for debugging and analysis
- **MCP Server**: Run as an MCP server for integration with other tools
- **Per-Agent Configuration**: Configure models and settings per agent type
- **Configuration Validation**: Validate config files with helpful error messages and typo suggestions
- **Observability**: Optional Prometheus metrics export for monitoring (via `metrics` feature)

## Quick Start

```bash
# Build the project (includes TUI by default)
cargo build --release

# Run with TUI (default when in interactive terminal)
cargo run --release -- "your task description"

# Run in headless mode (no TUI, console output only)
cargo run --release -- --headless "your task description"

# Build without TUI support
cargo build --release --no-default-features

# Build with metrics support
cargo build --release --features metrics
```

### Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `tui` | ✅ Yes | Terminal User Interface with real-time agent monitoring |
| `testing` | No | Test utilities and mock ACP client for integration testing |
| `metrics` | No | Prometheus metrics collection and HTTP exporter |

## Terminal User Interface (TUI)

Paperboat includes a Terminal User Interface that is **enabled by default** when running in an interactive terminal. The TUI provides real-time monitoring of agent activity.

### TUI Mode Behavior

When built with the `tui` feature (default):
- TUI **automatically enables** in interactive terminals
- Use `--headless` to disable TUI and use console output instead
- TUI is automatically disabled when output is piped or in non-interactive environments

```bash
# TUI mode (default in interactive terminal)
cargo run --release -- "your task description"

# Headless mode (console output only)
cargo run --release -- --headless "your task description"
```

**Note**: TUI mode cannot be used with:
- Piped input/output (TUI auto-disables)
- Non-interactive environments (TUI auto-disables)
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
| `s` | Open model settings |
| `Esc` | Close overlays (help/settings) |

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

#### Model Settings
Press `s` to open the model settings overlay and configure which AI models to use for each agent type:
- **Orchestrator**: Coordinates overall task execution
- **Planner**: Creates detailed implementation plans
- **Implementer**: Executes individual tasks

Navigate with arrow keys, press Enter to select a model, and changes are saved automatically. Settings are stored in TOML files under `.paperboat/agents/` in your project directory.

**Note**: Model changes apply to newly spawned agents only; currently running agents continue with their original model.

## Model Configuration

Paperboat supports per-agent model configuration through TOML files. Configuration is loaded from two locations, with project-level settings taking priority:

1. **User-level** (defaults): `~/.paperboat/agents/`
2. **Project-level** (overrides): `.paperboat/agents/`

Each agent type has its own configuration file:
- `orchestrator.toml`
- `planner.toml`
- `implementer.toml`

### Configuration File Format

```toml
# Orchestrator agent configuration
model = "opus"
```

### Available Model Aliases

You can use either the full model ID or a short alias:

| Alias | Model |
|-------|-------|
| `opus` | Claude Opus 4.5 |
| `sonnet` | Claude Sonnet 4.5 |
| `haiku` | Claude Haiku 4.5 |

### Configuration Priority

Settings are merged with this priority (highest to lowest):
1. Project-level config (`.paperboat/agents/*.toml`)
2. User-level config (`~/.paperboat/agents/*.toml`)
3. Built-in defaults

This allows you to set personal defaults in your home directory while allowing project-specific overrides.

## Troubleshooting

### TUI Issues

**TUI doesn't appear:**
The TUI is enabled by default in interactive terminals. If it doesn't appear:
- Ensure you're running in a real terminal, not piped
- Check that stdout is connected to a TTY
- Verify the binary was built with the `tui` feature (default)

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
Use `--headless` to disable TUI and use console output instead:
```bash
cargo run --release -- --headless "your task"
```

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
| `--headless` | Disable TUI, use console output (TUI is enabled by default in interactive terminals) |
| `--mcp-server` | Run as MCP server (mutually exclusive with TUI mode) |
| `--socket <path>` | Unix socket path for MCP server (with `--mcp-server`) |
| `--validate-config` | Validate configuration files and exit (checks model aliases, file syntax) |
| `--json-logs` | Enable JSON-formatted log output for machine parsing |
| `--metrics` | Enable metrics collection with Prometheus exporter (requires `metrics` feature) |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `PAPERBOAT_LOG_DIR` | Override log directory (default: `.paperboat/logs`) |
| `PAPERBOAT_SOCKET` | Default socket path for MCP server (fallback if `--socket` not provided) |
| `PAPERBOAT_MODEL` | Override model for all agents in debug builds |
| `PAPERBOAT_JSON_LOGS` | Set to `1` or `true` to enable JSON log format (alternative to `--json-logs`) |
| `PAPERBOAT_METRICS` | Set to `1` or `true` to enable metrics collection (alternative to `--metrics`) |

## Logs

Logs are stored in the `.paperboat/logs/` directory with timestamped run folders:
```
.paperboat/logs/
└── 2026-03-07_14-30-45_abc123/
    ├── app.log           # Application logs
    ├── orchestrator.log  # Root orchestrator
    ├── planner.log       # Planner agent
    └── subtask-001/      # Child scope logs
        └── ...
```

## Development

### Prerequisites

- Rust (stable, latest version recommended)
- Git

### Setup

```bash
# Clone the repository
git clone <repository-url>
cd paperboat

# Install git hooks (recommended)
./scripts/install-hooks.sh

# Build the project
cargo build
```

### Development Tools

This project uses several tools to maintain code quality:

#### Code Formatting & Linting

```bash
# Check formatting
cargo fmt -- --check

# Apply formatting
cargo fmt

# Run clippy lints
cargo clippy --all-features -- -D warnings
```

#### Security & Dependency Checking

```bash
# Install cargo-audit for security vulnerability scanning
cargo install cargo-audit

# Run security audit
cargo audit

# Install cargo-deny for license and dependency checks
cargo install cargo-deny

# Run all deny checks (licenses, bans, advisories, sources)
cargo deny check all
```

#### Outdated Dependencies

```bash
# Install cargo-outdated
cargo install cargo-outdated

# Check for outdated dependencies
cargo outdated

# Update dependencies
cargo update
```

#### Documentation

```bash
# Build documentation
cargo doc --all-features --no-deps --open

# Run doc tests
cargo test --doc --all-features
```

#### Pre-commit Hooks

The project includes pre-commit hooks that run `cargo fmt` and `cargo clippy` before each commit:

```bash
# Install hooks (one-time setup)
./scripts/install-hooks.sh

# Hooks will automatically run on commit
# To skip hooks (not recommended): git commit --no-verify
```

### CI/CD

The project uses GitHub Actions for continuous integration:

- **CI Workflow** (`ci.yml`): Runs on every push/PR
  - Formatting check (`cargo fmt`)
  - Clippy lints (default, all-features, no-features)
  - Build (debug and release)
  - Tests (all features and no features)
  - Unused dependency check (`cargo-udeps`)
  - Documentation build

- **Security Workflow** (`security.yml`): Runs on dependency changes and weekly
  - Security audit (`cargo-audit`)
  - License and dependency check (`cargo-deny`)
  - Outdated dependency check (weekly/manual)

- **Coverage Workflow** (`coverage.yml`): Runs on every push/PR
  - Code coverage with `cargo-llvm-cov`
  - Reports to Codecov

### Running Tests

```bash
# Run all tests (debug mode, 454 tests)
cargo test --all-features

# Run all tests (release mode)
cargo test --all-features --release

# Run specific test
cargo test test_name --all-features

# Run tests with output
cargo test --all-features -- --nocapture

# Run tests from specific module
cargo test tui::events --all-features
cargo test testing::integration --all-features

# Run shell-based integration tests (requires auggie)
./tests/integration/run_all_tests.sh
./tests/integration/run_all_tests.sh release
```

### Test Coverage

The test suite covers:
- **454 tests** in debug mode (includes debug-only model override tests)
- **7 mock scenarios** for integration testing
- **33+ TUI event handling tests**
- **67+ configuration tests** (loading, merging, resolving, writing, validation)
- **47+ MCP server protocol tests**
- **Error type tests** for all error modules

## License

MIT

