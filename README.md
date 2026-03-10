# Paperboat

[![CI](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/ci.yml)
[![Security](https://github.com/dbmrq/paperboat/actions/workflows/security.yml/badge.svg)](https://github.com/dbmrq/paperboat/actions/workflows/security.yml)
[![codecov](https://codecov.io/gh/dbmrq/paperboat/graph/badge.svg)](https://codecov.io/gh/dbmrq/paperboat)

"Any abyss can be sailed using tiny paper boats." — João Guimarães Rosa

1. AI agents can perform small tasks
2. AI agents can be used to break large tasks into smaller tasks
3. AI agents can be used to spawn new AI agents

Paperboat uses these three concepts to accomplish nearly anything.


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


## License

MIT

