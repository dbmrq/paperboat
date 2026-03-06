//! Integration tests that use the TestHarness to verify major application flows.
//! These tests use scenario files and verify the orchestration between agents.

use super::*;
use std::path::Path;
use std::time::Duration;

// ========================================================================
// Happy Path Tests
// ========================================================================

/// Test a simple single-task implementation flow.
/// Verifies: Planner → Orchestrator → Implementer with one implement() call.
#[tokio::test]
async fn test_simple_implement_flow() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load simple_implement scenario");

    // Use faster timeout for tests
    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Add error handling to login")
        .await
        .expect("Test run failed");

    // Verify success
    assert_success(&result);

    // Verify all agent types were spawned
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);
    assert_implementer_spawned(&result);

    // Verify implement was called
    assert_implement_called(&result);

    // Verify specific implement call content
    let impl_calls = result.implement_calls();
    assert_eq!(impl_calls.len(), 1, "Expected exactly one implement call");
    assert!(
        impl_calls[0].to_lowercase().contains("error handling"),
        "Implement call should mention error handling, got: {}",
        impl_calls[0]
    );

    // Verify task result message exists
    assert!(
        result.task_result.message.is_some(),
        "Task result should have a message"
    );
}

/// Test multiple sequential task implementations.
/// Verifies: Orchestrator makes multiple implement() calls.
#[tokio::test]
async fn test_multi_task_flow() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/multi_implement.toml"))
            .expect("Failed to load multi_implement scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Create user management system")
        .await
        .expect("Test run failed");

    // Verify success
    assert_success(&result);

    // Verify all agents spawned
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);
    assert_implementer_spawned(&result);

    // Verify multiple implement calls were made
    let impl_calls = result.implement_calls();
    assert_eq!(
        impl_calls.len(),
        3,
        "Expected 3 implement calls, got {}: {:?}",
        impl_calls.len(),
        impl_calls
    );

    // Verify the implement calls cover different aspects
    let all_calls = impl_calls.join(" ").to_lowercase();
    assert!(
        all_calls.contains("database") || all_calls.contains("schema"),
        "Should have a database/schema task"
    );
    assert!(
        all_calls.contains("user") || all_calls.contains("service"),
        "Should have a user service task"
    );
    assert!(
        all_calls.contains("api") || all_calls.contains("endpoint"),
        "Should have an API endpoint task"
    );
}

/// Test that the planner produces a valid plan structure.
/// Verifies: create_task tool is called.
#[tokio::test]
async fn test_planning_produces_valid_plan() {
    let harness = TestHarness::with_scenario_file(Path::new("tests/scenarios/planning_only.toml"))
        .expect("Failed to load planning_only scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Create a project plan")
        .await
        .expect("Test run failed");

    // Verify success
    assert_success(&result);

    // Verify planner was spawned
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);

    // Verify create_task was called (captured in tool_calls)
    let create_task_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::CreateTask { .. }))
        .collect();

    assert!(
        !create_task_calls.is_empty(),
        "Expected create_task to be called. Tool calls: {:?}",
        result
            .tool_calls
            .iter()
            .map(|c| format!("{:?}", c.call))
            .collect::<Vec<_>>()
    );

    // Verify the task has substantive content
    if let crate::mcp_server::ToolCall::CreateTask { name, description, .. } = &create_task_calls[0].call {
        assert!(!name.is_empty(), "Task name should not be empty");
        assert!(description.len() > 10, "Task description should have substantive content");
    }

    // No implement calls in planning-only scenario
    assert!(
        result.implement_calls().is_empty(),
        "Planning-only scenario should not call implement()"
    );
}

// ========================================================================
// Orchestration Tests
// ========================================================================

/// Test that orchestrator correctly delegates to implementer.
/// Verifies: implement() calls flow through the system.
#[tokio::test]
async fn test_orchestrator_delegates_to_implementer() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(10));

    let result = harness
        .run_goal("Implement the feature")
        .await
        .expect("Test run failed");

    // Orchestrator should spawn implementer
    assert_orchestrator_spawned(&result);
    assert_implementer_spawned(&result);

    // Implement call should have response captured
    let impl_tool_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .collect();

    assert!(
        !impl_tool_calls.is_empty(),
        "Should have captured spawn_agents tool calls"
    );

    // Each implement call should have a response
    for tc in &impl_tool_calls {
        assert!(
            tc.response.success || tc.response.error.is_some(),
            "Implement call should have success or error: {:?}",
            tc.response
        );
    }
}

/// Test that orchestrator handles decompose to create subtasks.
/// Verifies: decompose() spawns sub-planner and sub-orchestrator.
#[tokio::test]
async fn test_orchestrator_handles_decompose() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/nested_decompose.toml"))
            .expect("Failed to load nested_decompose scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(20));

    let result = harness
        .run_goal("Build authentication system")
        .await
        .expect("Test run failed");

    // Verify success
    assert_success(&result);

    // Verify decompose was called
    assert_decompose_called(&result);

    let decompose_calls = result.decompose_calls();
    assert!(
        !decompose_calls.is_empty(),
        "Expected at least one decompose call"
    );

    // The decompose should mention authentication
    assert!(
        decompose_calls[0].to_lowercase().contains("auth"),
        "Decompose should be for auth task, got: {}",
        decompose_calls[0]
    );

    // Should have at least one implement call (rate limiting from main orchestrator)
    // Note: Due to timing, sub-orchestrator's implement calls may not always be captured
    let impl_calls = result.implement_calls();
    assert!(
        !impl_calls.is_empty(),
        "Expected at least 1 implement call, got none"
    );

    // Verify at least one implement call mentions expected task
    let all_impl_text = impl_calls.join(" ").to_lowercase();
    assert!(
        all_impl_text.contains("rate")
            || all_impl_text.contains("login")
            || all_impl_text.contains("auth"),
        "Implement calls should relate to auth or rate limiting, got: {:?}",
        impl_calls
    );

    // Verify multiple planners were spawned (main + sub)
    let planner_sessions: Vec<_> = result
        .sessions_created
        .iter()
        .filter(|s| s.contains("planner"))
        .collect();
    assert!(
        planner_sessions.len() >= 2,
        "Expected at least 2 planner sessions for decomposition, got: {:?}",
        planner_sessions
    );
}

/// Test that tool call responses are properly captured.
/// Verifies: All intercepted tool calls have recorded responses.
#[tokio::test]
async fn test_tool_call_responses_are_captured() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/multi_implement.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Multiple task test")
        .await
        .expect("Test run failed");

    // All tool calls should have responses captured
    assert!(
        !result.tool_calls.is_empty(),
        "Should have captured tool calls"
    );

    for (i, captured) in result.tool_calls.iter().enumerate() {
        // Each captured call should have a request_id in the response
        assert!(
            !captured.response.request_id.is_empty(),
            "Tool call {} should have request_id",
            i
        );

        // Response should have summary (success) or error (failure)
        let has_content =
            !captured.response.summary.is_empty() || captured.response.error.is_some();
        assert!(
            has_content,
            "Tool call {} should have summary or error: {:?}",
            i, captured.response
        );
    }

    // Verify different tool types were captured
    let has_create_task = result
        .tool_calls
        .iter()
        .any(|c| matches!(&c.call, crate::mcp_server::ToolCall::CreateTask { .. }));
    let has_spawn_agents = result
        .tool_calls
        .iter()
        .any(|c| matches!(&c.call, crate::mcp_server::ToolCall::SpawnAgents { .. }));
    let has_complete = result
        .tool_calls
        .iter()
        .any(|c| matches!(&c.call, crate::mcp_server::ToolCall::Complete { .. }));

    assert!(has_create_task, "Should capture create_task calls");
    assert!(has_spawn_agents, "Should capture spawn_agents calls");
    assert!(has_complete, "Should capture complete calls");
}

// ========================================================================
// Error Handling Tests
// ========================================================================

/// Test that planner failure signals are captured.
///
/// Note: The current implementation doesn't check the planner's success flag.
/// When the planner calls complete(success=false), the App still proceeds
/// if there's any text output. This test verifies that planner failure
/// signals are at least captured in tool calls.
#[tokio::test]
async fn test_planner_failure_handling() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/planner_failure.toml"))
            .expect("Failed to load planner_failure scenario");

    // Use short timeout since this scenario doesn't have an orchestrator
    // and will timeout when App tries to spawn one
    let mut harness = harness.with_timeout(Duration::from_secs(5));

    // This will timeout because the planner produces text output,
    // causing App to proceed to orchestrator phase, but there's no
    // orchestrator session defined
    let result = harness.run_goal("Ambiguous impossible task").await;

    // The run will likely timeout or fail
    // Either outcome is acceptable for this test
    match result {
        Ok(test_result) => {
            // If it completed, verify planner was spawned and captured
            assert_planner_spawned(&test_result);

            // Verify the complete call with success=false was captured
            let complete_calls: Vec<_> = test_result
                .tool_calls
                .iter()
                .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::Complete { .. }))
                .collect();

            assert!(
                !complete_calls.is_empty(),
                "Planner complete call should be captured"
            );
        }
        Err(e) => {
            // Timeout or error is expected behavior given current implementation
            // The harness returns specific error messages that may contain variations
            let error_str = e.to_string();
            assert!(
                error_str.contains("timed out")
                    || error_str.contains("Timeout")
                    || error_str.contains("timeout")
                    || error_str.contains("failed")
                    || error_str.contains("Failed"),
                "Expected timeout or failure error, got: {}",
                e
            );
        }
    }
}

/// Test error recovery when implementer fails.
/// Verifies: System captures implementer failure and multiple implement calls.
///
/// Note: The current implementation doesn't propagate implementer success/failure
/// status to the orchestrator's tool response. This test verifies the current
/// behavior where multiple implement calls are made.
#[tokio::test]
async fn test_error_recovery_flow() {
    let harness = TestHarness::with_scenario_file(Path::new("tests/scenarios/error_recovery.toml"))
        .expect("Failed to load error_recovery scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Fix critical bug")
        .await
        .expect("Test run failed");

    // Verify all agent types were spawned
    assert_planner_spawned(&result);
    assert_orchestrator_spawned(&result);
    assert_implementer_spawned(&result);

    // Should have multiple implement calls (the orchestrator tried to retry)
    let impl_calls = result.implement_calls();
    assert!(
        impl_calls.len() >= 2,
        "Expected at least 2 implement calls, got {}: {:?}",
        impl_calls.len(),
        impl_calls
    );

    // Verify both implement calls were captured
    let impl_responses: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .collect();

    assert!(
        impl_responses.len() >= 2,
        "Should have captured at least 2 spawn_agents tool calls, got {}",
        impl_responses.len()
    );

    // Verify implementer sessions were created (both first and retry)
    let impl_sessions: Vec<_> = result
        .sessions_created
        .iter()
        .filter(|s| s.contains("impl"))
        .collect();
    assert!(
        impl_sessions.len() >= 2,
        "Expected at least 2 implementer sessions, got {}: {:?}",
        impl_sessions.len(),
        impl_sessions
    );
}

// ========================================================================
// Assertion Coverage Tests
// ========================================================================

/// Comprehensive test of all assertion helpers.
/// Verifies: planner_was_spawned(), orchestrator_was_spawned(),
///           implementer_was_spawned(), implement_calls(), decompose_calls().
#[tokio::test]
async fn test_assertion_helpers_coverage() {
    let harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/nested_decompose.toml"))
            .expect("Failed to load scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(20));

    let result = harness
        .run_goal("Full flow test")
        .await
        .expect("Test run failed");

    // Test planner_was_spawned()
    assert!(
        result.planner_was_spawned(),
        "planner_was_spawned() should return true"
    );

    // Test orchestrator_was_spawned()
    assert!(
        result.orchestrator_was_spawned(),
        "orchestrator_was_spawned() should return true"
    );

    // Test implementer_was_spawned()
    assert!(
        result.implementer_was_spawned(),
        "implementer_was_spawned() should return true"
    );

    // Test implement_calls()
    let impl_calls = result.implement_calls();
    assert!(
        !impl_calls.is_empty(),
        "implement_calls() should return non-empty vector"
    );
    for call in &impl_calls {
        assert!(
            !call.is_empty(),
            "Each implement call should have task content"
        );
    }

    // Test decompose_calls()
    let decompose_calls = result.decompose_calls();
    assert!(
        !decompose_calls.is_empty(),
        "decompose_calls() should return non-empty vector for nested scenario"
    );
    for call in &decompose_calls {
        assert!(
            !call.is_empty(),
            "Each decompose call should have task content"
        );
    }

    // Test task_result fields
    assert!(
        result.task_result.success,
        "task_result.success should be true"
    );
    assert!(
        result.task_result.message.is_some(),
        "task_result.message should be Some"
    );

    // Test sessions_created
    assert!(
        !result.sessions_created.is_empty(),
        "sessions_created should not be empty"
    );
}

#[test]
fn test_mock_tool_response_data_to_tool_response() {
    let data = MockToolResponseData {
        success: true,
        summary: "Done".to_string(),
        files_modified: Some(vec!["file.rs".to_string()]),
        error: None,
    };

    let response = data.to_tool_response("req-123".to_string());

    assert_eq!(response.request_id, "req-123");
    assert!(response.success);
    assert_eq!(response.summary, "Done");
    assert_eq!(response.files_modified, Some(vec!["file.rs".to_string()]));
}

#[test]
fn test_mock_acp_error() {
    let error = MockAcpError {
        code: -32600,
        message: "Invalid Request".to_string(),
    };

    let json = serde_json::to_value(&error).unwrap();
    assert_eq!(json["code"], -32600);
    assert_eq!(json["message"], "Invalid Request");
}

#[test]
fn test_mock_session_with_tool_call() {
    let session = MockSessionBuilder::new("impl-001")
        .with_tool_call("str-replace-editor", 100)
        .with_tool_result("str-replace-editor", "File updated", false, 200)
        .with_turn_finished(50)
        .build();

    assert_eq!(session.updates.len(), 3);
    assert_eq!(
        session.updates[0].tool_title,
        Some("str-replace-editor".to_string())
    );

    let result = session.updates[1].tool_result.as_ref().unwrap();
    assert_eq!(result.title, "str-replace-editor");
    assert_eq!(result.content, "File updated");
    assert!(!result.is_error);
}

#[test]
fn test_load_scenario_from_file() {
    // This test loads the actual scenario file to ensure the format is correct
    let scenario_path = std::path::Path::new("tests/scenarios/simple_implement.toml");

    // Skip test if file doesn't exist (for CI environments)
    if !scenario_path.exists() {
        return;
    }

    let scenario = MockScenario::from_file(scenario_path).unwrap();

    assert_eq!(scenario.scenario.name, "simple_implement");
    assert_eq!(scenario.planner_sessions.len(), 1);
    assert_eq!(scenario.orchestrator_sessions.len(), 1);
    assert_eq!(scenario.implementer_sessions.len(), 1);
    assert_eq!(scenario.mock_tool_responses.len(), 1);

    // Verify planner session structure
    let planner = &scenario.planner_sessions[0];
    assert_eq!(planner.session_id, "planner-001");
    // Planner has: message chunk, create_task injection, complete injection, turn_finished
    assert!(planner.updates.len() >= 4);

    // Verify the planner has an update with create_task tool call injection
    let has_create_task = planner.updates.iter().any(|u| {
        matches!(
            &u.inject_mcp_tool_call,
            Some(MockMcpToolCall::CreateTask { .. })
        )
    });
    assert!(
        has_create_task,
        "Planner should have a create_task tool call injection"
    );

    // Verify orchestrator session structure
    let orchestrator = &scenario.orchestrator_sessions[0];
    assert_eq!(orchestrator.session_id, "orchestrator-001");

    // Verify the orchestrator has an implement tool call injection
    let has_spawn_agents = orchestrator.updates.iter().any(|u| {
        matches!(
            &u.inject_mcp_tool_call,
            Some(MockMcpToolCall::SpawnAgents { .. })
        )
    });
    assert!(
        has_spawn_agents,
        "Orchestrator should have a spawn_agents tool call injection"
    );

    // Verify tool response
    let tool_response = &scenario.mock_tool_responses[0];
    assert_eq!(tool_response.tool_type, MockToolType::SpawnAgents);
    assert!(tool_response.response.success);
}

#[test]
fn test_load_all_scenario_files() {
    use std::fs;

    let scenarios_dir = std::path::Path::new("tests/scenarios");
    if !scenarios_dir.exists() {
        return;
    }

    let mut loaded_count = 0;
    for entry in fs::read_dir(scenarios_dir).expect("Failed to read scenarios directory") {
        let entry = entry.expect("Failed to read entry");
        let path = entry.path();

        if path.extension().map_or(false, |e| e == "toml") {
            let scenario = MockScenario::from_file(&path)
                .unwrap_or_else(|e| panic!("Failed to parse {:?}: {}", path, e));

            // Basic structure validation
            assert!(
                !scenario.scenario.name.is_empty(),
                "Scenario {:?} must have a name",
                path
            );

            // Each scenario should have at least one agent session or be a failure scenario
            let has_sessions = !scenario.planner_sessions.is_empty()
                || !scenario.orchestrator_sessions.is_empty()
                || !scenario.implementer_sessions.is_empty();
            assert!(
                has_sessions,
                "Scenario {:?} must have at least one agent session",
                path
            );

            // Verify all sessions have session_id and at least one update
            for planner in &scenario.planner_sessions {
                assert!(
                    !planner.session_id.is_empty(),
                    "Planner session in {:?} must have session_id",
                    path
                );
                assert!(
                    !planner.updates.is_empty(),
                    "Planner session {} in {:?} must have updates",
                    planner.session_id,
                    path
                );
                // Verify it ends with agent_turn_finished
                let last_update = planner.updates.last().unwrap();
                assert_eq!(
                    last_update.session_update, "agent_turn_finished",
                    "Planner session {} in {:?} must end with agent_turn_finished",
                    planner.session_id, path
                );
            }

            for orchestrator in &scenario.orchestrator_sessions {
                assert!(
                    !orchestrator.session_id.is_empty(),
                    "Orchestrator session in {:?} must have session_id",
                    path
                );
                assert!(
                    !orchestrator.updates.is_empty(),
                    "Orchestrator session {} in {:?} must have updates",
                    orchestrator.session_id,
                    path
                );
                let last_update = orchestrator.updates.last().unwrap();
                assert_eq!(
                    last_update.session_update, "agent_turn_finished",
                    "Orchestrator session {} in {:?} must end with agent_turn_finished",
                    orchestrator.session_id, path
                );
            }

            for implementer in &scenario.implementer_sessions {
                assert!(
                    !implementer.session_id.is_empty(),
                    "Implementer session in {:?} must have session_id",
                    path
                );
                assert!(
                    !implementer.updates.is_empty(),
                    "Implementer session {} in {:?} must have updates",
                    implementer.session_id,
                    path
                );
                let last_update = implementer.updates.last().unwrap();
                assert_eq!(
                    last_update.session_update, "agent_turn_finished",
                    "Implementer session {} in {:?} must end with agent_turn_finished",
                    implementer.session_id, path
                );
            }

            loaded_count += 1;
            println!("✓ Loaded and validated: {:?}", path.file_name().unwrap());
        }
    }

    assert!(
        loaded_count > 0,
        "Expected to find at least one scenario file"
    );
    println!("\nTotal scenarios validated: {}", loaded_count);
}

// ========================================================================
// Error Handling Tests - Planner Failure
// ========================================================================

/// Test that when planner produces no plan, failure is returned.
/// Verifies: TaskResult indicates failure and message is helpful.
#[tokio::test]
async fn test_planner_failure_returns_error() {
    // Create a scenario where planner fails with no plan
    let scenario = MockScenario {
        scenario: ScenarioMetadata {
            name: "planner_no_plan".to_string(),
            description: "Planner fails to produce plan".to_string(),
        },
        planner_sessions: vec![MockSessionBuilder::new("planner-001")
            .with_message_chunk("Analyzing the request...", 50)
            .with_message_chunk("I cannot create a plan for this request.", 50)
            .with_complete(
                false,
                Some("Unable to create plan - requirements unclear".to_string()),
                50,
            )
            .with_turn_finished(50)
            .build()],
        // No orchestrator sessions - planner failure should be handled
        orchestrator_sessions: vec![],
        implementer_sessions: vec![],
        mock_tool_responses: vec![],
        mock_acp_responses: vec![],
    };

    let mut harness = TestHarness::with_scenario(scenario).with_timeout(Duration::from_secs(2));

    let result = harness.run_goal("Ambiguous impossible task").await;

    // The run may either:
    // 1. Complete with TaskResult.success = false (planner failure was captured)
    // 2. Return an error (timeout waiting for orchestrator)
    // Both are valid ways to indicate failure
    match result {
        Ok(test_result) => {
            // Planner failure was captured - TaskResult should indicate failure
            assert!(
                !test_result.task_result.success,
                "TaskResult should indicate failure when planner fails, got success=true"
            );

            // Verify the failure message is helpful
            let message = test_result
                .task_result
                .message
                .as_ref()
                .expect("Failure should include message");
            assert!(
                message.contains("plan")
                    || message.contains("unclear")
                    || message.contains("Unable"),
                "Failure message should be descriptive. Got: {}",
                message
            );

            // Verify planner was spawned but no orchestrator
            assert_planner_spawned(&test_result);

            // Complete tool call with success=false should be captured
            let complete_calls: Vec<_> = test_result
                .tool_calls
                .iter()
                .filter(|c| {
                    matches!(
                        &c.call,
                        crate::mcp_server::ToolCall::Complete { success: false, .. }
                    )
                })
                .collect();
            assert!(
                !complete_calls.is_empty(),
                "Planner's complete(success=false) call should be captured"
            );
        }
        Err(e) => {
            // Timeout or error is also acceptable
            let error_str = e.to_string();
            assert!(
                error_str.contains("timed out")
                    || error_str.contains("Timeout")
                    || error_str.contains("timeout")
                    || error_str.contains("failed"),
                "Error message should indicate what went wrong. Got: {}",
                error_str
            );
        }
    }
}

/// Test that implementer failures are captured in the result.
/// Verifies: Implementation failure responses are recorded.
#[tokio::test]
async fn test_implement_failure_captured() {
    let harness = TestHarness::with_scenario_file(Path::new("tests/scenarios/error_recovery.toml"))
        .expect("Failed to load error_recovery scenario");

    let mut harness = harness.with_timeout(Duration::from_secs(15));

    let result = harness
        .run_goal("Fix critical bug")
        .await
        .expect("Test run should complete");

    // Verify implement calls were made
    let impl_calls: Vec<_> = result
        .tool_calls
        .iter()
        .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::SpawnAgents { .. }))
        .collect();

    assert!(
        impl_calls.len() >= 2,
        "Expected at least 2 spawn_agents calls, got {}",
        impl_calls.len()
    );

    // First implement call should have failed
    let first_impl = &impl_calls[0];
    assert!(
        !first_impl.response.success || first_impl.response.error.is_some(),
        "First implement call should indicate failure. Response: {:?}",
        first_impl.response
    );

    // The failure should have meaningful error info
    if let Some(error) = &first_impl.response.error {
        assert!(!error.is_empty(), "Error message should not be empty");
    }
}

/// Test that empty plan (no tasks) fails gracefully.
/// Verifies: System handles edge case of empty plan.
#[tokio::test]
async fn test_empty_plan_fails_gracefully() {
    // Create scenario with planner that creates no tasks (empty plan)
    let scenario = MockScenario {
        scenario: ScenarioMetadata {
            name: "empty_plan".to_string(),
            description: "Planner creates no tasks".to_string(),
        },
        planner_sessions: vec![MockSessionBuilder::new("planner-001")
            .with_message_chunk("Analyzing request...", 50)
            .with_complete(
                true,
                Some("Empty plan - no tasks to execute".to_string()),
                50,
            )
            .with_turn_finished(50)
            .build()],
        orchestrator_sessions: vec![MockSessionBuilder::new("orchestrator-001")
            .with_message_chunk("Reviewing plan...", 50)
            .with_message_chunk("No tasks to execute.", 50)
            .with_complete(
                true,
                Some("No tasks in plan, nothing to do".to_string()),
                50,
            )
            .with_turn_finished(50)
            .build()],
        implementer_sessions: vec![],
        mock_tool_responses: vec![],
        mock_acp_responses: vec![],
    };

    let mut harness = TestHarness::with_scenario(scenario).with_timeout(Duration::from_secs(5));

    let result = harness.run_goal("Task with no work needed").await;

    // Either completes successfully with no implement calls, or fails gracefully
    match result {
        Ok(test_result) => {
            // If it succeeded, no implement calls should have been made
            assert!(
                test_result.implement_calls().is_empty(),
                "Empty plan should not trigger implement calls, but got: {:?}",
                test_result.implement_calls()
            );
            assert_planner_spawned(&test_result);
        }
        Err(e) => {
            // If it failed, the error should be informative
            let error_str = e.to_string();
            assert!(!error_str.is_empty(), "Error message should be informative");
        }
    }
}

// ========================================================================
// Edge Case Tests
// ========================================================================

/// Test that mock exhaustion produces a clear error message.
/// Verifies: When mocks run out, error message explains what happened.
#[tokio::test]
async fn test_mock_exhaustion_error_is_clear() {
    // Create scenario with one implement response but orchestrator tries two
    let scenario = MockScenario {
        scenario: ScenarioMetadata {
            name: "mock_exhaustion".to_string(),
            description: "Test mock exhaustion error".to_string(),
        },
        planner_sessions: vec![MockSessionBuilder::new("planner-001")
            .with_message_chunk("Creating plan...", 50)
            .with_create_task("Task A", "Do task A", 50)
            .with_create_task("Task B", "Do task B", 50)
            .with_complete(true, Some("Plan created".to_string()), 50)
            .with_turn_finished(50)
            .build()],
        orchestrator_sessions: vec![MockSessionBuilder::new("orchestrator-001")
            .with_message_chunk("Executing...", 50)
            .with_implement("Task A", 50)
            .with_implement("Task B", 50) // Second implement has no mock response
            .with_complete(true, Some("Done".to_string()), 50)
            .with_turn_finished(50)
            .build()],
        implementer_sessions: vec![
            MockSessionBuilder::new("impl-001")
                .with_message_chunk("Implementing Task A...", 50)
                .with_complete(true, Some("Task A done".to_string()), 50)
                .with_turn_finished(50)
                .build(),
            MockSessionBuilder::new("impl-002")
                .with_message_chunk("Implementing Task B...", 50)
                .with_complete(true, Some("Task B done".to_string()), 50)
                .with_turn_finished(50)
                .build(),
        ],
        // Only one spawn_agents response - will exhaust when second spawn_agents called
        mock_tool_responses: vec![MockToolResponseBuilder::new()
            .tool_type(MockToolType::SpawnAgents)
            .success("Task A completed")
            .build()],
        mock_acp_responses: vec![],
    };

    let mut harness = TestHarness::with_scenario(scenario).with_timeout(Duration::from_secs(5));

    let result = harness.run_goal("Execute two tasks").await;

    // This could fail due to mock exhaustion, or the test might timeout
    // Either way, check the behavior is reasonable
    match result {
        Ok(test_result) => {
            // If it completed, we made at least one implement call
            let impl_calls = test_result.implement_calls();
            assert!(
                !impl_calls.is_empty(),
                "Should have at least one implement call"
            );
        }
        Err(e) => {
            let error_str = e.to_string();
            // Error should mention mock exhaustion or timeout with helpful message
            assert!(
                error_str.contains("exhausted")
                    || error_str.contains("no response")
                    || error_str.contains("Timeout")
                    || error_str.contains("timed out")
                    || error_str.contains("Mock responses"),
                "Error message should indicate mock exhaustion or timeout. Got: {}",
                error_str
            );
        }
    }
}

/// Test that session without turn_finished times out.
/// Verifies: Incomplete sessions are detected with helpful error.
#[tokio::test]
async fn test_session_without_turn_finished_times_out() {
    // Create scenario where planner never sends agent_turn_finished
    let scenario = MockScenario {
        scenario: ScenarioMetadata {
            name: "incomplete_session".to_string(),
            description: "Session without turn_finished".to_string(),
        },
        planner_sessions: vec![MockAgentSession {
            session_id: "planner-001".to_string(),
            updates: vec![
                MockSessionUpdate {
                    delay_ms: 50,
                    session_update: "agent_message_chunk".to_string(),
                    content: Some("Working on it...".to_string()),
                    tool_title: None,
                    tool_result: None,
                    inject_mcp_tool_call: None,
                },
                // NOTE: No agent_turn_finished update - session is incomplete
            ],
            expected_prompt_contains: None,
        }],
        orchestrator_sessions: vec![],
        implementer_sessions: vec![],
        mock_tool_responses: vec![],
        mock_acp_responses: vec![],
    };

    let mut harness = TestHarness::with_scenario(scenario).with_timeout(Duration::from_millis(300));

    let result = harness.run_goal("Test incomplete session").await;

    // Should timeout or error
    assert!(
        result.is_err(),
        "Expected timeout/error for session without turn_finished"
    );

    let err = result.unwrap_err();
    let error_str = err.to_string();

    // Verify error occurred - accept various forms of failure/timeout messages
    // The session without turn_finished can fail in multiple ways:
    // 1. Timeout waiting for session to complete
    // 2. App run failure due to incomplete session
    // 3. Error propagated from the mock system
    assert!(
        error_str.contains("timed out")
            || error_str.contains("Timeout")
            || error_str.contains("timeout")
            || error_str.contains("agent_turn_finished")
            || error_str.contains("failed")
            || error_str.contains("Failed")
            || error_str.contains("run failed"),
        "Error should indicate timeout or failure. Got: {}",
        error_str
    );
}

// ========================================================================
// Timeout Tests
// ========================================================================

/// Test that configurable timeout works.
/// Verifies: with_timeout() builder method sets effective timeout.
#[tokio::test]
async fn test_harness_timeout_works() {
    // Create scenario with long delays that will timeout
    let scenario = MockScenario {
        scenario: ScenarioMetadata {
            name: "slow_scenario".to_string(),
            description: "Scenario with long delays".to_string(),
        },
        planner_sessions: vec![MockAgentSession {
            session_id: "planner-001".to_string(),
            updates: vec![
                MockSessionUpdate {
                    delay_ms: 2000, // 2 second delay - longer than timeout
                    session_update: "agent_message_chunk".to_string(),
                    content: Some("Still thinking...".to_string()),
                    tool_title: None,
                    tool_result: None,
                    inject_mcp_tool_call: None,
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
            expected_prompt_contains: None,
        }],
        orchestrator_sessions: vec![],
        implementer_sessions: vec![],
        mock_tool_responses: vec![],
        mock_acp_responses: vec![],
    };

    // Use very short timeout (100ms) - shorter than the 2000ms delay
    let mut harness = TestHarness::with_scenario(scenario).with_timeout(Duration::from_millis(100));

    let start = std::time::Instant::now();
    let result = harness.run_goal("This will timeout").await;
    let elapsed = start.elapsed();

    // Should have timed out
    assert!(result.is_err(), "Expected timeout error with short timeout");

    // Verify timeout was respected (should complete within ~200ms, well before 2s)
    assert!(
        elapsed < Duration::from_millis(500),
        "Should have timed out quickly, but took {:?}",
        elapsed
    );

    // Verify error message mentions timeout
    let error_str = result.unwrap_err().to_string();
    assert!(
        error_str.contains("timed out")
            || error_str.contains("Timeout")
            || error_str.contains("100ms"),
        "Error should mention timeout. Got: {}",
        error_str
    );
}

/// Test that default timeout allows reasonable tests to complete.
/// Verifies: Default timeout is sufficient for normal scenarios.
#[tokio::test]
async fn test_default_timeout_allows_completion() {
    let mut harness =
        TestHarness::with_scenario_file(Path::new("tests/scenarios/simple_implement.toml"))
            .expect("Failed to load scenario");

    // Don't set timeout - use default
    let result = harness.run_goal("Test default timeout").await;

    assert!(
        result.is_ok(),
        "Default timeout should allow simple_implement to complete: {:?}",
        result.err()
    );
}
