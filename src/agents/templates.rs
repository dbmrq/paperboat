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

