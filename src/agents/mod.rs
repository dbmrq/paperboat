//! Agent templates and registry.
//!
//! Agent roles are auto-discovered from the `prompts/` directory at compile time.
//! To add a new agent type, create `prompts/newrole.txt` and rebuild.

pub mod config;
mod templates;

#[cfg(test)]
pub use config::IMPLEMENTER_CONFIG;
pub use config::{get_tool_config, ORCHESTRATOR_CONFIG, PLANNER_CONFIG};
pub use templates::AgentRegistry;

// Include the auto-generated roles module
mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated_roles.rs"));
}

pub use generated::{get_prompt, SPAWNABLE_ROLES};

/// Agent roles that can be spawned.
///
/// Note: The enum variants are kept for type safety in internal code,
/// but the actual list of spawnable roles is auto-discovered from `prompts/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentRole {
    /// Standard implementer - can edit files and run processes
    Implementer,
    /// Verifier - read-only but can run tests
    Verifier,
    /// Explorer - read-only, for context gathering
    Explorer,
    /// Custom agent with user-defined prompt and tools
    Custom,
    /// Dynamic role loaded from prompts/ directory
    Dynamic(String),
}

impl AgentRole {
    /// Parse a role from string. Returns Dynamic for unknown roles that have prompts.
    pub fn from_str(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        match lower.as_str() {
            "implementer" => Some(Self::Implementer),
            "verifier" => Some(Self::Verifier),
            "explorer" => Some(Self::Explorer),
            "custom" => Some(Self::Custom),
            _ => {
                // Check if it's a valid dynamic role (has a prompt file)
                if SPAWNABLE_ROLES.contains(&lower.as_str()) {
                    Some(Self::Dynamic(lower))
                } else {
                    None
                }
            }
        }
    }

    /// Get the string representation of this role.
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Implementer => "implementer",
            Self::Verifier => "verifier",
            Self::Explorer => "explorer",
            Self::Custom => "custom",
            Self::Dynamic(name) => name.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_role_from_str() {
        assert_eq!(
            AgentRole::from_str("implementer"),
            Some(AgentRole::Implementer)
        );
        assert_eq!(AgentRole::from_str("VERIFIER"), Some(AgentRole::Verifier));
        assert_eq!(AgentRole::from_str("Explorer"), Some(AgentRole::Explorer));
        assert_eq!(AgentRole::from_str("custom"), Some(AgentRole::Custom));
    }

    #[test]
    fn test_agent_role_as_str() {
        assert_eq!(AgentRole::Implementer.as_str(), "implementer");
        assert_eq!(AgentRole::Verifier.as_str(), "verifier");
    }

    #[test]
    fn test_spawnable_roles_discovered() {
        // Should discover at least implementer, verifier, explorer, selfimprover from prompts/
        assert!(SPAWNABLE_ROLES.contains(&"implementer"));
        assert!(SPAWNABLE_ROLES.contains(&"verifier"));
        assert!(SPAWNABLE_ROLES.contains(&"explorer"));
        assert!(SPAWNABLE_ROLES.contains(&"selfimprover"));
        // orchestrator and planner should NOT be in spawnable roles
        assert!(!SPAWNABLE_ROLES.contains(&"orchestrator"));
        assert!(!SPAWNABLE_ROLES.contains(&"planner"));
    }

    #[test]
    fn test_get_prompt_returns_content() {
        // Should be able to get prompts for discovered roles
        assert!(get_prompt("implementer").is_some());
        assert!(get_prompt("verifier").is_some());
        assert!(get_prompt("explorer").is_some());
        assert!(get_prompt("selfimprover").is_some());
        // Unknown role returns None
        assert!(get_prompt("nonexistent").is_none());
    }

    #[test]
    fn test_selfimprover_role_uses_dynamic() {
        // selfimprover should be parsed as a Dynamic role
        let role = AgentRole::from_str("selfimprover");
        assert_eq!(role, Some(AgentRole::Dynamic("selfimprover".to_string())));
    }
}
