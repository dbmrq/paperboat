# Batch 3: Agent Templates + Logging Cleanup (Phases 4+5)

## Prerequisites

- Batch 2 (structured tasks) is complete
- `cargo check` and `cargo test` pass before starting
- The following should already exist from Batch 2:
  - TaskManager in App struct
  - create_task MCP tool working
  - LogEvent variants for TaskCreated/TaskStateChanged

**This batch is INDEPENDENT of Batch 2's internal implementation.** It only requires that the codebase compiles and tests pass.

---

## Context

This batch adds agent templates (verifier, explorer, custom) and cleans up logging.

### Design Principles
- **Templates** - Predefined agent configurations with appropriate tool restrictions
- **Ad-hoc agents** - Custom agents with explicit tool whitelist
- **Clean logging** - Remove duplicate console output, rely on AgentWriter + LogEvents

### Tool Surface After This Batch

| Agent Type | Tools | Augment Tools |
|------------|-------|---------------|
| **Orchestrator** | `spawn_agents`, `decompose`, `complete` | read-only |
| **Planner** | `create_task`, `complete` | read-only, NO built-in task tools |
| **Implementer** | `complete` | all editing tools |
| **Verifier** | `complete` | read-only + launch-process (for running tests) |
| **Explorer** | `complete` | read-only + web access (no execution) |
| **Custom** | `complete` | explicit whitelist only |

---

## Implementation Tasks

### Task 1: Create Agent Module with AgentRole

Create `src/agents/mod.rs`:

```rust
//! Agent templates and registry.

mod templates;

pub use templates::{AgentRegistry, AgentTemplate};

/// Agent roles that can be spawned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentRole {
    Implementer,
    Verifier,
    Explorer,
    Custom,
}

impl AgentRole {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "implementer" => Some(Self::Implementer),
            "verifier" => Some(Self::Verifier),
            "explorer" => Some(Self::Explorer),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Implementer => "implementer",
            Self::Verifier => "verifier",
            Self::Explorer => "explorer",
            Self::Custom => "custom",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_role_from_str() {
        assert_eq!(AgentRole::from_str("implementer"), Some(AgentRole::Implementer));
        assert_eq!(AgentRole::from_str("VERIFIER"), Some(AgentRole::Verifier));
        assert_eq!(AgentRole::from_str("Explorer"), Some(AgentRole::Explorer));
        assert_eq!(AgentRole::from_str("custom"), Some(AgentRole::Custom));
        assert_eq!(AgentRole::from_str("unknown"), None);
    }

    #[test]
    fn test_agent_role_as_str() {
        assert_eq!(AgentRole::Implementer.as_str(), "implementer");
        assert_eq!(AgentRole::Verifier.as_str(), "verifier");
    }
}
```

### Task 2: Create Agent Templates

Create `src/agents/templates.rs`:

```rust
//! Built-in agent templates.

use super::AgentRole;
use std::collections::HashMap;

/// Template defining an agent's prompt and tool restrictions.
pub struct AgentTemplate {
    /// The prompt template with {task} and {user_goal} placeholders.
    pub prompt_template: &'static str,
    /// Tools to remove from this agent type.
    pub removed_tools: Vec<&'static str>,
}

/// Registry of built-in agent templates.
pub struct AgentRegistry {
    templates: HashMap<AgentRole, AgentTemplate>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        let mut templates = HashMap::new();

        templates.insert(AgentRole::Implementer, AgentTemplate {
            prompt_template: include_str!("../../prompts/implementer.txt"),
            removed_tools: vec![],  // Gets all tools
        });

        templates.insert(AgentRole::Verifier, AgentTemplate {
            prompt_template: include_str!("../../prompts/verifier.txt"),
            removed_tools: vec![
                // No editing - read-only except for running tests
                "str-replace-editor",
                "save-file",
                "remove-files",
            ],
        });

        templates.insert(AgentRole::Explorer, AgentTemplate {
            prompt_template: include_str!("../../prompts/explorer.txt"),
            removed_tools: vec![
                // No editing
                "str-replace-editor",
                "save-file",
                "remove-files",
                // No execution (can only read and search)
                "launch-process",
                "kill-process",
                "read-process",
                "write-process",
                "list-processes",
            ],
        });

        Self { templates }
    }

    pub fn get(&self, role: &AgentRole) -> Option<&AgentTemplate> {
        self.templates.get(role)
    }

    pub fn has_role(&self, role: &AgentRole) -> bool {
        self.templates.contains_key(role)
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_implementer() {
        let registry = AgentRegistry::new();
        let template = registry.get(&AgentRole::Implementer).unwrap();
        assert!(template.removed_tools.is_empty());
    }

    #[test]
    fn test_registry_has_verifier() {
        let registry = AgentRegistry::new();
        let template = registry.get(&AgentRole::Verifier).unwrap();
        assert!(template.removed_tools.contains(&"str-replace-editor"));
        assert!(!template.removed_tools.contains(&"launch-process")); // Can run tests
    }

    #[test]
    fn test_registry_has_explorer() {
        let registry = AgentRegistry::new();
        let template = registry.get(&AgentRole::Explorer).unwrap();
        assert!(template.removed_tools.contains(&"str-replace-editor"));
        assert!(template.removed_tools.contains(&"launch-process")); // Can't execute
    }

    #[test]
    fn test_registry_no_custom_template() {
        let registry = AgentRegistry::new();
        // Custom agents don't have a template - they provide their own config
        assert!(registry.get(&AgentRole::Custom).is_none());
    }
}
```

### Task 3: Create Verifier and Explorer Prompts

Create `prompts/verifier.txt`:

```
You are a verification agent. Your job is to verify that work was completed correctly.

## Your Task
{task}

## User's Original Goal
{user_goal}

## Verification Process

1. **Review Requirements** - Read the task description and understand what should have been done
2. **Check Implementation** - Use view and codebase-retrieval to examine the changes
3. **Run Tests** - Use launch-process to run relevant tests:
   - `cargo test` for Rust projects
   - `npm test` for JavaScript/TypeScript
   - Or appropriate test command for the project
4. **Check for Regressions** - Ensure existing functionality still works
5. **Validate Code Quality** - Look for obvious issues, missing error handling, etc.

## What You CAN Do
- Read files with `view`
- Search code with `codebase-retrieval`
- Run tests with `launch-process`

## What You CANNOT Do
- Edit any files
- Make any changes to the codebase

## Completion

Call `complete()` when verification is done:
- `success=true` if all checks pass
- `success=false` if issues found (include specific details in the message)

Do NOT wait for user input. Do NOT ask questions. Verify and report.
```

Create `prompts/explorer.txt`:

```
You are an exploration agent. Your job is to research and gather information.

## Your Task
{task}

## User's Original Goal
{user_goal}

## Exploration Process

1. **Understand the Request** - What information is needed?
2. **Search the Codebase** - Use codebase-retrieval to find relevant code
3. **Examine Files** - Use view to read specific files
4. **Search the Web** - Use web-search and web-fetch for external information if needed
5. **Synthesize Findings** - Compile your discoveries into actionable insights

## What You CAN Do
- Read files with `view`
- Search code with `codebase-retrieval`
- Search the web with `web-search`
- Fetch web pages with `web-fetch`

## What You CANNOT Do
- Edit any files
- Run any commands
- Make any changes

## Completion

Call `complete()` with a summary of your findings:
- `success=true` with findings in the message
- `success=false` if you couldn't find the requested information

Your summary should be detailed enough for other agents to act on.

Do NOT wait for user input. Do NOT ask questions. Explore and report.
```

### Task 4: Update AgentSpec to Support Custom Agents

In `src/mcp_server/types.rs`, update `AgentSpec`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentSpec {
    /// The role of the agent (e.g., "implementer", "verifier", "explorer", "custom")
    pub role: String,
    /// The task to be performed by this agent
    pub task: String,
    /// Custom prompt (required for role="custom", optional for others)
    #[serde(default)]
    pub prompt: Option<String>,
    /// Explicit tool whitelist (required for role="custom")
    #[serde(default)]
    pub tools: Option<Vec<String>>,
}
```

### Task 5: Integrate AgentRegistry into App

In `src/main.rs`, add:
```rust
mod agents;
```

In `src/app/mod.rs`:
1. Add import: `use crate::agents::AgentRegistry;`
2. Add field: `pub(crate) agent_registry: AgentRegistry,`
3. Initialize in constructors: `agent_registry: AgentRegistry::new(),`

### Task 6: Update Agent Spawning to Use Templates

In `src/app/agent_spawner.rs`:

1. Import AgentRole and use the registry
2. When spawning an agent, determine the prompt and removed_tools based on role:

```rust
use crate::agents::AgentRole;

// In spawn_implementer or equivalent:
let (prompt, removed_tools) = match AgentRole::from_str(&role) {
    Some(AgentRole::Custom) => {
        // Custom: require prompt and tools from spec
        let prompt = spec.prompt.as_ref()
            .ok_or_else(|| anyhow!("Custom agent requires 'prompt'"))?
            .clone();
        let allowed_tools = spec.tools.as_ref()
            .ok_or_else(|| anyhow!("Custom agent requires 'tools' whitelist"))?;

        // Derive removed_tools from allowed_tools
        let all_tools = vec!["str-replace-editor", "save-file", "remove-files",
                             "launch-process", "kill-process", "read-process",
                             "write-process", "list-processes", "web-search", "web-fetch"];
        let removed: Vec<String> = all_tools.iter()
            .filter(|t| !allowed_tools.contains(&t.to_string()))
            .map(|s| s.to_string())
            .collect();

        (prompt, removed)
    }
    Some(role) => {
        // Template role: get from registry
        let template = self.agent_registry.get(&role)
            .ok_or_else(|| anyhow!("No template for role: {:?}", role))?;

        let prompt = template.prompt_template
            .replace("{task}", &spec.task)
            .replace("{user_goal}", &self.original_goal);
        let removed = template.removed_tools.iter().map(|s| s.to_string()).collect();

        (prompt, removed)
    }
    None => {
        // Unknown role - treat as implementer for backward compatibility
        tracing::warn!("Unknown agent role '{}', treating as implementer", role);
        let template = self.agent_registry.get(&AgentRole::Implementer).unwrap();
        let prompt = template.prompt_template
            .replace("{task}", &spec.task)
            .replace("{user_goal}", &self.original_goal);
        (prompt, vec![])
    }
};
```

### Task 7: Validate Custom Agents in spawn_agents Handler

In `src/app/orchestrator.rs`, add validation before spawning:

```rust
// In the SpawnAgents handler, before spawning:
for agent in &agents {
    if agent.role.to_lowercase() == "custom" {
        if agent.prompt.is_none() {
            let response = ToolResponse::failure(
                request.request_id.clone(),
                "Custom agent requires 'prompt' field".to_string(),
            );
            let _ = response_tx.send(response);
            continue; // Skip to next iteration or return error
        }
        if agent.tools.is_none() {
            let response = ToolResponse::failure(
                request.request_id.clone(),
                "Custom agent requires 'tools' whitelist".to_string(),
            );
            let _ = response_tx.send(response);
            continue;
        }
    }
}
```

### Task 8: Clean Up Duplicate Console Output

**In `src/app/session_drain.rs`**, find and REMOVE:
```rust
print!("{text}");
std::io::Write::flush(&mut std::io::stdout()).ok();
```

The `AgentWriter` already handles logging to files and emitting LogEvents.

**In `src/app/session.rs`**, find and REMOVE any duplicate `print!()` calls.

**In `src/app/orchestrator_acp.rs`**, find and REMOVE:
```rust
print!("{text}");
std::io::Write::flush(&mut std::io::stdout()).ok();
```

The orchestrator output should only go through AgentWriter.

### Task 9: Verify and Test

1. Run `cargo check` - must pass
2. Run `cargo test` - all tests must pass
3. Verify the new agent module tests pass

---

## Files to Create

| File | Purpose |
|------|---------|
| `src/agents/mod.rs` | AgentRole enum and module exports |
| `src/agents/templates.rs` | AgentRegistry and AgentTemplate |
| `prompts/verifier.txt` | Verifier agent prompt |
| `prompts/explorer.txt` | Explorer agent prompt |

## Files to Modify

| File | Changes |
|------|---------|
| `src/main.rs` | Add `mod agents;` |
| `src/mcp_server/types.rs` | Add `prompt` and `tools` fields to AgentSpec |
| `src/app/mod.rs` | Add AgentRegistry field |
| `src/app/agent_spawner.rs` | Use templates for agent spawning |
| `src/app/orchestrator.rs` | Validate custom agent fields |
| `src/app/session_drain.rs` | Remove print!() calls |
| `src/app/session.rs` | Remove print!() calls |
| `src/app/orchestrator_acp.rs` | Remove print!() calls |

---

## Testing

```bash
cargo check
cargo test
```

---

## Success Criteria

- [ ] `cargo check` passes
- [ ] `cargo test` passes
- [ ] AgentRole enum with from_str/as_str works
- [ ] AgentRegistry contains implementer, verifier, explorer templates
- [ ] Verifier template removes editing tools but keeps launch-process
- [ ] Explorer template removes editing AND execution tools
- [ ] AgentSpec supports optional `prompt` and `tools` fields
- [ ] Custom agents are validated (require prompt and tools)
- [ ] No duplicate print!() output (only AgentWriter)

---

## Important Notes

1. **Backward compatibility** - Unknown roles should fall back to implementer behavior with a warning
2. **Custom agents are advanced** - They require explicit configuration; no defaults
3. **Sequential execution** - Agents still run sequentially (concurrent mode is disabled); this is expected
4. **Keep existing prompts** - Don't modify implementer.txt; the new prompts are additive

---

## Rollback

```bash
git checkout .
```

