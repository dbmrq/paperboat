//! Built-in agent templates.

use super::config::{EXPLORER_CONFIG, IMPLEMENTER_CONFIG, VERIFIER_CONFIG};
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

        // Use centralized config from agents::config for tool restrictions
        templates.insert(AgentRole::Implementer, AgentTemplate {
            prompt_template: include_str!("../../prompts/implementer.txt"),
            removed_tools: IMPLEMENTER_CONFIG.removed_auggie_tools.to_vec(),
        });

        templates.insert(AgentRole::Verifier, AgentTemplate {
            prompt_template: include_str!("../../prompts/verifier.txt"),
            removed_tools: VERIFIER_CONFIG.removed_auggie_tools.to_vec(),
        });

        templates.insert(AgentRole::Explorer, AgentTemplate {
            prompt_template: include_str!("../../prompts/explorer.txt"),
            removed_tools: EXPLORER_CONFIG.removed_auggie_tools.to_vec(),
        });

        Self { templates }
    }

    pub fn get(&self, role: &AgentRole) -> Option<&AgentTemplate> {
        self.templates.get(role)
    }

    #[allow(dead_code)]
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
}

