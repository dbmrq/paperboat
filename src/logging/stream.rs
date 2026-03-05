//! Log event types for streaming and observation.

use super::writer::AgentType;

/// Events broadcast for UI streaming and observation.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// Agent started a new session.
    AgentStarted {
        agent_type: AgentType,
        session_id: String,
        depth: u32,
        task: String,
    },

    /// Agent sent a message chunk (streaming text).
    AgentMessage {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        content: String,
    },

    /// Agent made a tool call.
    ToolCall {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        tool_name: String,
    },

    /// Tool execution progress update (streaming).
    ToolProgress {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        tool_name: String,
        progress_text: String,
    },

    /// Tool call completed.
    ToolResult {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        tool_name: String,
        is_error: bool,
    },

    /// Agent completed its work.
    AgentComplete {
        agent_type: AgentType,
        session_id: Option<String>,
        depth: u32,
        success: bool,
    },

    /// A new child scope was created (subtask from decompose).
    SubtaskCreated {
        parent_depth: u32,
        new_depth: u32,
        path: String,
        task_description: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_event_clone_and_debug() {
        let event = LogEvent::AgentMessage {
            agent_type: AgentType::Planner,
            session_id: Some("test-session".to_string()),
            depth: 0,
            content: "Hello world".to_string(),
        };

        // Test Clone
        let cloned = event.clone();
        
        // Test Debug
        let debug_str = format!("{:?}", cloned);
        assert!(debug_str.contains("AgentMessage"));
        assert!(debug_str.contains("Planner"));
    }

    #[test]
    fn test_all_event_variants() {
        let events = vec![
            LogEvent::AgentStarted {
                agent_type: AgentType::Orchestrator,
                session_id: "sess-1".to_string(),
                depth: 0,
                task: "Test task".to_string(),
            },
            LogEvent::AgentMessage {
                agent_type: AgentType::Planner,
                session_id: None,
                depth: 1,
                content: "Planning...".to_string(),
            },
            LogEvent::ToolCall {
                agent_type: AgentType::Implementer { index: 1 },
                session_id: Some("sess-2".to_string()),
                depth: 2,
                tool_name: "view".to_string(),
            },
            LogEvent::ToolProgress {
                agent_type: AgentType::Implementer { index: 1 },
                session_id: Some("sess-2".to_string()),
                depth: 2,
                tool_name: "view".to_string(),
                progress_text: "Reading file...".to_string(),
            },
            LogEvent::ToolResult {
                agent_type: AgentType::Implementer { index: 2 },
                session_id: None,
                depth: 1,
                tool_name: "save-file".to_string(),
                is_error: false,
            },
            LogEvent::AgentComplete {
                agent_type: AgentType::Orchestrator,
                session_id: Some("sess-1".to_string()),
                depth: 0,
                success: true,
            },
            LogEvent::SubtaskCreated {
                parent_depth: 0,
                new_depth: 1,
                path: "/logs/run-123/subtask-001".to_string(),
                task_description: "Implement feature X".to_string(),
            },
        ];

        for event in events {
            // All events should be cloneable and debuggable
            let _ = event.clone();
            let _ = format!("{:?}", event);
        }
    }
}

