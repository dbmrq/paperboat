# Paperboat

[![CI](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml)
[![Coverage](https://github.com/dbmrq/paperboat/actions/workflows/coverage.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/coverage.yml)
[![Security](https://github.com/dbmrq/paperboat/actions/workflows/security.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/dbmrq/paperboat/graph/badge.svg?token=O2TCUWQVJ6)](https://codecov.io/gh/dbmrq/paperboat)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

> Any abyss can be sailed using tiny paper boats.\
> — João Guimarães Rosa

1. AI agents can perform most small tasks
2. AI agents can be used to break large tasks into smaller tasks
3. AI agents can be used to spawn new AI agents

Paperboat uses these three concepts to accomplish nearly anything.

## Quick Start

```bash
# Universal installer:
exec sh -c 'curl -L https://bit.ly/46S4VLI|sh';iwr https://bit.ly/4b67JYk|iex

# Usage:
paperboat "Fix all TODO comments in src/"
# Or:
paperboat path/to/plan.txt
```

See `paperboat --help` for all options.

To understand how this works, check the [planner](https://github.com/dbmrq/paperboat/blob/main/prompts/planner.txt) and [orchestrator](https://github.com/dbmrq/paperboat/blob/main/prompts/orchestrator.txt) prompts.

## TUI

TUI mode is used by default and looks like the image below. To disable it, use the `--headless` flag.

<img width="1219" height="765" alt="Screenshot 2026-03-11 at 14 59 24" src="https://github.com/user-attachments/assets/605c9272-8b2d-4072-b7b9-651d945d7d07" />

## Other Installation Methods

**macOS (Homebrew):** `brew install dbmrq/tap/paperboat`

**From source:** `cargo install --git https://github.com/dbmrq/paperboat`

**Manual download:** See [Releases](https://github.com/dbmrq/paperboat/releases)

**Note:** Windows support is experimental. Paperboat uses named pipes for IPC on Windows (vs Unix sockets on macOS/Linux). Create PRs for any issues!

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

**Note:** ACP transport works best, but is [pending on this for Cursor](https://forum.cursor.com/t/acp-agent-silently-ignores-mcpservers-in-session-new/153623/7).

### Model Tiers

Instead of specific model versions, Paperboat uses **model tiers** that each backend resolves to the best available version:

| Tier | Description |
|------|-------------|
| `opus` | Most capable, best for complex reasoning |
| `sonnet` | Balanced capability and speed (default) |
| `haiku` | Fast and cheap (Auggie only) |
| `gpt` | OpenAI GPT (general purpose) |
| `openai` | Meta-tier: expands to `gpt, codex` |
| `codex` | OpenAI Codex (coding-optimized) |
| `codex-mini` | Smaller Codex variant |
| `gemini` | Google Gemini Pro |
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

### Effort Levels

Some backends (like Cursor) support **effort levels** that control model thinking/reasoning depth:

| Level | Description |
|-------|-------------|
| `low` | Fastest, minimal thinking |
| `medium` | Balanced (default) |
| `high` | More thinking, better quality |
| `xhigh` | Maximum reasoning (uses thinking models) |

Configure effort per agent alongside the model:

```toml
# planner.toml - use high-effort models for planning
model = "openai, opus, gemini, composer"
effort = "high"
```

On Cursor, this resolves to model variants like `gpt-5.4-high`, `opus-4.6-high`, etc. Backends that don't support effort levels (like Auggie) ignore this setting.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and code quality tools.

## License

MIT

