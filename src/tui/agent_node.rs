//! Agent node types for the TUI tree.
//!
//! This module contains [`AgentNode`] and [`AgentStatus`] which represent
//! individual agents in the hierarchical tree structure displayed in the TUI.

use std::time::Instant;

use crate::logging::AgentType;

// ============================================================================
// Agent Status
// ============================================================================

/// Status of an agent in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentStatus {
    /// Agent is currently running
    #[default]
    Running,
    /// Agent completed successfully
    Completed,
    /// Agent failed
    Failed,
}

// ============================================================================
// Agent Node (Tree Data)
// ============================================================================

/// Metadata about an agent in the tree.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some fields are stored for future UI features
pub struct AgentNode {
    /// The session ID (unique identifier)
    pub session_id: String,
    /// Type of agent (Orchestrator, Planner, Implementer)
    pub agent_type: AgentType,
    /// Depth in the hierarchy (0 = root)
    pub depth: u32,
    /// The task this agent is working on
    pub task: String,
    /// When the agent started
    pub start_time: Instant,
    /// Current status
    pub status: AgentStatus,
    /// Children session IDs (for building tree)
    pub children: Vec<String>,
    /// Parent session ID (None for root)
    pub parent_session_id: Option<String>,
}

impl AgentNode {
    /// Creates a new agent node from an `AgentStarted` event.
    #[must_use]
    pub fn new(agent_type: AgentType, session_id: String, depth: u32, task: String) -> Self {
        Self {
            session_id,
            agent_type,
            depth,
            task,
            start_time: Instant::now(),
            status: AgentStatus::Running,
            children: Vec::new(),
            parent_session_id: None,
        }
    }

    /// Returns a display name for this agent.
    ///
    /// For running agents, the status icon alternates between "~" and "-" based on
    /// the animation frame to create a smooth animated indicator.
    #[must_use]
    pub fn display_name(&self, animation_frame: u32) -> String {
        let status_icon = match self.status {
            AgentStatus::Running => {
                // Alternate between "~" and "-" every ~500ms at 60 FPS
                if animation_frame % 60 < 30 {
                    "~"
                } else {
                    "-"
                }
            }
            AgentStatus::Completed => "✓",
            AgentStatus::Failed => "✗",
        };
        format!("{} {}", status_icon, self.agent_type.name())
    }
}
