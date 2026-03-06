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

