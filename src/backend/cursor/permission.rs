//! Permission policy for tool access control.
//!
//! This module provides `PermissionPolicy`, which controls which tools an agent
//! can use. It is used by both ACP and CLI transports to enforce consistent
//! tool access rules across different agent types.

use std::collections::HashSet;

use crate::agents::{AgentToolConfig, IMPLEMENTER_CONFIG, ORCHESTRATOR_CONFIG, PLANNER_CONFIG};

/// All MCP tool names that Paperboat exposes.
const ALL_MCP_TOOLS: &[&str] = &[
    "set_goal",
    "create_task",
    "complete",
    "decompose",
    "spawn_agents",
    "skip_tasks",
    "list_tasks",
];

/// Permission policy for tool access control.
///
/// This controls which tools an agent can use. When the backend receives a
/// permission request for a tool, it checks this policy to decide whether
/// to allow or deny the tool call.
///
/// # Design
///
/// - The policy is immutable after creation
/// - Tool names are matched exactly (case-sensitive)
/// - MCP tools (Paperboat's own tools) are checked against `allowed_mcp_tools`
/// - Backend/built-in tools are checked against `denied_tools`
///
/// # Example
///
/// ```ignore
/// let policy = PermissionPolicy::for_orchestrator();
/// assert!(!policy.should_allow("edit_file")); // File editing denied
/// assert!(policy.should_allow("list_tasks")); // MCP tools allowed
/// ```
#[derive(Debug, Clone, Default)]
pub struct PermissionPolicy {
    /// Tools that should always be rejected (backend/built-in tools).
    pub denied_tools: HashSet<String>,
    /// MCP tools that are allowed (all others are denied).
    pub allowed_mcp_tools: HashSet<String>,
}

impl PermissionPolicy {
    /// Create a policy that allows all tools.
    pub fn allow_all() -> Self {
        Self {
            denied_tools: HashSet::new(),
            allowed_mcp_tools: ALL_MCP_TOOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// Create a policy from an agent tool configuration.
    ///
    /// This uses the centralized config from `src/agents/config.rs` to determine
    /// which tools are allowed/denied for each agent type.
    pub fn from_agent_config(config: &AgentToolConfig) -> Self {
        Self {
            // Deny the tools listed in removed_auggie_tools
            denied_tools: config
                .removed_auggie_tools
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            // Only allow the MCP tools listed for this agent type
            allowed_mcp_tools: config.mcp_tools.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// Create a policy for planner agents.
    ///
    /// Denies file editing tools but allows MCP tools for planning.
    #[allow(dead_code)]
    pub fn for_planner() -> Self {
        Self::from_agent_config(&PLANNER_CONFIG)
    }

    /// Create a policy for orchestrator agents.
    ///
    /// Denies file editing tools but allows MCP tools for orchestration.
    #[allow(dead_code)]
    pub fn for_orchestrator() -> Self {
        Self::from_agent_config(&ORCHESTRATOR_CONFIG)
    }

    /// Create a policy for implementer agents.
    ///
    /// Allows all tools including file editing.
    #[allow(dead_code)]
    pub fn for_implementer() -> Self {
        Self::from_agent_config(&IMPLEMENTER_CONFIG)
    }

    /// Check if a tool should be allowed.
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The name of the tool to check
    ///
    /// # Returns
    ///
    /// `true` if the tool should be allowed, `false` otherwise.
    pub fn should_allow(&self, tool_name: &str) -> bool {
        // Check if it's an MCP tool
        if ALL_MCP_TOOLS.contains(&tool_name) {
            return self.allowed_mcp_tools.contains(tool_name);
        }

        // For backend tools, check if it's denied
        !self.denied_tools.contains(tool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Allow All Policy Tests
    // ========================================================================

    #[test]
    fn test_allow_all_allows_mcp_tools() {
        let policy = PermissionPolicy::allow_all();
        assert!(policy.should_allow("list_tasks"));
        assert!(policy.should_allow("spawn_agents"));
        assert!(policy.should_allow("complete"));
    }

    #[test]
    fn test_allow_all_allows_all_mcp_tools() {
        let policy = PermissionPolicy::allow_all();
        // Should allow all MCP tools
        for tool in ALL_MCP_TOOLS {
            assert!(policy.should_allow(tool), "Expected {} to be allowed", tool);
        }
    }

    #[test]
    fn test_allow_all_allows_backend_tools() {
        let policy = PermissionPolicy::allow_all();
        // Backend tools like str-replace-editor, save-file, launch-process
        assert!(policy.should_allow("str-replace-editor"));
        assert!(policy.should_allow("save-file"));
        assert!(policy.should_allow("launch-process"));
    }

    #[test]
    fn test_allow_all_has_empty_denied_tools() {
        let policy = PermissionPolicy::allow_all();
        assert!(policy.denied_tools.is_empty());
    }

    #[test]
    fn test_allow_all_has_all_mcp_tools() {
        let policy = PermissionPolicy::allow_all();
        assert_eq!(policy.allowed_mcp_tools.len(), ALL_MCP_TOOLS.len());
    }

    // ========================================================================
    // Orchestrator Policy Tests
    // ========================================================================

    #[test]
    fn test_orchestrator_denies_file_editing() {
        let policy = PermissionPolicy::for_orchestrator();
        // File editing tools should be denied (using actual tool names)
        assert!(!policy.should_allow("str-replace-editor"));
        assert!(!policy.should_allow("save-file"));
        assert!(!policy.should_allow("remove-files"));
    }

    #[test]
    fn test_orchestrator_allows_mcp_tools() {
        let policy = PermissionPolicy::for_orchestrator();
        // Orchestrator-specific MCP tools should be allowed
        // Note: "set_goal" is not in orchestrator's mcp_tools, but spawn_agents is
        assert!(policy.should_allow("spawn_agents"));
        assert!(policy.should_allow("decompose"));
        assert!(policy.should_allow("complete"));
    }

    #[test]
    fn test_orchestrator_allows_list_tasks() {
        let policy = PermissionPolicy::for_orchestrator();
        assert!(policy.should_allow("list_tasks"));
    }

    #[test]
    fn test_orchestrator_allows_skip_tasks() {
        let policy = PermissionPolicy::for_orchestrator();
        assert!(policy.should_allow("skip_tasks"));
    }

    #[test]
    fn test_orchestrator_allows_create_task() {
        let policy = PermissionPolicy::for_orchestrator();
        // Orchestrator can now create tasks dynamically (e.g., from agent suggestions)
        assert!(policy.should_allow("create_task"));
    }

    #[test]
    fn test_orchestrator_denies_set_goal() {
        let policy = PermissionPolicy::for_orchestrator();
        // set_goal is a planner tool, not orchestrator
        assert!(!policy.should_allow("set_goal"));
    }

    // ========================================================================
    // Planner Policy Tests
    // ========================================================================

    #[test]
    fn test_planner_denies_file_editing() {
        let policy = PermissionPolicy::for_planner();
        // File editing tools should be denied
        assert!(!policy.should_allow("str-replace-editor"));
        assert!(!policy.should_allow("save-file"));
        // Process execution also denied
        assert!(!policy.should_allow("launch-process"));
    }

    #[test]
    fn test_planner_allows_mcp_tools() {
        let policy = PermissionPolicy::for_planner();
        // Planner MCP tools
        assert!(policy.should_allow("set_goal"));
        assert!(policy.should_allow("create_task"));
        assert!(policy.should_allow("complete"));
    }

    #[test]
    fn test_planner_denies_spawn_agents() {
        let policy = PermissionPolicy::for_planner();
        // spawn_agents is an orchestrator tool, not planner
        assert!(!policy.should_allow("spawn_agents"));
    }

    #[test]
    fn test_planner_denies_decompose() {
        let policy = PermissionPolicy::for_planner();
        // decompose is an orchestrator tool, not planner
        assert!(!policy.should_allow("decompose"));
    }

    #[test]
    fn test_planner_denies_remove_files() {
        let policy = PermissionPolicy::for_planner();
        assert!(!policy.should_allow("remove-files"));
    }

    // ========================================================================
    // Implementer Policy Tests
    // ========================================================================

    #[test]
    fn test_implementer_allows_backend_tools() {
        let policy = PermissionPolicy::for_implementer();
        // Implementer can edit files
        assert!(policy.should_allow("str-replace-editor"));
        assert!(policy.should_allow("save-file"));
        assert!(policy.should_allow("launch-process"));
    }

    #[test]
    fn test_implementer_allows_mcp_complete() {
        let policy = PermissionPolicy::for_implementer();
        // Implementer only has "complete" MCP tool
        assert!(policy.should_allow("complete"));
        // But not orchestrator tools (they're not in allowed_mcp_tools)
        assert!(!policy.should_allow("spawn_agents"));
    }

    #[test]
    fn test_implementer_allows_remove_files() {
        let policy = PermissionPolicy::for_implementer();
        assert!(policy.should_allow("remove-files"));
    }

    #[test]
    fn test_implementer_allows_view() {
        let policy = PermissionPolicy::for_implementer();
        assert!(policy.should_allow("view"));
    }

    #[test]
    fn test_implementer_denies_spawn_agents() {
        let policy = PermissionPolicy::for_implementer();
        assert!(!policy.should_allow("spawn_agents"));
    }

    #[test]
    fn test_implementer_denies_set_goal() {
        let policy = PermissionPolicy::for_implementer();
        assert!(!policy.should_allow("set_goal"));
    }

    #[test]
    fn test_implementer_denies_create_task() {
        let policy = PermissionPolicy::for_implementer();
        assert!(!policy.should_allow("create_task"));
    }

    // ========================================================================
    // Policy Trait and Structure Tests
    // ========================================================================

    #[test]
    fn test_permission_policy_is_clone() {
        let policy = PermissionPolicy::for_orchestrator();
        let cloned = policy.clone();
        assert_eq!(policy.denied_tools, cloned.denied_tools);
        assert_eq!(policy.allowed_mcp_tools, cloned.allowed_mcp_tools);
    }

    #[test]
    fn test_permission_policy_is_debug() {
        let policy = PermissionPolicy::for_planner();
        let debug_str = format!("{:?}", policy);
        assert!(debug_str.contains("PermissionPolicy"));
    }

    #[test]
    fn test_permission_policy_default() {
        let policy = PermissionPolicy::default();
        // Default should have empty sets
        assert!(policy.denied_tools.is_empty());
        assert!(policy.allowed_mcp_tools.is_empty());
    }

    // ========================================================================
    // Edge Cases
    // ========================================================================

    #[test]
    fn test_unknown_tool_not_denied_allowed() {
        // Unknown tools that aren't MCP tools should be allowed (not in denied list)
        let policy = PermissionPolicy::for_orchestrator();
        // A hypothetical tool not in any list
        assert!(policy.should_allow("some_random_tool"));
    }

    #[test]
    fn test_mcp_tool_not_in_allowed_denied() {
        let policy = PermissionPolicy::for_implementer();
        // An MCP tool that's not in the implementer's allowed list
        assert!(!policy.should_allow("decompose"));
    }

    #[test]
    fn test_case_sensitive_mcp_tool_matching() {
        let policy = PermissionPolicy::for_orchestrator();
        // MCP tool names should be case-sensitive when checked against allowed_mcp_tools
        assert!(policy.should_allow("spawn_agents"));
        // Different case versions aren't in the MCP tools list
        // But since they're not recognized as MCP tools, they fall through to
        // the denied_tools check. Since "Spawn_Agents" isn't in denied_tools, it's allowed.
        // This test verifies the matching logic is case-sensitive for MCP tools.
        assert!(policy.allowed_mcp_tools.contains("spawn_agents"));
        assert!(!policy.allowed_mcp_tools.contains("Spawn_Agents"));
        assert!(!policy.allowed_mcp_tools.contains("SPAWN_AGENTS"));
    }

    #[test]
    fn test_empty_tool_name() {
        let policy = PermissionPolicy::allow_all();
        // Empty string is not an MCP tool and not in denied list
        assert!(policy.should_allow(""));
    }

    #[test]
    fn test_tool_with_special_characters() {
        let policy = PermissionPolicy::for_implementer();
        // Tools with dashes are common
        assert!(policy.should_allow("str-replace-editor"));
    }
}
