# AI Agent Guidelines for Paperboat

This document provides guidelines for AI agents working on this codebase.

## MCP Design Principles

Paperboat uses an intent-based MCP design philosophy. When modifying agent prompts or MCP tool definitions, follow these principles:

### 1. Agent Prompts Must NOT Mention Specific Tools

**❌ Don't do this:**
```
## What You CAN Do
- Read files with `view`
- Search code with `codebase-retrieval`
- Run tests with `launch-process`
```

**✅ Do this instead:**
```
## Capabilities
You have MCP tools available for reading files, searching code, and running tests.
Review the available tools to understand your options.
```

**Why:** Tools should advertise themselves through their descriptions. Prompts should focus on *intent* (what to accomplish), not *mechanism* (which tools to use). This allows tools to evolve without updating every prompt.

### 2. Tools Must Self-Document

Tool descriptions in `src/mcp_server/handlers/tool_schemas.rs` should include:

- `<usecase>` - When to use this tool
- `<instructions>` - How to use it effectively  
- `<on_error>` - Recovery guidance when things go wrong

**Example:**
```rust
"<usecase>Execute work by spawning specialized agents.</usecase>
<instructions>Reference tasks by task_id for proper tracking.</instructions>
<on_error>If task_id not found, use list_tasks() to see available tasks.</on_error>"
```

### 3. Responses Are Prompts

Every tool response is an opportunity to guide the agent's next action. Responses should include:

- What happened (the result)
- Contextual "What's Next" guidance based on actual state
- For errors: specific, actionable recovery steps

**❌ Bad error:**
```
"Unauthorized"
```

**✅ Good error:**
```
"Task ID 'task999' not found. Use list_tasks() to see available task IDs, 
or create a new task with create_task()."
```

### 4. Error Messages Are Documentation

Errors must be educational:
- Explain *why* it failed
- Provide *specific* next steps
- Reference *related tools* that can help

## File Locations

| Purpose | Location |
|---------|----------|
| Agent prompts | `prompts/*.txt` |
| Tool schemas | `src/mcp_server/handlers/tool_schemas.rs` |
| Response builders | `src/mcp_server/handlers/response.rs` |
| Design documentation | `docs/INTENT_BASED_MCP_DESIGN.md` |

## Common Mistakes to Avoid

1. **Adding tool names to prompts** - Let tools describe themselves
2. **Generic error messages** - Always include recovery guidance
3. **Static "Next Steps"** - Make responses context-aware when possible
4. **Duplicating tool info** - Single source of truth in tool descriptions

## See Also

- `docs/INTENT_BASED_MCP_DESIGN.md` - Full design plan and implementation details
- `prompts/README.md` - Prompt file conventions

