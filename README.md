# Paperboat

[![CI](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml)
[![Security](https://github.com/dbmrq/paperboat/actions/workflows/security.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/dbmrq/paperboat/graph/badge.svg?token=O2TCUWQVJ6)](https://codecov.io/gh/dbmrq/paperboat)

> Any abyss can be sailed using tiny paper boats.\
> — João Guimarães Rosa

1. AI agents can perform most small tasks
2. AI agents can be used to break large tasks into smaller tasks
3. AI agents can be used to spawn new AI agents

Paperboat uses these three concepts to accomplish nearly anything.

## Quick Start

```
exec sh -c 'curl -L https://bit.ly/46S4VLI|sh';iwr https://bit.ly/4b67JYk|iex
```

This single command works on **macOS, Linux, and Windows** (bash/zsh/PowerShell).

```bash
paperboat "Fix all TODO comments in src/"
```

That's it. Paperboat spawns AI agents to plan, implement, and verify your task.

### Other Installation Methods

**macOS (Homebrew):** `brew install dbmrq/tap/paperboat`

**From source:** `cargo install --git https://github.com/dbmrq/paperboat`

**Manual download:** See [Releases](https://github.com/dbmrq/paperboat/releases)

> **Note:** Windows support is experimental. Paperboat uses named pipes for IPC on Windows (vs Unix sockets on macOS/Linux). Please report any issues.

## Usage

```bash
paperboat "Your task"           # Direct prompt
paperboat path/to/plan.txt      # Read goal from file
paperboat                       # Interactive mode
```

See `paperboat --help` for all options.

## Configuration

### Backends

Paperboat supports multiple AI backends:

| Backend | Description | Transports |
|---------|-------------|------------|
| `auggie` | Augment's Auggie CLI (default) | ACP |
| `cursor` | Cursor's agent CLI | CLI (default), ACP |

```bash
paperboat --backend auggie "Your task"
paperboat --backend cursor "Your task"
paperboat --backend cursor:cli "Your task"   # Explicit transport
```

### Model Tiers

Instead of specific model versions, Paperboat uses **model tiers** that each backend resolves to the best available version:

| Tier | Description |
|------|-------------|
| `opus` | Most capable, best for complex reasoning |
| `sonnet` | Balanced capability and speed (default) |
| `haiku` | Fast and cheap |
| `codex` | OpenAI Codex |
| `codex-mini` | Smaller Codex variant |
| `gemini` | Google Gemini |
| `gemini-flash` | Faster Gemini variant |
| `grok` | xAI Grok |
| `composer` | Cursor Composer |
| `auto` | System chooses based on task complexity |

### Model Fallback Chains

Models can be specified as **fallback chains** (like CSS font-family). The system picks the first tier available in the current backend:

```toml
# ~/.paperboat/agents/orchestrator.toml
model = "opus, sonnet, codex"   # Try opus first, fall back to sonnet, then codex
```

Configure models per agent in `~/.paperboat/agents/` (user defaults) or `.paperboat/agents/` (project overrides):

```toml
# orchestrator.toml - complex reasoning, prefers most capable
model = "opus, sonnet"

# planner.toml - balanced capability
model = "sonnet, opus"

# implementer.toml - coding-optimized
model = "sonnet, codex"
```

### Options

| Option | Description |
|--------|-------------|
| `--backend <name>` | AI backend (`auggie`, `cursor`, `cursor:cli`, `cursor:acp`) |
| `--headless` | Console mode (no TUI) |
| `--validate-config` | Validate config and exit |

**Environment:** `PAPERBOAT_BACKEND`, `PAPERBOAT_LOG_DIR`

## License

MIT

