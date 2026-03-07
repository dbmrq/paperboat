# Intent-Based MCP Design & Response-as-Prompt Pattern

This document outlines the strategy for improving agent-tool interactions in Paperboat.

## Core Principles

1. **Agent prompts shouldn't mention specific MCP tools** - They should mention *useful strategies* and *checking MCP tools in general*, but the MCP tools should advertise themselves with good descriptions.

2. **Every response is an opportunity to nudge** - When returning results from an MCP tool, include not just the data but contextual guidance: "the result was XYZ; now you can do A, B, or C."

3. **Error messages are documentation** - Errors must be educational, explaining why something failed and providing specific, actionable next steps.

4. **Design around user intent, not API endpoints** - Tools should map to what agents want to accomplish, not to internal implementation details.

## Current State Analysis

### What's Already Working Well

1. **Tool descriptions use `<usecase>/<instructions>` XML tags** - See `tool_schemas.rs`
2. **Response builders include "Next Steps"** - `response.rs` provides contextual guidance
3. **Notes and suggested tasks flow back** - `build_summary_with_notes_and_suggested_tasks()` enriches responses

### What Needs Improvement

| Area | Current State | Problem |
|------|--------------|---------|
| Agent prompts | Mention specific tools by name | Tools should advertise themselves |
| Error messages | Generic ("Failed to create task") | Not educational; no recovery path |
| Response context | Static "Next Steps" sections | Not dynamic based on actual state |
| Tool responses | Data + generic next steps | Missing contextual nudges |

## Implementation Plan

### Phase 0: Document the Strategy ✅ DONE

Updated `AGENTS.md` to codify these principles. This prevents future agents from re-adding specific tool mentions to prompts.

**Files created:**
- `AGENTS.md` - MCP Design Principles section (root of repo)
- `docs/INTENT_BASED_MCP_DESIGN.md` - This detailed plan

**Key rules documented:**
- Never mention specific MCP tool names in agent prompts
- Tools self-document via descriptions; prompts describe intent
- Error messages must include recovery guidance
- Responses should include contextual "what's next" based on state

**Note:** If you adopt GitHub Copilot or Cursor in the future, create corresponding
context files (`.github/copilot-instructions.md`, `.cursorrules`) that reference `AGENTS.md`.

### Phase 1: High Impact, Low Effort

1. **Improve error messages** in `response.rs` with recovery guidance
2. **Add `<on_error>` tags** to tool descriptions in `tool_schemas.rs`
3. **Remove specific tool names** from agent prompts (`prompts/*.txt`)

### Phase 2: Medium Effort, High Value

4. **Pass task state to response builders** - Extend `ToolResponse` or pass context
5. **Make "Next Steps" context-aware** - Show actual remaining tasks, parallel opportunities
6. **Add hints about dependency unlocking** - "This task unblocked X, Y, Z"

### Phase 3: Polish

7. **Create centralized contextual hints module** - `src/mcp_server/contextual_hints.rs`
8. **Refine guidance based on real runs** - Test and iterate
9. **Add hints for edge cases** - Blocked tasks, suggested tasks needing attention

## Detailed Changes

### Agent Prompts: Remove Tool-Specific Instructions

**Files:** `prompts/*.txt`

**Current (explorer.txt):**
```
## What You CAN Do
- Read files with `view`
- Search code with `codebase-retrieval`  
- Search the web with `web-search`
- Fetch web pages with `web-fetch`
```

**Proposed:**
```
## Capabilities
You have MCP tools available for reading files, searching code, and researching online.
Review the available tools to understand your options.
```

**Apply to:** `explorer.txt`, `verifier.txt`, `implementer.txt`, `orchestrator.txt`

### Enhance Tool Descriptions

**File:** `src/mcp_server/handlers/tool_schemas.rs`

Add `<on_error>` tags with recovery guidance:

```rust
// Example for spawn_agents
"<usecase>Execute work by spawning specialized agents for your tasks.</usecase>
<instructions>Reference tasks by task_id to ensure proper tracking.
For parallel work, spawn multiple agents without dependencies together.</instructions>
<on_error>If task_id is not found, use list_tasks() to see available tasks.
If you need ad-hoc work, use create_task() first.</on_error>"
```

### Make Response Builders Context-Aware

**File:** `src/mcp_server/handlers/response.rs`

Include actual task state in responses:

```
✅ Spawned 3 agent(s) completed successfully.

## Summary
[implementer] ✓ Added user model

## What's Next
- **3 tasks remaining**: task004, task005, task006
- task004 and task005 have no dependencies—spawn them together
- task006 depends on task005; wait for it first
```

### Enrich Error Messages

**Current:**
```
"Failed to create task '{}': {}"
```

**Proposed:**
```
"❌ Failed to create task 'X': {error}

## How to Fix
- If the task name already exists, use a different name or find it with list_tasks()
- If this is a planning issue, review the goal with set_goal() first"
```

## Files to Modify

| File | Changes |
|------|---------|
| `AGENTS.md` | Add MCP design principles section |
| `prompts/explorer.txt` | Remove tool-specific instructions |
| `prompts/verifier.txt` | Remove tool-specific instructions |
| `prompts/implementer.txt` | Minor cleanup |
| `prompts/orchestrator.txt` | Remove specific tool mentions |
| `src/mcp_server/handlers/tool_schemas.rs` | Add `<on_error>` tags |
| `src/mcp_server/handlers/response.rs` | Context-aware responses |
| `src/mcp_server/types.rs` | Extend `ToolResponse` with task state |
| `src/app/orchestrator.rs` | Pass task state to response builders |
| `src/tasks/manager.rs` | Add helper methods for hints |
| New: `src/mcp_server/contextual_hints.rs` | Centralized hint logic |

## References

- Original design philosophy: [MCP Design Post](internal reference)
- See also: `prompts/README.md` for prompt conventions

