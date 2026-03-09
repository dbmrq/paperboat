# Agent Prompts

This directory contains system prompts for Paperboat's AI agents. Prompts are embedded at compile time, so changes require a rebuild.

## Prompt Files

| File | Purpose |
|------|---------|
| `orchestrator.txt` | Root orchestrator - coordinates task execution and spawns agents |
| `planner.txt` | Creates detailed implementation plans from high-level goals |
| `implementer.txt` | Executes individual tasks (file editing, process execution) |
| `verifier.txt` | Validates implementation (read-only, can run tests) |
| `explorer.txt` | Gathers information (read-only, no execution) |
| `selfimprover.txt` | Analyzes run logs and makes incremental improvements |

## Adding New Agent Types

To add a new agent type:

1. Create `prompts/newrole.txt` with the system prompt
2. Optionally configure tool restrictions in `src/agents/config.rs`
3. Rebuild the project

The build script (`build.rs`) auto-discovers prompt files and generates the necessary code.

## Prompt Template Variables

Prompts can include these placeholders (filled at runtime):

- `{task}` - The specific task description for this agent
- `{user_goal}` - The user's original high-level goal
- `{context}` - Additional context from previous agents

## Runtime Configuration

Model selection (which AI model to use) is configured separately via:
- User-level: `~/.paperboat/agents/*.toml`
- Project-level: `.paperboat/agents/*.toml`

See the main README for configuration details.

