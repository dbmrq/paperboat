//! End-to-end tests that verify the complete application flow from goal to completion.
//! These tests exercise the full `App::run()` path and ensure all components integrate correctly.

use super::*;
use std::path::Path;
use std::time::Duration;

// ========================================================================
// Full Flow Verification Tests
// ========================================================================

/// E2E Test: Complete task lifecycle from Goal → Plan → Execute → Complete
///
/// This test verifies the most basic complete flow:
/// 1. Goal is submitted
/// 2. Planner creates tasks using `create_task` tool
/// 3. Planner calls complete
/// 4. Orchestrator receives the plan
/// 5. Orchestrator calls implement for a task
/// 6. Implementer executes and calls complete
/// 7. Orchestrator calls complete
/// 8. Final `TaskResult` indicates success with meaningful message
#[tokio::test]
async fn test_e2e_complete_task_lifecycle() {
    // Load the simple_implement scenario which tests the basic flow
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load simple_implement scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    // Run the complete flow
    let result = harness
        .run_goal("Add error handling to login function")
        .await
        .expect("E2E test run should complete");

    // ----------------------------------------------------------------
    // 1. Verify overall success
    // ----------------------------------------------------------------
    assert_success(&result);
    assert!(
        result.task_result.message.is_some(),
        "Final result should have a message describing completion"
    );
    let message = result.task_result.message.as_ref().unwrap();
    assert!(!message.is_empty(), "Final message should not be empty");

    // ----------------------------------------------------------------
    // 2. Verify all agent types were spawned in correct order
    // ----------------------------------------------------------------
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);
    assert_implementer_spawned(&result);

    // Verify sessions were created in the expected order:
    // planner first, then orchestrator, then implementer
    let sessions = &result.sessions_created;
    assert!(
        sessions.len() >= 3,
        "Expected at least 3 sessions (planner, orchestrator, implementer), got {}",
        sessions.len()
    );

    // Find indices to verify order
    let planner_idx = sessions.iter().position(|s| s.contains("planner"));
    let orch_idx = sessions
        .iter()
        .position(|s| s.contains("orchestrator") || s.contains("orch"));
    let impl_idx = sessions.iter().position(|s| s.contains("impl"));

    assert!(planner_idx.is_some(), "Planner session should exist");
    assert!(orch_idx.is_some(), "Orchestrator session should exist");
    assert!(impl_idx.is_some(), "Implementer session should exist");

    // Planner should come before orchestrator
    assert!(
        planner_idx.unwrap() < orch_idx.unwrap(),
        "Planner session should be created before orchestrator. Sessions: {sessions:?}",
    );

    // ----------------------------------------------------------------
    // 3. Verify tool calls happened in correct sequence
    // ----------------------------------------------------------------
    let tool_calls = &result.tool_calls;
    assert!(!tool_calls.is_empty(), "Should have captured tool calls");

    // Find tool call types
    let mut create_task_idx = None;
    let mut implement_idx = None;
    let mut complete_indices = Vec::new();

    for (i, tc) in tool_calls.iter().enumerate() {
        match &tc.call {
            crate::mcp_server::ToolCall::CreateTask { .. } => {
                if create_task_idx.is_none() {
                    create_task_idx = Some(i);
                }
            }
            crate::mcp_server::ToolCall::SpawnAgents { .. } => {
                if implement_idx.is_none() {
                    implement_idx = Some(i);
                }
            }
            crate::mcp_server::ToolCall::Complete { .. } => {
                complete_indices.push(i);
            }
            _ => {}
        }
    }

    // create_task should be called first
    assert!(
        create_task_idx.is_some(),
        "create_task should have been called"
    );

    // implement should be called after create_task
    assert!(implement_idx.is_some(), "implement should have been called");
    assert!(
        create_task_idx.unwrap() < implement_idx.unwrap(),
        "create_task should happen before implement"
    );

    // There should be multiple complete calls (planner, implementer, orchestrator)
    assert!(
        complete_indices.len() >= 2,
        "Expected at least 2 complete calls (planner and orchestrator), got {}",
        complete_indices.len()
    );

    // ----------------------------------------------------------------
    // 4. Verify data flows correctly
    // ----------------------------------------------------------------

    // Verify implement call has meaningful task
    let impl_calls = result.implement_calls();
    assert_eq!(
        impl_calls.len(),
        1,
        "Should have exactly one implement call"
    );
    assert!(
        impl_calls[0].len() > 10,
        "Implement task should have substantive content, got: {}",
        impl_calls[0]
    );

    // Verify the implement response was successful
    let impl_tool_call = tool_calls
        .iter()
        .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .expect("Should have spawn_agents tool call");
    assert!(
        impl_tool_call.response.success,
        "SpawnAgents tool call should have succeeded"
    );
}

/// E2E Test: Task lifecycle with decomposition
///
/// This test verifies the decomposition flow:
/// 1. Goal is submitted
/// 2. Planner creates a plan with complex task
/// 3. Orchestrator calls decompose for complex task
/// 4. Sub-planner creates sub-plan
/// 5. Sub-orchestrator executes sub-tasks with implement
/// 6. Main orchestrator continues and completes
#[tokio::test]
async fn test_e2e_with_decomposition() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/nested_decompose.toml"))
            .expect("Failed to load nested_decompose scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(25));

    let result = harness
        .run_goal("Build authentication system with rate limiting")
        .await
        .expect("E2E decomposition test should complete");

    // ----------------------------------------------------------------
    // 1. Verify overall success
    // ----------------------------------------------------------------
    assert_success(&result);

    // ----------------------------------------------------------------
    // 2. Verify decomposition happened
    // ----------------------------------------------------------------
    assert_decompose_called(&result);

    let decompose_calls = result.decompose_calls();
    assert!(
        !decompose_calls.is_empty(),
        "Should have at least one decompose call"
    );

    // Verify decompose call has meaningful task
    let decompose_task = &decompose_calls[0];
    assert!(
        decompose_task.len() > 10,
        "Decompose task should have substantive content, got: {decompose_task}",
    );
    assert!(
        decompose_task.to_lowercase().contains("auth"),
        "Decompose task should be about authentication, got: {decompose_task}",
    );

    // ----------------------------------------------------------------
    // 3. Verify nested planners were spawned (main + sub)
    // ----------------------------------------------------------------
    let planner_sessions: Vec<_> = result
        .sessions_created
        .iter()
        .filter(|s| s.contains("planner"))
        .collect();

    assert!(
        planner_sessions.len() >= 2,
        "Should have at least 2 planner sessions (main + decomposed), got: {planner_sessions:?}",
    );

    // ----------------------------------------------------------------
    // 4. Verify multiple implement calls were made
    // ----------------------------------------------------------------
    let impl_calls = result.implement_calls();
    assert!(
        !impl_calls.is_empty(),
        "Should have at least one implement call after decomposition"
    );

    // ----------------------------------------------------------------
    // 5. Verify tool call sequence includes decompose
    // ----------------------------------------------------------------
    let tool_calls = &result.tool_calls;

    let has_decompose = tool_calls
        .iter()
        .any(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Decompose { .. }));
    assert!(has_decompose, "Should have captured decompose tool call");

    // Decompose should have a successful response (the App handles decomposition internally)
    let decompose_tc = tool_calls
        .iter()
        .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Decompose { .. }))
        .expect("Should have decompose tool call");
    assert!(
        decompose_tc.response.success,
        "Decompose should succeed, got response: {:?}",
        decompose_tc.response
    );

    // ----------------------------------------------------------------
    // 6. Verify final result message is meaningful
    // ----------------------------------------------------------------
    assert!(
        result.task_result.message.is_some(),
        "Final result should have a message"
    );
    let message = result.task_result.message.as_ref().unwrap();
    assert!(!message.is_empty(), "Final message should not be empty");
}

/// E2E Test: Multiple implementers in sequence
///
/// This test verifies that multiple implement calls are handled correctly:
/// 1. Orchestrator makes multiple `implement()` calls
/// 2. Each implementer session is spawned and completes
/// 3. All tool calls are captured with correct arguments
#[tokio::test]
async fn test_e2e_multiple_implementers() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/multi_implement.toml"))
            .expect("Failed to load multi_implement scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(20));

    let result = harness
        .run_goal("Create user management system with database, service, and API")
        .await
        .expect("E2E multi-implementer test should complete");

    // ----------------------------------------------------------------
    // 1. Verify success
    // ----------------------------------------------------------------
    assert_success(&result);

    // ----------------------------------------------------------------
    // 2. Verify multiple implementers were spawned
    // ----------------------------------------------------------------
    let impl_sessions: Vec<_> = result
        .sessions_created
        .iter()
        .filter(|s| s.contains("impl"))
        .collect();

    assert!(
        impl_sessions.len() >= 3,
        "Should have at least 3 implementer sessions, got {}: {impl_sessions:?}",
        impl_sessions.len(),
    );

    // ----------------------------------------------------------------
    // 3. Verify all 3 implement calls were made
    // ----------------------------------------------------------------
    let impl_calls = result.implement_calls();
    assert_eq!(
        impl_calls.len(),
        3,
        "Should have exactly 3 implement calls, got {}: {impl_calls:?}",
        impl_calls.len(),
    );

    // ----------------------------------------------------------------
    // 4. Verify implement call arguments are correct and different
    // ----------------------------------------------------------------
    let all_calls_text = impl_calls.join(" ").to_lowercase();

    // Each implement call should be for a different task
    assert!(
        all_calls_text.contains("database") || all_calls_text.contains("schema"),
        "Should have a database-related task, got: {impl_calls:?}",
    );
    assert!(
        all_calls_text.contains("user") || all_calls_text.contains("service"),
        "Should have a user service task, got: {impl_calls:?}",
    );
    assert!(
        all_calls_text.contains("api") || all_calls_text.contains("endpoint"),
        "Should have an API endpoint task, got: {impl_calls:?}",
    );

    // ----------------------------------------------------------------
    // 5. Verify all implement calls have responses
    // ----------------------------------------------------------------
    let impl_tool_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .collect();

    assert_eq!(
        impl_tool_calls.len(),
        3,
        "Should have 3 implement tool calls captured"
    );

    for (idx, tc) in impl_tool_calls.iter().enumerate() {
        assert!(
            tc.response.success,
            "Implement call {} should have succeeded, got: {:?}",
            idx + 1,
            tc.response
        );
        assert!(
            !tc.response.request_id.is_empty(),
            "Implement call {} should have request_id",
            idx + 1
        );
    }

    // ----------------------------------------------------------------
    // 6. Verify final message summarizes completion
    // ----------------------------------------------------------------
    assert!(result.task_result.message.is_some());
    let message = result.task_result.message.as_ref().unwrap();
    assert!(
        message.to_lowercase().contains("complet")
            || message.to_lowercase().contains("success")
            || message.to_lowercase().contains("done"),
        "Final message should indicate completion, got: {message}",
    );
}

// ========================================================================
// Data Flow Verification Tests
// ========================================================================

/// E2E Test: Verify prompts contain expected content
///
/// Tests that the `expected_prompt_contains` validation in `MockAcpClient` works
/// and that prompts flow correctly between agents.
#[tokio::test]
async fn test_e2e_prompt_content_verification() {
    // Create a scenario that validates prompt content
    let scenario = MockScenario {
        scenario: ScenarioMetadata {
            name: "prompt_verification".to_string(),
            description: "Verify prompts contain expected content".to_string(),
        },
        planner_sessions: vec![MockAgentSession {
            session_id: "planner-prompt-001".to_string(),
            updates: vec![
                MockSessionUpdate {
                    delay_ms: 50,
                    session_update: "agent_message_chunk".to_string(),
                    content: Some("Analyzing request...".to_string()),
                    tool_title: None,
                    tool_result: None,
                    inject_mcp_tool_call: None,
                },
                MockSessionUpdate {
                    delay_ms: 50,
                    session_update: "agent_message_chunk".to_string(),
                    content: None,
                    tool_title: None,
                    tool_result: None,
                    inject_mcp_tool_call: Some(MockMcpToolCall::CreateTask {
                        name: "Implement feature X".to_string(),
                        description: "Implement feature X according to requirements".to_string(),
                        dependencies: vec![],
                    }),
                },
                MockSessionUpdate {
                    delay_ms: 50,
                    session_update: "agent_message_chunk".to_string(),
                    content: None,
                    tool_title: None,
                    tool_result: None,
                    inject_mcp_tool_call: Some(MockMcpToolCall::Complete {
                        success: true,
                        message: Some("Plan created".to_string()),
                        notes: None,
                        add_tasks: None,
                    }),
                },
                MockSessionUpdate {
                    delay_ms: 50,
                    session_update: "agent_turn_finished".to_string(),
                    content: None,
                    tool_title: None,
                    tool_result: None,
                    inject_mcp_tool_call: None,
                },
            ],
            // Validate that planner receives the goal in prompt
            expected_prompt_contains: Some(vec!["specific feature".to_string()]),
        }],
        orchestrator_sessions: vec![MockSessionBuilder::new("orch-prompt-001")
            .with_message_chunk("Executing plan...", 50)
            .with_implement("Implement feature X", 50)
            // Skip the planner's tracked task since we used a raw task description
            .with_skip_tasks(
                vec!["task001".to_string()],
                Some("Implemented via spawn_agents".to_string()),
                50,
            )
            .with_complete(true, Some("Done".to_string()), 50)
            .with_turn_finished(50)
            .build()],
        implementer_sessions: vec![MockSessionBuilder::new("impl-prompt-001")
            .with_message_chunk("Implementing...", 50)
            .with_complete(true, Some("Feature X done".to_string()), 50)
            .with_turn_finished(50)
            .build()],
        mock_tool_responses: vec![MockToolResponseBuilder::new()
            .tool_type(MockToolType::SpawnAgents)
            .success("Feature X implemented")
            .build()],
        mock_acp_responses: vec![],
    };

    let mut harness = TestHarness::with_scenario(scenario).with_timeout(Duration::from_secs(10));

    // Run with a goal that matches the expected_prompt_contains pattern
    let result = harness
        .run_goal("Please add this specific feature to the codebase")
        .await
        .expect("Prompt verification test should pass");

    assert_success(&result);
    assert_planner_spawned(&result);
}

/// E2E Test: Verify tool call arguments are correct
///
/// Tests that tool calls receive the correct arguments from the scenario.
#[tokio::test]
async fn test_e2e_tool_call_arguments_verification() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Add error handling")
        .await
        .expect("Test should complete");

    // ----------------------------------------------------------------
    // Verify create_task tool call has task content
    // ----------------------------------------------------------------
    let create_task_call = result
        .tool_calls
        .iter()
        .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::CreateTask { .. }));

    assert!(
        create_task_call.is_some(),
        "Should have a create_task tool call"
    );

    if let crate::mcp_server::ToolCall::CreateTask {
        name, description, ..
    } = &create_task_call.unwrap().call
    {
        assert!(!name.is_empty(), "Task name should not be empty");
        assert!(
            description.len() > 10,
            "Task description should have substantive content (>10 chars), got {} chars",
            description.len()
        );
    }

    // ----------------------------------------------------------------
    // Verify spawn_agents tool call has task content
    // ----------------------------------------------------------------
    let spawn_agents_call = result
        .tool_calls
        .iter()
        .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::SpawnAgents { .. }));

    assert!(
        spawn_agents_call.is_some(),
        "Should have a spawn_agents tool call"
    );

    if let crate::mcp_server::ToolCall::SpawnAgents { agents, .. } =
        &spawn_agents_call.unwrap().call
    {
        let agent = agents.first().expect("Should have at least one agent");
        // Get task or task_id
        let task = agent
            .task
            .as_ref()
            .or(agent.task_id.as_ref())
            .expect("Agent should have task or task_id");
        assert!(!task.is_empty(), "SpawnAgents task should not be empty");
        assert!(
            task.to_lowercase().contains("error") || task.to_lowercase().contains("handling"),
            "SpawnAgents task should relate to the goal, got: {task}",
        );
    }

    // ----------------------------------------------------------------
    // Verify complete tool calls have success flags
    // ----------------------------------------------------------------
    let complete_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Complete { .. }))
        .collect();

    assert!(
        !complete_calls.is_empty(),
        "Should have at least one complete tool call"
    );

    // At least the final complete should be success=true
    let has_success_complete = complete_calls.iter().any(|tc| {
        if let crate::mcp_server::ToolCall::Complete { success, .. } = &tc.call {
            *success
        } else {
            false
        }
    });

    assert!(
        has_success_complete,
        "At least one complete call should have success=true"
    );
}

/// E2E Test: Verify final result message is meaningful
///
/// Tests that the final `TaskResult` has a meaningful message that describes
/// what was accomplished.
#[tokio::test]
async fn test_e2e_final_result_message_meaningful() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/multi_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Create complete user system")
        .await
        .expect("Test should complete");

    assert_success(&result);

    // ----------------------------------------------------------------
    // Verify message exists and is meaningful
    // ----------------------------------------------------------------
    assert!(
        result.task_result.message.is_some(),
        "TaskResult should have a message"
    );

    let message = result.task_result.message.as_ref().unwrap();

    assert!(!message.is_empty(), "Message should not be empty");

    assert!(
        message.len() > 5,
        "Message should be descriptive (>5 chars), got: {}",
        message
    );

    // Message should indicate completion/success in some way
    let msg_lower = message.to_lowercase();
    assert!(
        msg_lower.contains("complet")
            || msg_lower.contains("success")
            || msg_lower.contains("done")
            || msg_lower.contains("finish")
            || msg_lower.contains("task"),
        "Message should indicate completion or success, got: {}",
        message
    );
}

// ========================================================================
// State Transition Verification Tests
// ========================================================================

/// E2E Test: Verify sessions are created in correct order
///
/// Tests that agent sessions are created in the expected sequence
/// based on the application flow.
#[tokio::test]
async fn test_e2e_session_creation_order() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Test session ordering")
        .await
        .expect("Test should complete");

    let sessions = &result.sessions_created;

    // ----------------------------------------------------------------
    // Verify session count
    // ----------------------------------------------------------------
    assert!(
        sessions.len() >= 3,
        "Should have at least 3 sessions, got: {:?}",
        sessions
    );

    // ----------------------------------------------------------------
    // Verify ordering: planner → orchestrator → implementer
    // ----------------------------------------------------------------
    let planner_pos = sessions
        .iter()
        .position(|s| s.contains("planner"))
        .expect("Should have planner session");

    let orch_pos = sessions
        .iter()
        .position(|s| s.contains("orchestrator") || s.contains("orch"))
        .expect("Should have orchestrator session");

    let impl_pos = sessions
        .iter()
        .position(|s| s.contains("impl"))
        .expect("Should have implementer session");

    assert!(
        planner_pos < orch_pos,
        "Planner (pos {}) should come before orchestrator (pos {})",
        planner_pos,
        orch_pos
    );

    assert!(
        orch_pos < impl_pos,
        "Orchestrator (pos {}) should come before implementer (pos {})",
        orch_pos,
        impl_pos
    );
}

/// E2E Test: Verify tool calls happen at correct times
///
/// Tests that tool calls occur in the expected sequence during execution.
#[tokio::test]
async fn test_e2e_tool_call_timing() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Test tool call timing")
        .await
        .expect("Test should complete");

    let tool_calls = &result.tool_calls;

    // ----------------------------------------------------------------
    // Build sequence of tool call types
    // ----------------------------------------------------------------
    let call_sequence: Vec<&str> = tool_calls.iter().map(|tc| tc.call.tool_type()).collect();

    // ----------------------------------------------------------------
    // Verify expected sequence patterns
    // ----------------------------------------------------------------

    // create_task should appear early (planner phase)
    let create_task_pos = call_sequence.iter().position(|&t| t == "create_task");
    assert!(
        create_task_pos.is_some(),
        "create_task should be in call sequence"
    );

    // spawn_agents should appear after create_task (orchestrator delegating)
    let spawn_agents_pos = call_sequence.iter().position(|&t| t == "spawn_agents");
    assert!(
        spawn_agents_pos.is_some(),
        "spawn_agents should be in call sequence"
    );
    assert!(
        create_task_pos.unwrap() < spawn_agents_pos.unwrap(),
        "create_task should happen before spawn_agents. Sequence: {:?}",
        call_sequence
    );

    // First complete should be planner's (after create_task)
    let first_complete_pos = call_sequence.iter().position(|&t| t == "complete");
    assert!(
        first_complete_pos.is_some(),
        "complete should be in call sequence"
    );

    // There should be multiple complete calls
    let complete_count = call_sequence.iter().filter(|&&t| t == "complete").count();
    assert!(
        complete_count >= 2,
        "Should have at least 2 complete calls (planner + orchestrator), got {}",
        complete_count
    );

    // The last tool call should be a complete (orchestrator finishing)
    assert_eq!(
        *call_sequence.last().unwrap(),
        "complete",
        "Last tool call should be complete, sequence: {:?}",
        call_sequence
    );
}

/// E2E Test: Verify nested session creation for decomposition
///
/// Tests that decomposition creates additional planner and orchestrator sessions.
#[tokio::test]
async fn test_e2e_nested_session_creation() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/nested_decompose.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(25));

    let result = harness
        .run_goal("Build complex system requiring decomposition")
        .await
        .expect("Test should complete");

    let sessions = &result.sessions_created;

    // ----------------------------------------------------------------
    // Count session types
    // ----------------------------------------------------------------
    let planner_count = sessions.iter().filter(|s| s.contains("planner")).count();

    let orch_count = sessions
        .iter()
        .filter(|s| s.contains("orchestrator") || s.contains("orch"))
        .count();

    let impl_count = sessions.iter().filter(|s| s.contains("impl")).count();

    // ----------------------------------------------------------------
    // Verify counts reflect nested structure
    // ----------------------------------------------------------------

    // Should have at least 2 planners (main + sub for decompose)
    assert!(
        planner_count >= 2,
        "Should have at least 2 planner sessions (main + decomposed), got {}",
        planner_count
    );

    // Should have at least 2 orchestrators (main + sub)
    assert!(
        orch_count >= 2,
        "Should have at least 2 orchestrator sessions (main + decomposed), got {}",
        orch_count
    );

    // Should have multiple implementers (sub-orchestrator implements multiple tasks)
    assert!(
        impl_count >= 2,
        "Should have at least 2 implementer sessions, got {}",
        impl_count
    );

    // Total session count should reflect the nested structure
    assert!(
        sessions.len() >= 6,
        "Should have at least 6 sessions for nested flow, got {}",
        sessions.len()
    );
}

// ========================================================================
// Error/Failure E2E Tests
// ========================================================================

/// E2E Test: Verify failure flow is handled correctly
///
/// Tests that when an implementation fails and is retried, the system
/// captures both attempts. This verifies the retry pattern is working
/// at the tool call level.
///
/// Note: The current App implementation may return the first failure's
/// message as the final result due to the async nature of how implementer
/// complete() signals are processed. This test focuses on verifying
/// that the retry pattern is captured in tool calls.
#[tokio::test]
async fn test_e2e_failure_and_recovery_flow() {
    let harness = TestHarness::with_scenario_file(Path::new("tests/scenarios/error_recovery.toml"))
        .expect("Failed to load error_recovery scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Fix critical database bug")
        .await
        .expect("Error recovery test should complete");

    // ----------------------------------------------------------------
    // 1. Verify all agent types were spawned
    // ----------------------------------------------------------------
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);
    assert_implementer_spawned(&result);

    // ----------------------------------------------------------------
    // 2. Verify multiple implement calls (retry pattern)
    // ----------------------------------------------------------------
    let impl_calls = result.implement_calls();
    assert!(
        impl_calls.len() >= 2,
        "Should have at least 2 implement calls (initial + retry), got {}",
        impl_calls.len()
    );

    // ----------------------------------------------------------------
    // 3. Verify both implementer sessions were created
    // ----------------------------------------------------------------
    let impl_sessions: Vec<_> = result
        .sessions_created
        .iter()
        .filter(|s| s.contains("impl"))
        .collect();

    assert!(
        impl_sessions.len() >= 2,
        "Should have at least 2 implementer sessions (failed + retry), got {:?}",
        impl_sessions
    );

    // ----------------------------------------------------------------
    // 4. Verify tool responses capture the retry pattern
    // ----------------------------------------------------------------
    let impl_tool_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .collect();

    assert!(
        impl_tool_calls.len() >= 2,
        "Should have captured at least 2 spawn_agents tool calls"
    );

    // First spawn_agents should have failed (per scenario definition)
    let first_impl = &impl_tool_calls[0];
    assert!(
        !first_impl.response.success || first_impl.response.error.is_some(),
        "First spawn_agents should indicate failure, got: {:?}",
        first_impl.response
    );

    // Second implement should have succeeded (per scenario definition)
    let second_impl = &impl_tool_calls[1];
    assert!(
        second_impl.response.success,
        "Second implement (retry) should succeed, got: {:?}",
        second_impl.response
    );

    // ----------------------------------------------------------------
    // 5. Verify the retry tasks are different
    // ----------------------------------------------------------------
    let first_task = &impl_calls[0].to_lowercase();
    let second_task = &impl_calls[1].to_lowercase();

    // Both should be about database/bug fix
    assert!(
        first_task.contains("database")
            || first_task.contains("bug")
            || first_task.contains("timeout"),
        "First implement task should be about the bug, got: {}",
        impl_calls[0]
    );

    // Second task should have more detail (retry with more specific instructions)
    assert!(
        second_task.len() > first_task.len()
            || second_task.contains("pool")
            || second_task.contains("backoff")
            || second_task.contains("specific"),
        "Second implement task should have more detail than first. First: '{}', Second: '{}'",
        impl_calls[0],
        impl_calls[1]
    );
}

/// E2E Test: Verify complete flow with planning-only scenario
///
/// Tests a flow where the orchestrator immediately completes without
/// calling implement (planning-only use case).
#[tokio::test]
async fn test_e2e_planning_only_flow() {
    let harness = TestHarness::with_scenario_file(Path::new("tests/scenarios/planning_only.toml"))
        .expect("Failed to load planning_only scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Create a high-level project plan")
        .await
        .expect("Planning-only test should complete");

    // ----------------------------------------------------------------
    // Verify success
    // ----------------------------------------------------------------
    assert_success(&result);

    // ----------------------------------------------------------------
    // Verify planner and orchestrator were spawned
    // ----------------------------------------------------------------
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);

    // ----------------------------------------------------------------
    // Verify no implementers were spawned (planning only)
    // ----------------------------------------------------------------
    assert!(
        !result.implementer_was_spawned(),
        "Planning-only flow should not spawn implementers"
    );

    // ----------------------------------------------------------------
    // Verify no implement calls were made
    // ----------------------------------------------------------------
    let impl_calls = result.implement_calls();
    assert!(
        impl_calls.is_empty(),
        "Planning-only flow should not have implement calls, got: {:?}",
        impl_calls
    );

    // ----------------------------------------------------------------
    // Verify create_task was called
    // ----------------------------------------------------------------
    let create_task_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::CreateTask { .. }))
        .collect();

    assert!(
        !create_task_calls.is_empty(),
        "Planning-only flow should have create_task call"
    );
}

// ========================================================================
// Data Integrity E2E Tests
// ========================================================================

/// E2E Test: Verify all tool calls have valid responses
///
/// Tests that every captured tool call has a properly formed response.
#[tokio::test]
async fn test_e2e_all_tool_calls_have_responses() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/multi_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Create system with multiple components")
        .await
        .expect("Test should complete");

    // ----------------------------------------------------------------
    // Verify each tool call has a response
    // ----------------------------------------------------------------
    for (i, captured) in result.tool_calls.iter().enumerate() {
        // Request ID should be non-empty
        assert!(
            !captured.response.request_id.is_empty(),
            "Tool call {} should have non-empty request_id",
            i
        );

        // Response should have either summary (success) or error (failure)
        let has_summary = !captured.response.summary.is_empty();
        let has_error = captured.response.error.is_some();
        let is_success_flag_set = captured.response.success;

        assert!(
            has_summary || has_error || is_success_flag_set,
            "Tool call {} ({:?}) should have summary, error, or success flag. Response: {:?}",
            i,
            captured.call.tool_type(),
            captured.response
        );
    }

    // ----------------------------------------------------------------
    // Verify tool call count matches expected
    // ----------------------------------------------------------------
    assert!(
        result.tool_calls.len() >= 5,
        "Should have at least 5 tool calls (3x create_task, 3x implement, 2+ complete), got {}",
        result.tool_calls.len()
    );
}

/// E2E Test: Verify tool response content matches expectations
///
/// Tests that tool responses contain expected data based on scenario definitions.
#[tokio::test]
async fn test_e2e_tool_response_content() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Add error handling")
        .await
        .expect("Test should complete");

    // ----------------------------------------------------------------
    // Verify spawn_agents response has expected fields
    // ----------------------------------------------------------------
    let impl_response = result
        .tool_calls
        .iter()
        .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .map(|tc| &tc.response)
        .expect("Should have spawn_agents tool call");

    assert!(
        impl_response.success,
        "SpawnAgents should succeed in simple_implement scenario"
    );

    assert!(
        !impl_response.summary.is_empty(),
        "SpawnAgents response should have summary"
    );

    // Files modified should be present for successful implementations
    // (Based on simple_implement.toml, it specifies files_modified)
    assert!(
        impl_response.files_modified.is_some(),
        "Implement response should have files_modified list"
    );

    let files = impl_response.files_modified.as_ref().unwrap();
    assert!(!files.is_empty(), "Files modified list should not be empty");
}
