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

#[test]
fn test_mock_session_builder_with_skip_tasks() {
    let session = MockSessionBuilder::new("orch-001")
        .with_message_chunk("Executing...", 0)
        .with_skip_tasks(
            vec!["task001".to_string(), "task002".to_string()],
            Some("Not needed".to_string()),
            50,
        )
        .with_complete(true, Some("Done".to_string()), 0)
        .with_turn_finished(0)
        .build();

    assert_eq!(session.session_id, "orch-001");
    assert_eq!(session.updates.len(), 4);

    // Find the skip_tasks update
    let skip_update = session
        .updates
        .iter()
        .find(|u| {
            matches!(
                &u.inject_mcp_tool_call,
                Some(MockMcpToolCall::SkipTasks { .. })
            )
        })
        .expect("Should have skip_tasks injection");

    match &skip_update.inject_mcp_tool_call {
        Some(MockMcpToolCall::SkipTasks { task_ids, reason }) => {
            assert_eq!(
                task_ids,
                &vec!["task001".to_string(), "task002".to_string()]
            );
            assert_eq!(reason, &Some("Not needed".to_string()));
        }
        _ => panic!("Expected SkipTasks injection"),
    }
}

#[test]
fn test_mock_session_builder_with_skip_tasks_no_reason() {
    let session = MockSessionBuilder::new("orch-002")
        .with_skip_tasks(vec!["task003".to_string()], None, 0)
        .build();

    let skip_update = session
        .updates
        .iter()
        .find(|u| {
            matches!(
                &u.inject_mcp_tool_call,
                Some(MockMcpToolCall::SkipTasks { .. })
            )
        })
        .expect("Should have skip_tasks injection");

    match &skip_update.inject_mcp_tool_call {
        Some(MockMcpToolCall::SkipTasks { task_ids, reason }) => {
            assert_eq!(task_ids, &vec!["task003".to_string()]);
            assert!(reason.is_none());
        }
        _ => panic!("Expected SkipTasks injection"),
    }
}

#[test]
fn test_mock_session_builder_with_create_task_dependencies() {
    let session = MockSessionBuilder::new("planner-deps-001")
        .with_create_task("Task A", "Primary work", 0)
        .with_create_task_dependencies(
            "Task B",
            "Follow-up work",
            vec!["Task A".to_string()],
            0,
        )
        .build();

    let create_updates: Vec<_> = session
        .updates
        .iter()
        .filter_map(|update| update.inject_mcp_tool_call.as_ref())
        .collect();
    assert_eq!(create_updates.len(), 2);

    match create_updates[1] {
        MockMcpToolCall::CreateTask {
            name,
            description,
            dependencies,
        } => {
            assert_eq!(name, "Task B");
            assert_eq!(description, "Follow-up work");
            assert_eq!(dependencies, &vec!["Task A".to_string()]);
        }
        _ => panic!("Expected CreateTask injection"),
    }
}

#[test]
fn test_mock_session_builder_with_spawn_agents_batch() {
    let session = MockSessionBuilder::new("orch-batch-001")
        .with_spawn_agents(
            vec![
                MockAgentSpec {
                    role: Some("implementer".to_string()),
                    task: Some("Task A".to_string()),
                    task_id: None,
                    prompt: None,
                    tools: None,
                    model_complexity: None,
                },
                MockAgentSpec {
                    role: Some("verifier".to_string()),
                    task: Some("Task B".to_string()),
                    task_id: None,
                    prompt: None,
                    tools: Some(vec!["read_file".to_string()]),
                    model_complexity: None,
                },
            ],
            MockWaitMode::Any,
            25,
        )
        .build();

    let spawn_update = session
        .updates
        .iter()
        .find(|u| {
            matches!(
                &u.inject_mcp_tool_call,
                Some(MockMcpToolCall::SpawnAgents { .. })
            )
        })
        .expect("Should have spawn_agents injection");

    match &spawn_update.inject_mcp_tool_call {
        Some(MockMcpToolCall::SpawnAgents { task, agents, wait }) => {
            assert!(task.is_none());
            assert_eq!(*wait, MockWaitMode::Any);
            assert_eq!(agents.len(), 2);
            assert_eq!(agents[0].task.as_deref(), Some("Task A"));
            assert_eq!(agents[1].role.as_deref(), Some("verifier"));
        }
        _ => panic!("Expected SpawnAgents injection"),
    }
}

#[test]
fn test_mock_scenario_parse_spawn_agents_batch() {
    let toml = r#"
[scenario]
name = "spawn_batch"
description = "Parse multi-agent spawn batch"

[[orchestrator_sessions]]
session_id = "orchestrator-001"

[[orchestrator_sessions.updates]]
delay_ms = 0
session_update = "agent_message_chunk"
content = "[Calling spawn_agents]"

[orchestrator_sessions.updates.inject_mcp_tool_call]
tool = "spawn_agents"
wait = "any"

[[orchestrator_sessions.updates.inject_mcp_tool_call.agents]]
role = "implementer"
task = "Task A"

[[orchestrator_sessions.updates.inject_mcp_tool_call.agents]]
role = "verifier"
task = "Task B"
"#;

    let scenario = MockScenario::parse(toml).expect("Scenario should parse");
    let update = &scenario.orchestrator_sessions[0].updates[0];

    match &update.inject_mcp_tool_call {
        Some(MockMcpToolCall::SpawnAgents { task, agents, wait }) => {
            assert!(task.is_none());
            assert_eq!(*wait, MockWaitMode::Any);
            assert_eq!(agents.len(), 2);
            assert_eq!(agents[0].task.as_deref(), Some("Task A"));
            assert_eq!(agents[1].role.as_deref(), Some("verifier"));
        }
        _ => panic!("Expected SpawnAgents injection"),
    }
}
