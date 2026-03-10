//! Centralized agent tool configuration.
//!
//! This module defines which tools each agent type has access to, for both:
//! - **MCP tools**: Our orchestration tools (`spawn_agents`, complete, `create_task`, etc.)
//! - **Backend tools**: The agent's built-in tools (str-replace-editor, save-file, etc.)
//!
//! Tool access is controlled via:
//! - `PAPERBOAT_AGENT_TYPE` env var: Controls which MCP tools are exposed
//! - `PAPERBOAT_REMOVED_TOOLS` env var: Controls which backend tools are removed
//!
//! Note: The `removed_auggie_tools` field name is historical but applies to all backends.

/// All auggie tools that can be filtered (used in tests).
/// These are the tools provided by the Augment platform.
#[cfg(test)]
const ALL_AUGGIE_TOOLS: &[&str] = &[
    // File editing
    "str-replace-editor",
    "save-file",
    "remove-files",
    "apply_patch",
    // Code search and view
    "view",
    "codebase-retrieval",
    // Process management
    "launch-process",
    "kill-process",
    "read-process",
    "write-process",
    "list-processes",
    // Web tools
    "web-search",
    "web-fetch",
    // Task management
    "view_tasklist",
    "reorganize_tasklist",
    "update_tasks",
    "add_tasks",
    // Sub-agents
    "sub-agent-explore",
    "sub-agent-plan",
];

/// Configuration for a specific agent type's tool access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentToolConfig {
    /// MCP tools this agent can use.
    pub mcp_tools: &'static [&'static str],
    /// Backend tools to REMOVE from this agent (tools it cannot use).
    /// Named `removed_auggie_tools` for historical reasons, but applies to all backends.
    pub removed_auggie_tools: &'static [&'static str],
}

impl AgentToolConfig {
    /// Get the auggie tools this agent is allowed to use (test helper).
    #[cfg(test)]
    fn allowed_auggie_tools(&self) -> Vec<&'static str> {
        ALL_AUGGIE_TOOLS
            .iter()
            .filter(|t| !self.removed_auggie_tools.contains(t))
            .copied()
            .collect()
    }
}

// ============================================================================
// Agent Tool Configurations
// ============================================================================

/// Planner: Creates plans from goals. Read-only access, no editing or execution.
pub const PLANNER_CONFIG: AgentToolConfig = AgentToolConfig {
    mcp_tools: &["set_goal", "create_task", "complete"],
    removed_auggie_tools: &[
        // No file editing - planner only plans
        "str-replace-editor",
        "save-file",
        "remove-files",
        "apply_patch",
        // No process execution - planner only plans
        "launch-process",
        "kill-process",
        "read-process",
        "write-process",
        "list-processes",
        // No built-in task management - we provide our own
        "view_tasklist",
        "reorganize_tasklist",
        "update_tasks",
        "add_tasks",
        // No sub-agents - we provide our own
        "sub-agent-explore",
        "sub-agent-plan",
    ],
};

/// Orchestrator: Coordinates task execution. No direct editing or execution.
pub const ORCHESTRATOR_CONFIG: AgentToolConfig = AgentToolConfig {
    mcp_tools: &[
        "decompose",
        "spawn_agents",
        "complete",
        "create_task",
        "skip_tasks",
        "list_tasks",
        "report_human_action",
    ],
    removed_auggie_tools: &[
        // No file editing - orchestrator delegates to implementers
        "str-replace-editor",
        "save-file",
        "remove-files",
        "apply_patch",
        // No process execution - orchestrator delegates
        "launch-process",
        "kill-process",
        "read-process",
        "write-process",
        "list-processes",
        // No web tools - orchestrator delegates
        "web-search",
        "web-fetch",
        // No built-in task management - we provide our own via MCP tools
        // This prevents confusion from Augment's internal task IDs (which use a different format)
        "view_tasklist",
        "reorganize_tasklist",
        "update_tasks",
        "add_tasks",
        // No built-in sub-agents - we provide our own
        "sub-agent-explore",
        "sub-agent-plan",
    ],
};

/// Implementer: Executes tasks. Full access to editing and execution tools.
pub const IMPLEMENTER_CONFIG: AgentToolConfig = AgentToolConfig {
    mcp_tools: &["complete"],
    removed_auggie_tools: &[
        // No built-in sub-agents - implementers complete their own tasks
        "sub-agent-explore",
        "sub-agent-plan",
    ],
};

/// Verifier: Validates implementations. Read-only + can run tests.
pub const VERIFIER_CONFIG: AgentToolConfig = AgentToolConfig {
    mcp_tools: &["complete"],
    removed_auggie_tools: &[
        // No file editing - verifiers only verify
        "str-replace-editor",
        "save-file",
        "remove-files",
        "apply_patch",
        // No built-in sub-agents
        "sub-agent-explore",
        "sub-agent-plan",
    ],
};

/// Explorer: Gathers information. Read-only, no editing or execution.
pub const EXPLORER_CONFIG: AgentToolConfig = AgentToolConfig {
    mcp_tools: &["complete"],
    removed_auggie_tools: &[
        // No file editing
        "str-replace-editor",
        "save-file",
        "remove-files",
        "apply_patch",
        // No process execution
        "launch-process",
        "kill-process",
        "read-process",
        "write-process",
        "list-processes",
        // No built-in sub-agents
        "sub-agent-explore",
        "sub-agent-plan",
    ],
};

// ============================================================================
// Helper Functions
// ============================================================================

/// Get the tool configuration for a spawnable agent role.
///
/// Used by the agent registry to determine `removed_auggie_tools`.
/// Unknown roles default to implementer (full capabilities).
pub fn get_tool_config(role: &str) -> &'static AgentToolConfig {
    match role {
        "verifier" => &VERIFIER_CONFIG,
        "explorer" => &EXPLORER_CONFIG,
        // "implementer" and new roles auto-discovered from prompts/ default to implementer capabilities
        _ => &IMPLEMENTER_CONFIG,
    }
}

/// Get the tool configuration for an agent type (test helper).
#[cfg(test)]
fn get_config(agent_type: &str) -> &'static AgentToolConfig {
    match agent_type {
        "planner" => &PLANNER_CONFIG,
        "implementer" => &IMPLEMENTER_CONFIG,
        "verifier" => &VERIFIER_CONFIG,
        "explorer" => &EXPLORER_CONFIG,
        // "orchestrator" and unknown types default to orchestrator (most restrictive coordinator role)
        _ => &ORCHESTRATOR_CONFIG,
    }
}

/// Format removed tools as a comma-separated string (test helper).
#[cfg(test)]
fn format_removed_tools(config: &AgentToolConfig) -> String {
    config.removed_auggie_tools.join(",")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planner_cannot_edit_files() {
        let allowed = PLANNER_CONFIG.allowed_auggie_tools();

        // Planner should NOT have editing tools
        assert!(
            !allowed.contains(&"str-replace-editor"),
            "Planner should not have str-replace-editor"
        );
        assert!(
            !allowed.contains(&"save-file"),
            "Planner should not have save-file"
        );
        assert!(
            !allowed.contains(&"remove-files"),
            "Planner should not have remove-files"
        );
        assert!(
            !allowed.contains(&"apply_patch"),
            "Planner should not have apply_patch"
        );

        // Planner SHOULD have read-only tools
        assert!(allowed.contains(&"view"), "Planner should have view");
        assert!(
            allowed.contains(&"codebase-retrieval"),
            "Planner should have codebase-retrieval"
        );
        assert!(
            allowed.contains(&"web-search"),
            "Planner should have web-search"
        );
    }

    #[test]
    fn test_planner_cannot_execute_processes() {
        let allowed = PLANNER_CONFIG.allowed_auggie_tools();

        assert!(
            !allowed.contains(&"launch-process"),
            "Planner should not have launch-process"
        );
        assert!(
            !allowed.contains(&"kill-process"),
            "Planner should not have kill-process"
        );
    }

    #[test]
    fn test_planner_mcp_tools() {
        assert!(PLANNER_CONFIG.mcp_tools.contains(&"create_task"));
        assert!(PLANNER_CONFIG.mcp_tools.contains(&"complete"));

        // Planner should NOT have orchestrator tools
        assert!(!PLANNER_CONFIG.mcp_tools.contains(&"spawn_agents"));
        assert!(!PLANNER_CONFIG.mcp_tools.contains(&"decompose"));
        assert!(!PLANNER_CONFIG.mcp_tools.contains(&"skip_tasks"));
    }

    #[test]
    fn test_orchestrator_cannot_edit_files() {
        let allowed = ORCHESTRATOR_CONFIG.allowed_auggie_tools();

        assert!(
            !allowed.contains(&"str-replace-editor"),
            "Orchestrator should not have str-replace-editor"
        );
        assert!(
            !allowed.contains(&"save-file"),
            "Orchestrator should not have save-file"
        );
    }

    #[test]
    fn test_orchestrator_mcp_tools() {
        assert!(ORCHESTRATOR_CONFIG.mcp_tools.contains(&"spawn_agents"));
        assert!(ORCHESTRATOR_CONFIG.mcp_tools.contains(&"decompose"));
        assert!(ORCHESTRATOR_CONFIG.mcp_tools.contains(&"complete"));
        // Orchestrator can now create tasks dynamically (e.g., from agent suggestions)
        assert!(ORCHESTRATOR_CONFIG.mcp_tools.contains(&"create_task"));
        // Orchestrator can skip tasks when appropriate
        assert!(ORCHESTRATOR_CONFIG.mcp_tools.contains(&"skip_tasks"));
    }

    #[test]
    fn test_implementer_can_edit_files() {
        let allowed = IMPLEMENTER_CONFIG.allowed_auggie_tools();

        // Implementer SHOULD have editing tools
        assert!(
            allowed.contains(&"str-replace-editor"),
            "Implementer should have str-replace-editor"
        );
        assert!(
            allowed.contains(&"save-file"),
            "Implementer should have save-file"
        );
        assert!(
            allowed.contains(&"launch-process"),
            "Implementer should have launch-process"
        );
    }

    #[test]
    fn test_implementer_mcp_tools() {
        // Implementer only gets complete
        assert_eq!(IMPLEMENTER_CONFIG.mcp_tools, &["complete"]);

        // Implementer should NOT have orchestrator tools
        assert!(!IMPLEMENTER_CONFIG.mcp_tools.contains(&"skip_tasks"));
    }

    #[test]
    fn test_verifier_cannot_edit_but_can_run() {
        let allowed = VERIFIER_CONFIG.allowed_auggie_tools();

        // Verifier should NOT edit
        assert!(!allowed.contains(&"str-replace-editor"));
        assert!(!allowed.contains(&"save-file"));

        // Verifier SHOULD run processes (for tests)
        assert!(allowed.contains(&"launch-process"));
    }

    #[test]
    fn test_verifier_mcp_tools() {
        // Verifier only gets complete
        assert_eq!(VERIFIER_CONFIG.mcp_tools, &["complete"]);

        // Verifier should NOT have orchestrator tools
        assert!(!VERIFIER_CONFIG.mcp_tools.contains(&"skip_tasks"));
    }

    #[test]
    fn test_explorer_is_read_only() {
        let allowed = EXPLORER_CONFIG.allowed_auggie_tools();

        // Explorer should NOT edit
        assert!(!allowed.contains(&"str-replace-editor"));
        assert!(!allowed.contains(&"save-file"));

        // Explorer should NOT execute
        assert!(!allowed.contains(&"launch-process"));

        // Explorer SHOULD read
        assert!(allowed.contains(&"view"));
        assert!(allowed.contains(&"codebase-retrieval"));
        assert!(allowed.contains(&"web-search"));
    }

    #[test]
    fn test_get_config_by_type() {
        assert_eq!(get_config("planner"), &PLANNER_CONFIG);
        assert_eq!(get_config("orchestrator"), &ORCHESTRATOR_CONFIG);
        assert_eq!(get_config("implementer"), &IMPLEMENTER_CONFIG);
        assert_eq!(get_config("verifier"), &VERIFIER_CONFIG);
        assert_eq!(get_config("explorer"), &EXPLORER_CONFIG);

        // Unknown defaults to orchestrator
        assert_eq!(get_config("unknown"), &ORCHESTRATOR_CONFIG);
    }

    #[test]
    fn test_format_removed_tools() {
        let formatted = format_removed_tools(&IMPLEMENTER_CONFIG);
        assert!(formatted.contains("sub-agent-explore"));
        assert!(formatted.contains("sub-agent-plan"));
        assert!(!formatted.contains("str-replace-editor")); // Implementer keeps this
    }

    #[test]
    fn test_no_agent_has_builtin_subagents() {
        // All agents should have built-in sub-agents removed
        // (we provide our own spawn_agents for orchestrators)
        for config in [
            &PLANNER_CONFIG,
            &ORCHESTRATOR_CONFIG,
            &IMPLEMENTER_CONFIG,
            &VERIFIER_CONFIG,
            &EXPLORER_CONFIG,
        ] {
            assert!(
                config.removed_auggie_tools.contains(&"sub-agent-explore"),
                "All agents should have sub-agent-explore removed"
            );
            assert!(
                config.removed_auggie_tools.contains(&"sub-agent-plan"),
                "All agents should have sub-agent-plan removed"
            );
        }
    }
}
