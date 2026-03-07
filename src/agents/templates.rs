//! Built-in agent templates.
//!
//! Templates are auto-discovered from the `prompts/` directory at compile time.
//! Tool restrictions are configured in `config.rs` - new roles default to implementer permissions.

use super::config::{get_tool_config, AgentToolConfig, IMPLEMENTER_CONFIG};
use super::{get_prompt, AgentRole, SPAWNABLE_ROLES};

/// Template defining an agent's prompt and tool restrictions.
pub struct AgentTemplate {
    /// The prompt template with {task} and {user_goal}/{context} placeholders.
    pub prompt_template: &'static str,
    /// Tools to remove from this agent type.
    pub removed_tools: Vec<&'static str>,
}

/// Registry of agent templates (auto-discovered from prompts/).
pub struct AgentRegistry {
    // No internal storage needed - we look up dynamically
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {}
    }

    /// Get the template for a role.
    ///
    /// Templates are auto-discovered from `prompts/` directory.
    /// Tool restrictions come from `config.rs` (defaults to implementer if not configured).
    pub fn get(&self, role: &AgentRole) -> Option<AgentTemplate> {
        let role_str = role.as_str();

        // Custom agents don't have templates - they provide their own
        if matches!(role, AgentRole::Custom) {
            return None;
        }

        // Get prompt from auto-discovered prompts
        let prompt = get_prompt(role_str)?;

        // Get tool config (falls back to implementer config for unknown roles)
        let config = get_tool_config(role_str);

        Some(AgentTemplate {
            prompt_template: prompt,
            removed_tools: config.removed_auggie_tools.to_vec(),
        })
    }

    /// Check if a role has a template.
    #[allow(dead_code)]
    pub fn has_role(&self, role: &AgentRole) -> bool {
        if matches!(role, AgentRole::Custom) {
            return false;
        }
        SPAWNABLE_ROLES.contains(&role.as_str())
    }

    /// Get all available spawnable role names.
    #[allow(dead_code)]
    pub fn available_roles(&self) -> &'static [&'static str] {
        SPAWNABLE_ROLES
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
        // Implementer can edit files and run processes
        assert!(!template.removed_tools.contains(&"str-replace-editor"));
        assert!(!template.removed_tools.contains(&"save-file"));
        assert!(!template.removed_tools.contains(&"launch-process"));
        // But sub-agents are removed for all agent types
        assert!(template.removed_tools.contains(&"sub-agent-explore"));
        assert!(template.removed_tools.contains(&"sub-agent-plan"));
    }

    #[test]
    fn test_registry_has_verifier() {
        let registry = AgentRegistry::new();
        let template = registry.get(&AgentRole::Verifier).unwrap();
        // Verifier cannot edit files
        assert!(template.removed_tools.contains(&"str-replace-editor"));
        assert!(template.removed_tools.contains(&"save-file"));
        // But verifier CAN run processes (for tests)
        assert!(!template.removed_tools.contains(&"launch-process"));
    }

    #[test]
    fn test_registry_has_explorer() {
        let registry = AgentRegistry::new();
        let template = registry.get(&AgentRole::Explorer).unwrap();
        // Explorer cannot edit files
        assert!(template.removed_tools.contains(&"str-replace-editor"));
        assert!(template.removed_tools.contains(&"save-file"));
        // Explorer cannot execute processes
        assert!(template.removed_tools.contains(&"launch-process"));
    }

    #[test]
    fn test_registry_no_custom_template() {
        let registry = AgentRegistry::new();
        // Custom agents don't have a template - they provide their own config
        assert!(registry.get(&AgentRole::Custom).is_none());
    }

    #[test]
    fn test_registry_lists_available_roles() {
        let registry = AgentRegistry::new();
        let roles = registry.available_roles();
        assert!(roles.contains(&"implementer"));
        assert!(roles.contains(&"verifier"));
        assert!(roles.contains(&"explorer"));
    }
}

