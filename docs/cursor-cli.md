# Cursor CLI Transport

The CLI transport is the **recommended** way to use Paperboat with Cursor. It provides better MCP tool support compared to the ACP transport.

## Usage

```bash
# Using CLI transport (default for Cursor)
paperboat --backend cursor "your task"

# Explicit CLI transport selection
paperboat --backend cursor:cli "your task"
```

## Why CLI Transport?

The CLI transport uses Cursor's non-interactive print mode (`agent --print`) which:

1. **Properly loads MCP servers** from `~/.cursor/mcp.json`
2. **Correctly executes MCP tools** registered with Paperboat
3. **Provides structured streaming output** for real-time progress

In contrast, Cursor's ACP mode currently has a bug where MCP servers are not loaded.

## How It Works

When Paperboat uses the CLI transport, it:

1. **Spawns a new `agent` process** for each prompt with the following flags:
   ```bash
   agent --print --force --approve-mcps --trust --output-format stream-json --model <model> --workspace <path>
   ```

2. **Parses streaming JSON output** line by line, converting events to `SessionUpdate` messages

3. **Applies permission policy** based on agent type (orchestrator, planner, implementer)

### Key CLI Flags

| Flag | Purpose |
|------|---------|
| `--print` | Non-interactive mode for scripting |
| `--force` | Allow file modifications without confirmation |
| `--approve-mcps` | Auto-approve MCP servers |
| `--trust` | Trust workspace without prompting |
| `--output-format stream-json` | Structured streaming output |
| `--model` | Select the AI model |
| `--workspace` | Set working directory |
| `--resume` | Resume a previous session |

## Streaming JSON Output Format

The CLI outputs newline-delimited JSON with the following message types:

### System Init
```json
{"type": "system", "subtype": "init", "model": "claude-3-5-sonnet"}
```

### Assistant Message
```json
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I'll help you..."}]}}
```

### Tool Call Started
```json
{
  "type": "tool_call",
  "subtype": "started",
  "tool_call": {
    "readToolCall": {"args": {"path": "src/main.rs"}}
  }
}
```

### Tool Call Completed
```json
{
  "type": "tool_call",
  "subtype": "completed",
  "tool_call": {
    "readToolCall": {"result": {"success": {"totalLines": 100}}}
  }
}
```

### Session Result
```json
{
  "type": "result",
  "subtype": "success",
  "session_id": "abc-123",
  "duration_ms": 5000
}
```

## Permission Policy

Paperboat applies different permission policies based on agent type:

| Agent Type | File Editing | MCP Tools | Description |
|------------|--------------|-----------|-------------|
| **Orchestrator** | ❌ No | Limited | Coordinates agents, no direct file access |
| **Planner** | ❌ No | Limited | Creates plans, read-only access |
| **Implementer** | ✅ Yes | Full | Executes tasks, full tool access |

The permission policy is enforced by:
- Providing only allowed tools to each agent type
- Configuring `--force` only for implementers

## Session Resumption

The CLI transport supports session resumption using the `--resume` flag:

```bash
# First prompt creates a session
agent --print "Start analyzing the code"
# Output includes: {"type": "result", "session_id": "abc-123"}

# Subsequent prompts can resume the session
agent --print --resume abc-123 "Continue with the implementation"
```

Paperboat automatically manages session IDs for multi-turn conversations.

## Comparison: CLI vs ACP

| Feature | CLI Transport | ACP Transport |
|---------|---------------|---------------|
| MCP Tools | ✅ Works | ❌ Broken |
| Session Persistence | Via `--resume` | Built-in |
| Bidirectional Comms | ❌ One-way | ✅ Full duplex |
| Tool Approval | Auto-approve | Manual/policy |
| Process Model | New process per prompt | Persistent process |

## Troubleshooting

### MCP tools not appearing

1. Verify MCP configuration exists:
   ```bash
   cat ~/.cursor/mcp.json
   ```

2. Check that Paperboat registered its MCP server:
   ```bash
   grep paperboat ~/.cursor/mcp.json
   ```

3. Ensure you're using CLI transport (not ACP):
   ```bash
   paperboat --backend cursor:cli "your task"
   ```

### Session not resuming

- Session IDs expire after inactivity
- Ensure `--resume` flag is being passed correctly
- Check logs for session ID extraction

## Implementation Details

The CLI transport is implemented in `src/backend/cursor/cli.rs`:

- `CursorCliTransport` struct manages the CLI process
- `parse_output_line()` converts JSON to `SessionUpdate`
- `spawn_agent()` launches the `agent` command with appropriate flags
- Permissions are applied via `PermissionPolicy`

See also:
- [Transport Architecture](cursor-cli-transport-implementation.md)
- [ACP Transport](cursor-acp.md) (for when Cursor fixes MCP support)