# Paperboat

[![CI](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml)
[![Security](https://github.com/dbmrq/paperboat/actions/workflows/security.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/dbmrq/paperboat/branch/main/graph/badge.svg)](https://codecov.io/gh/dbmrq/paperboat)

**Autonomous AI agents that break down complex tasks and accomplish nearly anything.**

## Quick Start

```bash
# Install (macOS)
brew install dbmrq/tap/paperboat

# Run
paperboat "Fix all TODO comments in src/"
```

That's it. Paperboat spawns AI agents to plan, implement, and verify your task.

### Other Platforms

**Linux/Windows:** Download from [Releases](https://github.com/dbmrq/paperboat/releases) or `cargo install --git https://github.com/dbmrq/paperboat`

## Usage

```bash
paperboat "Your task"           # Direct prompt
paperboat path/to/plan.txt      # Read goal from file
paperboat                       # Interactive mode
```

See `paperboat --help` for all options.

## The Idea

> "Any abyss can be sailed using tiny paper boats." — João Guimarães Rosa

1. AI agents can perform small tasks
2. AI agents can break large tasks into smaller tasks
3. AI agents can spawn new AI agents

Paperboat uses these three concepts to accomplish nearly anything.

## Configuration

### Model Selection

Configure models per agent in `~/.paperboat/agents/` (user defaults) or `.paperboat/agents/` (project overrides):

```toml
# orchestrator.toml, planner.toml, or implementer.toml
model = "opus"
```

### Options

| Option | Description |
|--------|-------------|
| `--backend <name>` | AI backend (`auggie`, `cursor`) |
| `--headless` | Console mode (no TUI) |
| `--validate-config` | Validate config and exit |

**Environment:** `PAPERBOAT_BACKEND`, `PAPERBOAT_LOG_DIR`

## License

MIT

