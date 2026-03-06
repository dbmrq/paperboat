//! Unit tests for the testing module's core types and builders.

use super::*;

#[test]
fn test_mock_tool_response_builder() {
    let response = MockToolResponseBuilder::new()
        .tool_type(MockToolType::SpawnAgents)
        .task_pattern(".*auth.*")
        .success("Implemented authentication")
        .files_modified(vec!["src/auth.rs".to_string()])
        .build();

    assert_eq!(response.tool_type, MockToolType::SpawnAgents);
    assert_eq!(response.task_pattern, Some(".*auth.*".to_string()));
    assert!(response.response.success);
    assert_eq!(response.response.summary, "Implemented authentication");
    assert_eq!(
        response.response.files_modified,
        Some(vec!["src/auth.rs".to_string()])
    );
}

#[test]
fn test_mock_tool_response_builder_failure() {
    let response = MockToolResponseBuilder::new()
        .tool_type(MockToolType::Decompose)
        .failure("Task too complex")
        .build();

    assert!(!response.response.success);
    assert_eq!(
        response.response.error,
        Some("Task too complex".to_string())
    );
}

#[test]
fn test_mock_session_builder() {
    let session = MockSessionBuilder::new("test-session-001")
        .with_message_chunk("Starting...", 100)
        .with_message_chunk("Planning complete.", 200)
        .with_turn_finished(50)
        .build();

    assert_eq!(session.session_id, "test-session-001");
    assert_eq!(session.updates.len(), 3);
    assert_eq!(session.updates[0].session_update, "agent_message_chunk");
    assert_eq!(session.updates[0].content, Some("Starting...".to_string()));
    assert_eq!(session.updates[1].session_update, "agent_message_chunk");
    assert_eq!(session.updates[2].session_update, "agent_turn_finished");
}

#[test]
fn test_mock_scenario_parse() {
    let toml = r#"
[scenario]
name = "test_scenario"
description = "A test scenario"

[[planner_sessions]]
session_id = "planner-001"

[[planner_sessions.updates]]
delay_ms = 100
session_update = "agent_message_chunk"
content = "Planning..."

[[planner_sessions.updates]]
delay_ms = 50
session_update = "agent_turn_finished"
"#;

    let scenario = MockScenario::parse(toml).unwrap();

    assert_eq!(scenario.scenario.name, "test_scenario");
    assert_eq!(scenario.planner_sessions.len(), 1);
    assert_eq!(scenario.planner_sessions[0].session_id, "planner-001");
    assert_eq!(scenario.planner_sessions[0].updates.len(), 2);
}

#[test]
fn test_mock_scenario_sessions_for() {
    let scenario = MockScenario {
        planner_sessions: vec![MockSessionBuilder::new("p1").build()],
        orchestrator_sessions: vec![
            MockSessionBuilder::new("o1").build(),
            MockSessionBuilder::new("o2").build(),
        ],
        implementer_sessions: vec![],
        ..Default::default()
    };

    assert_eq!(scenario.sessions_for(AgentType::Planner).len(), 1);
    assert_eq!(scenario.sessions_for(AgentType::Orchestrator).len(), 2);
    assert_eq!(scenario.sessions_for(AgentType::Implementer).len(), 0);
}

#[test]
fn test_mock_scenario_find_tool_response() {
    let scenario = MockScenario {
        mock_tool_responses: vec![
            MockToolResponseBuilder::new()
                .tool_type(MockToolType::SpawnAgents)
                .task_pattern(".*auth.*")
                .success("Auth done")
                .build(),
            MockToolResponseBuilder::new()
                .tool_type(MockToolType::SpawnAgents)
                .success("Default response")
                .build(),
        ],
        ..Default::default()
    };

    // Should match the auth pattern
    let response = scenario.find_tool_response(MockToolType::SpawnAgents, "spawn_agents auth");
    assert!(response.is_some());
    assert_eq!(response.unwrap().response.summary, "Auth done");

    // Should match the default (no pattern)
    let response = scenario.find_tool_response(MockToolType::SpawnAgents, "other task");
    assert!(response.is_some());
    assert_eq!(response.unwrap().response.summary, "Default response");

    // Should not match decompose
    let response = scenario.find_tool_response(MockToolType::Decompose, "any task");
    assert!(response.is_none());
}
