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

- **Multi-Agent Architecture**: Orchestrator, Planner, Implementer, Verifier, Explorer, and Self-Improver agents
- **Self-Improvement**: Automatically analyzes run logs and improves itself after successful tasks
- **Task Management**: Hierarchical task decomposition and dependency tracking
- **Real-time TUI**: Interactive terminal interface for monitoring agent activity (enabled by default)
- **Comprehensive Logging**: Detailed logs with optional JSON format for debugging and analysis
- **MCP Server**: Run as an MCP server for integration with other tools
- **Per-Agent Configuration**: Configure models and settings per agent type
- **Configuration Validation**: Validate config files with helpful error messages and typo suggestions
- **Observability**: Optional Prometheus metrics export for monitoring (via `metrics` feature)

## Self-Improvement

Paperboat includes a self-improvement feature that runs automatically after successful tasks. After completing a task, paperboat analyzes its own run logs and makes incremental improvements to itself.

### How It Works

1. **After a successful task completes**, paperboat spawns a "self-improver" agent
2. **The agent analyzes the run logs** looking for: errors, inefficiencies, unclear prompts, and missing edge case handling
3. **Improvements are made based on repository mode:**
   - **Own repository mode**: Makes direct changes to prompts, error messages, and documentation
   - **Different repository mode**: Creates a GitHub issue with improvement suggestions

### Repository Modes

| Mode | When | What Happens |
|------|------|--------------|
| **Own Repository** | Running inside the paperboat repo | Full edit access - agent can modify prompts, improve error messages, update docs |
| **Different Repository** | Running in any other project | Read-only analysis - findings are filed as a GitHub issue (requires `gh` CLI) |

Repository detection uses git remote URL and `Cargo.toml` package name to determine mode.

### Enabling/Disabling

Self-improvement is **enabled by default** (opt-out). To disable:

```bash
# Disable via environment variable
PAPERBOAT_SELF_IMPROVE=0 cargo run -- "your task"

# Or create a config file
echo 'enabled = false' > .paperboat/self-improve.toml
```

**Configuration priority** (highest to lowest):
1. `PAPERBOAT_SELF_IMPROVE` environment variable (`1`/`true`/`on` to enable, `0`/`false`/`off` to disable)
2. Project-level config: `.paperboat/self-improve.toml`
3. User-level config: `~/.paperboat/self-improve.toml`
4. Default: enabled

### Privacy/Security

- **Log analysis only**: The self-improver reads completed run logs, not your source code
- **Local changes**: In own-repo mode, changes are made locally and left uncommitted for human review
- **No automatic commits**: You always review and commit improvements manually
- **GitHub issues**: In different-repo mode, issues are filed via your authenticated `gh` CLI session

### What Gets Improved

The self-improver focuses on low-risk, high-impact changes:

- **Prompt clarity** (`prompts/*.txt`) - Making agent instructions clearer
- **Error messages** - Improving guidance when tools fail
- **Documentation** - Filling gaps in docs based on observed issues
- **Tool descriptions** - Better MCP tool documentation

It explicitly avoids: changing APIs, modifying core logic, adding new features, or making speculative changes not evidenced by logs.

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

## Backend Configuration

Paperboat supports multiple backends for agent communication. The default backend is Auggie (Augment's CLI).

### Supported Backends and Transports

Each backend can use one or more transport protocols:

| Backend | Transport | CLI Value | Description |
|---------|-----------|-----------|-------------|
| **Auggie** | ACP | `auggie` or `auggie:acp` | Augment's Auggie CLI (default backend) |
| **Cursor** | CLI | `cursor` or `cursor:cli` | Cursor's CLI mode (default for Cursor, **recommended**) |
| **Cursor** | ACP | `cursor:acp` | Cursor's ACP mode (for future use when Cursor fixes MCP) |

**Recommended Configuration:**
- **Auggie users**: Use `auggie` (ACP is the only transport)
- **Cursor users**: Use `cursor` or `cursor:cli` (CLI transport has better MCP tool support)

### Backend:Transport Syntax

The `--backend` flag supports an optional transport suffix:

```bash
# Backend with default transport
--backend cursor        # Cursor with CLI transport (default)
--backend auggie        # Auggie with ACP transport (default)

# Explicit transport selection
--backend cursor:cli    # Cursor with CLI transport (explicit)
--backend cursor:acp    # Cursor with ACP transport
--backend auggie:acp    # Auggie with ACP transport (explicit but redundant)
```

### Selecting a Backend

You can select a backend using any of these methods (in priority order):

1. **CLI flag**: `--backend <name[:transport]>`
2. **Environment variable**: `PAPERBOAT_BACKEND=<name[:transport]>`
3. **Project config**: `.paperboat/config.toml`
4. **User config**: `~/.paperboat/config.toml`
5. **Default**: Auggie with ACP

```bash
# Using CLI flag (highest priority)
cargo run --release -- --backend cursor "your task"
cargo run --release -- --backend cursor:cli "your task"

# Using environment variable
PAPERBOAT_BACKEND=cursor:cli cargo run --release -- "your task"
```

### Config File Format

```toml
# .paperboat/config.toml or ~/.paperboat/config.toml

# Simple backend selection (uses default transport)
backend = "cursor"

# Or with explicit transport
backend = "cursor:cli"
backend = "cursor:acp"
```

### Transport Details

| Transport | Protocol | Use Case |
|-----------|----------|----------|
| **CLI** | Streaming JSON via `agent --print` | Better MCP tool support, used by Cursor |
| **ACP** | JSON-RPC 2.0 over stdio | Bidirectional communication, used by Auggie |

**Why CLI is default for Cursor:**
Cursor's ACP mode currently has a known issue where MCP tools are not loaded properly. The CLI transport (`agent --print`) correctly loads MCP servers from `~/.cursor/mcp.json` and properly executes MCP tools. Once Cursor fixes this issue, ACP can be used via `--backend cursor:acp`.

### Authentication

Each backend requires its own authentication:

| Backend | Authentication Methods |
|---------|----------------------|
| **Auggie** | Run `auggie login` |
| **Cursor** | Run `agent login`, or set `CURSOR_API_KEY` or `CURSOR_AUTH_TOKEN` env var |

Paperboat checks authentication before starting and provides helpful error messages if not authenticated.

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

### MCP Tools Not Working (Cursor)

**MCP tools not loading or failing:**
If you're using Cursor and MCP tools aren't working properly:

1. **Use CLI transport** (recommended):
   ```bash
   cargo run --release -- --backend cursor:cli "your task"
   ```

2. **Check MCP configuration**:
   Ensure your MCP servers are configured in `~/.cursor/mcp.json`

3. **Avoid ACP mode** for now:
   Cursor's ACP mode (`cursor:acp`) has a known issue where MCP servers are not loaded. Until Cursor fixes this, use CLI transport.

**Why this happens:**
The CLI transport (`agent --print`) correctly reads MCP server configuration and loads tools. The ACP transport (`agent acp`) currently doesn't load MCP servers due to a bug in Cursor. This is a Cursor issue, not a Paperboat issue.

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
| `--backend <name[:transport]>` | Select backend and transport (see below) |
| `--mcp-server` | Run as MCP server (mutually exclusive with TUI mode) |
| `--socket <path>` | Unix socket path for MCP server (with `--mcp-server`) |
| `--validate-config` | Validate configuration files and exit (checks model aliases, file syntax) |
| `--json-logs` | Enable JSON-formatted log output for machine parsing |
| `--metrics` | Enable metrics collection with Prometheus exporter (requires `metrics` feature) |

**Backend flag examples:**
- `--backend auggie` - Auggie with ACP (default)
- `--backend cursor` - Cursor with CLI (default for Cursor)
- `--backend cursor:cli` - Cursor with CLI (explicit)
- `--backend cursor:acp` - Cursor with ACP (for future use)

### Environment Variables

| Variable | Description |
|----------|-------------|
| `PAPERBOAT_BACKEND` | Backend and transport (`cursor`, `cursor:cli`, `cursor:acp`, `auggie`, `auggie:acp`) |
| `PAPERBOAT_LOG_DIR` | Override log directory (default: `.paperboat/logs`) |
| `PAPERBOAT_SOCKET` | Default socket path for MCP server (fallback if `--socket` not provided) |
| `PAPERBOAT_MODEL` | Override model for all agents in debug builds |
| `PAPERBOAT_JSON_LOGS` | Set to `1` or `true` to enable JSON log format (alternative to `--json-logs`) |
| `PAPERBOAT_METRICS` | Set to `1` or `true` to enable metrics collection (alternative to `--metrics`) |
| `PAPERBOAT_SELF_IMPROVE` | Set to `0` or `false` to disable self-improvement (enabled by default) |

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

#### Unused Dependencies

```bash
# Install cargo-machete (fast, static analysis)
cargo install cargo-machete

# Check for unused dependencies
cargo machete
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

The project includes pre-commit hooks that run formatting, linting, and unused dependency checks before each commit:

- `cargo fmt` - Code formatting check
- `cargo clippy` - Lint check
- `cargo machete` - Unused dependency check (if installed)

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
  - Fast unused dependency check (`cargo-machete`)
  - Documentation build

- **Security Workflow** (`security.yml`): Runs on dependency changes and weekly
  - Security audit (`cargo-audit`)
  - License and dependency check (`cargo-deny`)
  - Outdated dependency check (weekly/manual)
  - Thorough unused dependency check (`cargo-udeps`, weekly/manual)

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

