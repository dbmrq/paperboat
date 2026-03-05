//! Scenario loading for the mock testing system.
//!
//! This module provides the MockScenario type and related functionality
//! for loading test scenarios from TOML files.

use super::types::{AgentType, MockAcpResponse, MockAgentSession, MockToolCallResponse, MockToolType};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

// ============================================================================
// Scenario Loader
// ============================================================================

/// A complete test scenario loaded from TOML.
///
/// Defines all the mock sessions and tool responses needed to simulate
/// a complete interaction with the orchestrator system.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MockScenario {
    /// Scenario metadata.
    #[serde(default)]
    pub scenario: ScenarioMetadata,

    /// Planner sessions (produce plans from goals).
    #[serde(default)]
    pub planner_sessions: Vec<MockAgentSession>,

    /// Orchestrator sessions (coordinate task execution).
    #[serde(default)]
    pub orchestrator_sessions: Vec<MockAgentSession>,

    /// Implementer sessions (execute individual tasks).
    #[serde(default)]
    pub implementer_sessions: Vec<MockAgentSession>,

    /// Mock responses for MCP tool calls.
    #[serde(default)]
    pub mock_tool_responses: Vec<MockToolCallResponse>,

    /// Mock responses for ACP method calls.
    #[serde(default)]
    pub mock_acp_responses: Vec<MockAcpResponse>,
}

/// Metadata about a test scenario.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ScenarioMetadata {
    /// Name of the scenario.
    #[serde(default)]
    pub name: String,

    /// Description of what the scenario tests.
    #[serde(default)]
    pub description: String,
}

impl MockScenario {
    /// Load a scenario from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Load a scenario from a TOML string.
    pub fn parse(toml_content: &str) -> Result<Self> {
        let scenario: MockScenario = toml::from_str(toml_content)?;
        Ok(scenario)
    }

    /// Create an empty scenario.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Get all sessions for a specific agent type.
    pub fn sessions_for(&self, agent_type: AgentType) -> &[MockAgentSession] {
        match agent_type {
            AgentType::Planner => &self.planner_sessions,
            AgentType::Orchestrator => &self.orchestrator_sessions,
            AgentType::Implementer => &self.implementer_sessions,
        }
    }

    /// Find a mock tool response matching the given tool type and task.
    pub fn find_tool_response(
        &self,
        tool_type: MockToolType,
        task: &str,
    ) -> Option<&MockToolCallResponse> {
        self.mock_tool_responses.iter().find(|r| {
            if r.tool_type != tool_type {
                return false;
            }
            match &r.task_pattern {
                Some(pattern) => {
                    regex::Regex::new(pattern)
                        .map(|re| re.is_match(task))
                        .unwrap_or(false)
                }
                None => true, // No pattern means match all
            }
        })
    }
}

