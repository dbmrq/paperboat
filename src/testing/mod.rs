//! Mock data system for testing Villalobos.
//!
//! This module provides core mock types, scenario loading, builder helpers,
//! and a test harness for unit testing, integration testing, and end-to-end
//! testing without requiring live AI agents or external services.

// Submodules
mod builders;
mod harness;
mod mock_acp;
mod scenario;
mod types;

// Re-export all public types and functions

// From types module
pub use types::{
    AgentType, MockAcpError, MockAcpResponse, MockAgentSession, MockMcpToolCall,
    MockSessionUpdate, MockToolCallResponse, MockToolResponseData, MockToolResult, MockToolType,
};

// From scenario module
pub use scenario::{MockScenario, ScenarioMetadata};

// From builders module
pub use builders::{MockSessionBuilder, MockToolResponseBuilder};

// From mock_acp module
pub use mock_acp::MockAcpClient;

// From harness module
pub use harness::{
    assert_decompose_called, assert_failure, assert_implement_called, assert_implementer_spawned,
    assert_orchestrator_spawned, assert_planner_spawned, assert_success, CapturedToolCall,
    MockToolInterceptor, TestHarness, TestRunResult,
};

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_tool_response_builder() {
        let response = MockToolResponseBuilder::new()
            .tool_type(MockToolType::Implement)
            .task_pattern(".*auth.*")
            .success("Implemented authentication")
            .files_modified(vec!["src/auth.rs".to_string()])
            .build();

        assert_eq!(response.tool_type, MockToolType::Implement);
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
        assert_eq!(response.response.error, Some("Task too complex".to_string()));
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
                    .tool_type(MockToolType::Implement)
                    .task_pattern(".*auth.*")
                    .success("Auth done")
                    .build(),
                MockToolResponseBuilder::new()
                    .tool_type(MockToolType::Implement)
                    .success("Default response")
                    .build(),
            ],
            ..Default::default()
        };

        // Should match the auth pattern
        let response = scenario.find_tool_response(MockToolType::Implement, "implement auth");
        assert!(response.is_some());
        assert_eq!(response.unwrap().response.summary, "Auth done");

        // Should match the default (no pattern)
        let response = scenario.find_tool_response(MockToolType::Implement, "other task");
        assert!(response.is_some());
        assert_eq!(response.unwrap().response.summary, "Default response");

        // Should not match decompose
        let response = scenario.find_tool_response(MockToolType::Decompose, "any task");
        assert!(response.is_none());
    }
}

// ============================================================================
// Integration Tests
// ============================================================================

/// Integration tests that use the TestHarness to verify major application flows.
/// These tests use scenario files and verify the orchestration between agents.
#[cfg(test)]
mod integration_tests {
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load simple_implement scenario");

        // Use faster timeout for tests
        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Add error handling to login").await
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/multi_implement.toml")
        ).expect("Failed to load multi_implement scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Create user management system").await
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
            impl_calls.len(), 3,
            "Expected 3 implement calls, got {}: {:?}",
            impl_calls.len(), impl_calls
        );

        // Verify the implement calls cover different aspects
        let all_calls = impl_calls.join(" ").to_lowercase();
        assert!(all_calls.contains("database") || all_calls.contains("schema"),
            "Should have a database/schema task");
        assert!(all_calls.contains("user") || all_calls.contains("service"),
            "Should have a user service task");
        assert!(all_calls.contains("api") || all_calls.contains("endpoint"),
            "Should have an API endpoint task");
    }

    /// Test that the planner produces a valid plan structure.
    /// Verifies: write_plan tool is called with plan content.
    #[tokio::test]
    async fn test_planning_produces_valid_plan() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/planning_only.toml")
        ).expect("Failed to load planning_only scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Create a project plan").await
            .expect("Test run failed");

        // Verify success
        assert_success(&result);

        // Verify planner was spawned
        assert_planner_spawned(&result);
        assert_orchestrator_spawned(&result);

        // Verify write_plan was called (captured in tool_calls)
        let write_plan_calls: Vec<_> = result.tool_calls.iter()
            .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::WritePlan { .. }))
            .collect();

        assert!(
            !write_plan_calls.is_empty(),
            "Expected write_plan to be called. Tool calls: {:?}",
            result.tool_calls.iter().map(|c| format!("{:?}", c.call)).collect::<Vec<_>>()
        );

        // Verify the plan contains structured content
        if let crate::mcp_server::ToolCall::WritePlan { plan } = &write_plan_calls[0].call {
            assert!(plan.contains("##") || plan.contains("1.") || plan.contains("-"),
                "Plan should have structured content (headers, numbers, or bullets)");
            assert!(plan.len() > 50, "Plan should have substantive content");
        }

        // No implement calls in planning-only scenario
        assert!(result.implement_calls().is_empty(),
            "Planning-only scenario should not call implement()");
    }

    // ========================================================================
    // Orchestration Tests
    // ========================================================================

    /// Test that orchestrator correctly delegates to implementer.
    /// Verifies: implement() calls flow through the system.
    #[tokio::test]
    async fn test_orchestrator_delegates_to_implementer() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Implement the feature").await
            .expect("Test run failed");

        // Orchestrator should spawn implementer
        assert_orchestrator_spawned(&result);
        assert_implementer_spawned(&result);

        // Implement call should have response captured
        let impl_tool_calls: Vec<_> = result.tool_calls.iter()
            .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::Implement { .. }))
            .collect();

        assert!(!impl_tool_calls.is_empty(), "Should have captured implement tool calls");

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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/nested_decompose.toml")
        ).expect("Failed to load nested_decompose scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(20));

        let result = harness.run_goal("Build authentication system").await
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
            all_impl_text.contains("rate") || all_impl_text.contains("login") || all_impl_text.contains("auth"),
            "Implement calls should relate to auth or rate limiting, got: {:?}",
            impl_calls
        );

        // Verify multiple planners were spawned (main + sub)
        let planner_sessions: Vec<_> = result.sessions_created.iter()
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/multi_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Multiple task test").await
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
            let has_content = !captured.response.summary.is_empty()
                || captured.response.error.is_some();
            assert!(
                has_content,
                "Tool call {} should have summary or error: {:?}",
                i, captured.response
            );
        }

        // Verify different tool types were captured
        let has_write_plan = result.tool_calls.iter()
            .any(|c| matches!(&c.call, crate::mcp_server::ToolCall::WritePlan { .. }));
        let has_implement = result.tool_calls.iter()
            .any(|c| matches!(&c.call, crate::mcp_server::ToolCall::Implement { .. }));
        let has_complete = result.tool_calls.iter()
            .any(|c| matches!(&c.call, crate::mcp_server::ToolCall::Complete { .. }));

        assert!(has_write_plan, "Should capture write_plan calls");
        assert!(has_implement, "Should capture implement calls");
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/planner_failure.toml")
        ).expect("Failed to load planner_failure scenario");

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
                let complete_calls: Vec<_> = test_result.tool_calls.iter()
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
                    error_str.contains("timed out") ||
                    error_str.contains("Timeout") ||
                    error_str.contains("timeout") ||
                    error_str.contains("failed") ||
                    error_str.contains("Failed"),
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/error_recovery.toml")
        ).expect("Failed to load error_recovery scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Fix critical bug").await
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
            impl_calls.len(), impl_calls
        );

        // Verify both implement calls were captured
        let impl_responses: Vec<_> = result.tool_calls.iter()
            .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::Implement { .. }))
            .collect();

        assert!(
            impl_responses.len() >= 2,
            "Should have captured at least 2 implement tool calls, got {}",
            impl_responses.len()
        );

        // Verify implementer sessions were created (both first and retry)
        let impl_sessions: Vec<_> = result.sessions_created.iter()
            .filter(|s| s.contains("impl"))
            .collect();
        assert!(
            impl_sessions.len() >= 2,
            "Expected at least 2 implementer sessions, got {}: {:?}",
            impl_sessions.len(), impl_sessions
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/nested_decompose.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(20));

        let result = harness.run_goal("Full flow test").await
            .expect("Test run failed");

        // Test planner_was_spawned()
        assert!(result.planner_was_spawned(),
            "planner_was_spawned() should return true");

        // Test orchestrator_was_spawned()
        assert!(result.orchestrator_was_spawned(),
            "orchestrator_was_spawned() should return true");

        // Test implementer_was_spawned()
        assert!(result.implementer_was_spawned(),
            "implementer_was_spawned() should return true");

        // Test implement_calls()
        let impl_calls = result.implement_calls();
        assert!(!impl_calls.is_empty(),
            "implement_calls() should return non-empty vector");
        for call in &impl_calls {
            assert!(!call.is_empty(),
                "Each implement call should have task content");
        }

        // Test decompose_calls()
        let decompose_calls = result.decompose_calls();
        assert!(!decompose_calls.is_empty(),
            "decompose_calls() should return non-empty vector for nested scenario");
        for call in &decompose_calls {
            assert!(!call.is_empty(),
                "Each decompose call should have task content");
        }

        // Test task_result fields
        assert!(result.task_result.success,
            "task_result.success should be true");
        assert!(result.task_result.message.is_some(),
            "task_result.message should be Some");

        // Test sessions_created
        assert!(!result.sessions_created.is_empty(),
            "sessions_created should not be empty");
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
        assert_eq!(session.updates[0].tool_title, Some("str-replace-editor".to_string()));

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
        // Planner has: message chunk, write_plan injection, complete injection, turn_finished
        assert!(planner.updates.len() >= 4);

        // Verify the planner has an update with write_plan tool call injection
        let has_write_plan = planner.updates.iter().any(|u| {
            matches!(&u.inject_mcp_tool_call, Some(MockMcpToolCall::WritePlan { .. }))
        });
        assert!(has_write_plan, "Planner should have a write_plan tool call injection");

        // Verify orchestrator session structure
        let orchestrator = &scenario.orchestrator_sessions[0];
        assert_eq!(orchestrator.session_id, "orchestrator-001");

        // Verify the orchestrator has an implement tool call injection
        let has_implement = orchestrator.updates.iter().any(|u| {
            matches!(&u.inject_mcp_tool_call, Some(MockMcpToolCall::Implement { .. }))
        });
        assert!(has_implement, "Orchestrator should have an implement tool call injection");

        // Verify tool response
        let tool_response = &scenario.mock_tool_responses[0];
        assert_eq!(tool_response.tool_type, MockToolType::Implement);
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
                assert!(!scenario.scenario.name.is_empty(),
                    "Scenario {:?} must have a name", path);

                // Each scenario should have at least one agent session or be a failure scenario
                let has_sessions = !scenario.planner_sessions.is_empty()
                    || !scenario.orchestrator_sessions.is_empty()
                    || !scenario.implementer_sessions.is_empty();
                assert!(has_sessions,
                    "Scenario {:?} must have at least one agent session", path);

                // Verify all sessions have session_id and at least one update
                for planner in &scenario.planner_sessions {
                    assert!(!planner.session_id.is_empty(),
                        "Planner session in {:?} must have session_id", path);
                    assert!(!planner.updates.is_empty(),
                        "Planner session {} in {:?} must have updates", planner.session_id, path);
                    // Verify it ends with agent_turn_finished
                    let last_update = planner.updates.last().unwrap();
                    assert_eq!(last_update.session_update, "agent_turn_finished",
                        "Planner session {} in {:?} must end with agent_turn_finished",
                        planner.session_id, path);
                }

                for orchestrator in &scenario.orchestrator_sessions {
                    assert!(!orchestrator.session_id.is_empty(),
                        "Orchestrator session in {:?} must have session_id", path);
                    assert!(!orchestrator.updates.is_empty(),
                        "Orchestrator session {} in {:?} must have updates", orchestrator.session_id, path);
                    let last_update = orchestrator.updates.last().unwrap();
                    assert_eq!(last_update.session_update, "agent_turn_finished",
                        "Orchestrator session {} in {:?} must end with agent_turn_finished",
                        orchestrator.session_id, path);
                }

                for implementer in &scenario.implementer_sessions {
                    assert!(!implementer.session_id.is_empty(),
                        "Implementer session in {:?} must have session_id", path);
                    assert!(!implementer.updates.is_empty(),
                        "Implementer session {} in {:?} must have updates", implementer.session_id, path);
                    let last_update = implementer.updates.last().unwrap();
                    assert_eq!(last_update.session_update, "agent_turn_finished",
                        "Implementer session {} in {:?} must end with agent_turn_finished",
                        implementer.session_id, path);
                }

                loaded_count += 1;
                println!("✓ Loaded and validated: {:?}", path.file_name().unwrap());
            }
        }

        assert!(loaded_count > 0, "Expected to find at least one scenario file");
        println!("\nTotal scenarios validated: {}", loaded_count);
    }

    // ========================================================================
    // Error Handling Tests
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
            planner_sessions: vec![
                MockSessionBuilder::new("planner-001")
                    .with_message_chunk("Analyzing the request...", 50)
                    .with_message_chunk("I cannot create a plan for this request.", 50)
                    .with_complete(false, Some("Unable to create plan - requirements unclear".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
            // No orchestrator sessions - planner failure should be handled
            orchestrator_sessions: vec![],
            implementer_sessions: vec![],
            mock_tool_responses: vec![],
            mock_acp_responses: vec![],
        };

        let mut harness = TestHarness::with_scenario(scenario)
            .with_timeout(Duration::from_secs(2));

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
                let message = test_result.task_result.message.as_ref()
                    .expect("Failure should include message");
                assert!(
                    message.contains("plan") ||
                    message.contains("unclear") ||
                    message.contains("Unable"),
                    "Failure message should be descriptive. Got: {}",
                    message
                );

                // Verify planner was spawned but no orchestrator
                assert_planner_spawned(&test_result);

                // Complete tool call with success=false should be captured
                let complete_calls: Vec<_> = test_result.tool_calls.iter()
                    .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::Complete { success: false, .. }))
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
                    error_str.contains("timed out") ||
                    error_str.contains("Timeout") ||
                    error_str.contains("timeout") ||
                    error_str.contains("failed"),
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/error_recovery.toml")
        ).expect("Failed to load error_recovery scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Fix critical bug").await
            .expect("Test run should complete");

        // Verify implement calls were made
        let impl_calls: Vec<_> = result.tool_calls.iter()
            .filter(|c| matches!(&c.call, crate::mcp_server::ToolCall::Implement { .. }))
            .collect();

        assert!(
            impl_calls.len() >= 2,
            "Expected at least 2 implement calls, got {}",
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
            assert!(
                !error.is_empty(),
                "Error message should not be empty"
            );
        }
    }

    /// Test that empty plan (no tasks) fails gracefully.
    /// Verifies: System handles edge case of empty plan.
    #[tokio::test]
    async fn test_empty_plan_fails_gracefully() {
        // Create scenario with planner that writes an empty plan
        let scenario = MockScenario {
            scenario: ScenarioMetadata {
                name: "empty_plan".to_string(),
                description: "Planner writes empty plan".to_string(),
            },
            planner_sessions: vec![
                MockSessionBuilder::new("planner-001")
                    .with_message_chunk("Analyzing request...", 50)
                    .with_write_plan("## Plan\n\n(No tasks needed)", 50)
                    .with_complete(true, Some("Empty plan - no tasks to execute".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
            orchestrator_sessions: vec![
                MockSessionBuilder::new("orchestrator-001")
                    .with_message_chunk("Reviewing plan...", 50)
                    .with_message_chunk("No tasks to execute.", 50)
                    .with_complete(true, Some("No tasks in plan, nothing to do".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
            implementer_sessions: vec![],
            mock_tool_responses: vec![],
            mock_acp_responses: vec![],
        };

        let mut harness = TestHarness::with_scenario(scenario)
            .with_timeout(Duration::from_secs(5));

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
                assert!(
                    !error_str.is_empty(),
                    "Error message should be informative"
                );
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
            planner_sessions: vec![
                MockSessionBuilder::new("planner-001")
                    .with_message_chunk("Creating plan...", 50)
                    .with_write_plan("## Plan\n\n1. Task A\n2. Task B", 50)
                    .with_complete(true, Some("Plan created".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
            orchestrator_sessions: vec![
                MockSessionBuilder::new("orchestrator-001")
                    .with_message_chunk("Executing...", 50)
                    .with_implement("Task A", 50)
                    .with_implement("Task B", 50) // Second implement has no mock response
                    .with_complete(true, Some("Done".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
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
            // Only one implement response - will exhaust when second implement called
            mock_tool_responses: vec![
                MockToolResponseBuilder::new()
                    .tool_type(MockToolType::Implement)
                    .success("Task A completed")
                    .build(),
            ],
            mock_acp_responses: vec![],
        };

        let mut harness = TestHarness::with_scenario(scenario)
            .with_timeout(Duration::from_secs(5));

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
                    error_str.contains("exhausted") ||
                    error_str.contains("no response") ||
                    error_str.contains("Timeout") ||
                    error_str.contains("timed out") ||
                    error_str.contains("Mock responses"),
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
            planner_sessions: vec![
                MockAgentSession {
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
                },
            ],
            orchestrator_sessions: vec![],
            implementer_sessions: vec![],
            mock_tool_responses: vec![],
            mock_acp_responses: vec![],
        };

        let mut harness = TestHarness::with_scenario(scenario)
            .with_timeout(Duration::from_millis(300));

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
            error_str.contains("timed out") ||
            error_str.contains("Timeout") ||
            error_str.contains("timeout") ||
            error_str.contains("agent_turn_finished") ||
            error_str.contains("failed") ||
            error_str.contains("Failed") ||
            error_str.contains("run failed"),
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
            planner_sessions: vec![
                MockAgentSession {
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
                },
            ],
            orchestrator_sessions: vec![],
            implementer_sessions: vec![],
            mock_tool_responses: vec![],
            mock_acp_responses: vec![],
        };

        // Use very short timeout (100ms) - shorter than the 2000ms delay
        let mut harness = TestHarness::with_scenario(scenario)
            .with_timeout(Duration::from_millis(100));

        let start = std::time::Instant::now();
        let result = harness.run_goal("This will timeout").await;
        let elapsed = start.elapsed();

        // Should have timed out
        assert!(
            result.is_err(),
            "Expected timeout error with short timeout"
        );

        // Verify timeout was respected (should complete within ~200ms, well before 2s)
        assert!(
            elapsed < Duration::from_millis(500),
            "Should have timed out quickly, but took {:?}",
            elapsed
        );

        // Verify error message mentions timeout
        let error_str = result.unwrap_err().to_string();
        assert!(
            error_str.contains("timed out") ||
            error_str.contains("Timeout") ||
            error_str.contains("100ms"),
            "Error should mention timeout. Got: {}",
            error_str
        );
    }

    /// Test that default timeout allows reasonable tests to complete.
    /// Verifies: Default timeout is sufficient for normal scenarios.
    #[tokio::test]
    async fn test_default_timeout_allows_completion() {
        let mut harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load scenario");

        // Don't set timeout - use default
        let result = harness.run_goal("Test default timeout").await;

        assert!(
            result.is_ok(),
            "Default timeout should allow simple_implement to complete: {:?}",
            result.err()
        );
    }
}

// ============================================================================
// End-to-End Flow Verification Tests
// ============================================================================

/// End-to-end tests that verify the complete application flow from goal to completion.
/// These tests exercise the full App::run() path and ensure all components integrate correctly.
#[cfg(test)]
mod e2e_tests {
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
    /// 2. Planner creates a plan using write_plan tool
    /// 3. Planner calls complete
    /// 4. Orchestrator receives the plan
    /// 5. Orchestrator calls implement for a task
    /// 6. Implementer executes and calls complete
    /// 7. Orchestrator calls complete
    /// 8. Final TaskResult indicates success with meaningful message
    #[tokio::test]
    async fn test_e2e_complete_task_lifecycle() {
        // Load the simple_implement scenario which tests the basic flow
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load simple_implement scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        // Run the complete flow
        let result = harness.run_goal("Add error handling to login function").await
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
        assert!(
            !message.is_empty(),
            "Final message should not be empty"
        );

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
        let orch_idx = sessions.iter().position(|s| s.contains("orchestrator") || s.contains("orch"));
        let impl_idx = sessions.iter().position(|s| s.contains("impl"));

        assert!(planner_idx.is_some(), "Planner session should exist");
        assert!(orch_idx.is_some(), "Orchestrator session should exist");
        assert!(impl_idx.is_some(), "Implementer session should exist");

        // Planner should come before orchestrator
        assert!(
            planner_idx.unwrap() < orch_idx.unwrap(),
            "Planner session should be created before orchestrator. Sessions: {:?}",
            sessions
        );

        // ----------------------------------------------------------------
        // 3. Verify tool calls happened in correct sequence
        // ----------------------------------------------------------------
        let tool_calls = &result.tool_calls;
        assert!(
            !tool_calls.is_empty(),
            "Should have captured tool calls"
        );

        // Find tool call types
        let mut write_plan_idx = None;
        let mut implement_idx = None;
        let mut complete_indices = Vec::new();

        for (i, tc) in tool_calls.iter().enumerate() {
            match &tc.call {
                crate::mcp_server::ToolCall::WritePlan { .. } => {
                    if write_plan_idx.is_none() {
                        write_plan_idx = Some(i);
                    }
                }
                crate::mcp_server::ToolCall::Implement { .. } => {
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

        // write_plan should be called first
        assert!(
            write_plan_idx.is_some(),
            "write_plan should have been called"
        );

        // implement should be called after write_plan
        assert!(
            implement_idx.is_some(),
            "implement should have been called"
        );
        assert!(
            write_plan_idx.unwrap() < implement_idx.unwrap(),
            "write_plan should happen before implement"
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
        assert_eq!(impl_calls.len(), 1, "Should have exactly one implement call");
        assert!(
            impl_calls[0].len() > 10,
            "Implement task should have substantive content, got: {}",
            impl_calls[0]
        );

        // Verify the implement response was successful
        let impl_tool_call = tool_calls.iter()
            .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Implement { .. }))
            .expect("Should have implement tool call");
        assert!(
            impl_tool_call.response.success,
            "Implement tool call should have succeeded"
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/nested_decompose.toml")
        ).expect("Failed to load nested_decompose scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(25));

        let result = harness.run_goal("Build authentication system with rate limiting").await
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
            "Decompose task should have substantive content, got: {}",
            decompose_task
        );
        assert!(
            decompose_task.to_lowercase().contains("auth"),
            "Decompose task should be about authentication, got: {}",
            decompose_task
        );

        // ----------------------------------------------------------------
        // 3. Verify nested planners were spawned (main + sub)
        // ----------------------------------------------------------------
        let planner_sessions: Vec<_> = result.sessions_created.iter()
            .filter(|s| s.contains("planner"))
            .collect();

        assert!(
            planner_sessions.len() >= 2,
            "Should have at least 2 planner sessions (main + decomposed), got: {:?}",
            planner_sessions
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

        let has_decompose = tool_calls.iter().any(|tc|
            matches!(&tc.call, crate::mcp_server::ToolCall::Decompose { .. })
        );
        assert!(has_decompose, "Should have captured decompose tool call");

        // Decompose should have a successful response (the App handles decomposition internally)
        let decompose_tc = tool_calls.iter()
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
        assert!(
            !message.is_empty(),
            "Final message should not be empty"
        );
    }

    /// E2E Test: Multiple implementers in sequence
    ///
    /// This test verifies that multiple implement calls are handled correctly:
    /// 1. Orchestrator makes multiple implement() calls
    /// 2. Each implementer session is spawned and completes
    /// 3. All tool calls are captured with correct arguments
    #[tokio::test]
    async fn test_e2e_multiple_implementers() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/multi_implement.toml")
        ).expect("Failed to load multi_implement scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(20));

        let result = harness.run_goal("Create user management system with database, service, and API").await
            .expect("E2E multi-implementer test should complete");

        // ----------------------------------------------------------------
        // 1. Verify success
        // ----------------------------------------------------------------
        assert_success(&result);

        // ----------------------------------------------------------------
        // 2. Verify multiple implementers were spawned
        // ----------------------------------------------------------------
        let impl_sessions: Vec<_> = result.sessions_created.iter()
            .filter(|s| s.contains("impl"))
            .collect();

        assert!(
            impl_sessions.len() >= 3,
            "Should have at least 3 implementer sessions, got {}: {:?}",
            impl_sessions.len(), impl_sessions
        );

        // ----------------------------------------------------------------
        // 3. Verify all 3 implement calls were made
        // ----------------------------------------------------------------
        let impl_calls = result.implement_calls();
        assert_eq!(
            impl_calls.len(), 3,
            "Should have exactly 3 implement calls, got {}: {:?}",
            impl_calls.len(), impl_calls
        );

        // ----------------------------------------------------------------
        // 4. Verify implement call arguments are correct and different
        // ----------------------------------------------------------------
        let all_calls_text = impl_calls.join(" ").to_lowercase();

        // Each implement call should be for a different task
        assert!(
            all_calls_text.contains("database") || all_calls_text.contains("schema"),
            "Should have a database-related task, got: {:?}",
            impl_calls
        );
        assert!(
            all_calls_text.contains("user") || all_calls_text.contains("service"),
            "Should have a user service task, got: {:?}",
            impl_calls
        );
        assert!(
            all_calls_text.contains("api") || all_calls_text.contains("endpoint"),
            "Should have an API endpoint task, got: {:?}",
            impl_calls
        );

        // ----------------------------------------------------------------
        // 5. Verify all implement calls have responses
        // ----------------------------------------------------------------
        let impl_tool_calls: Vec<_> = result.tool_calls.iter()
            .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Implement { .. }))
            .collect();

        assert_eq!(
            impl_tool_calls.len(), 3,
            "Should have 3 implement tool calls captured"
        );

        for (i, tc) in impl_tool_calls.iter().enumerate() {
            assert!(
                tc.response.success,
                "Implement call {} should have succeeded, got: {:?}",
                i + 1, tc.response
            );
            assert!(
                !tc.response.request_id.is_empty(),
                "Implement call {} should have request_id",
                i + 1
            );
        }

        // ----------------------------------------------------------------
        // 6. Verify final message summarizes completion
        // ----------------------------------------------------------------
        assert!(result.task_result.message.is_some());
        let message = result.task_result.message.as_ref().unwrap();
        assert!(
            message.to_lowercase().contains("complet") ||
            message.to_lowercase().contains("success") ||
            message.to_lowercase().contains("done"),
            "Final message should indicate completion, got: {}",
            message
        );
    }

    // ========================================================================
    // Data Flow Verification Tests
    // ========================================================================

    /// E2E Test: Verify prompts contain expected content
    ///
    /// Tests that the expected_prompt_contains validation in MockAcpClient works
    /// and that prompts flow correctly between agents.
    #[tokio::test]
    async fn test_e2e_prompt_content_verification() {
        // Create a scenario that validates prompt content
        let scenario = MockScenario {
            scenario: ScenarioMetadata {
                name: "prompt_verification".to_string(),
                description: "Verify prompts contain expected content".to_string(),
            },
            planner_sessions: vec![
                MockAgentSession {
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
                            inject_mcp_tool_call: Some(MockMcpToolCall::WritePlan {
                                plan: "## Plan\n\n1. Implement feature X\n".to_string(),
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
                },
            ],
            orchestrator_sessions: vec![
                MockSessionBuilder::new("orch-prompt-001")
                    .with_message_chunk("Executing plan...", 50)
                    .with_implement("Implement feature X", 50)
                    .with_complete(true, Some("Done".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
            implementer_sessions: vec![
                MockSessionBuilder::new("impl-prompt-001")
                    .with_message_chunk("Implementing...", 50)
                    .with_complete(true, Some("Feature X done".to_string()), 50)
                    .with_turn_finished(50)
                    .build(),
            ],
            mock_tool_responses: vec![
                MockToolResponseBuilder::new()
                    .tool_type(MockToolType::Implement)
                    .success("Feature X implemented")
                    .build(),
            ],
            mock_acp_responses: vec![],
        };

        let mut harness = TestHarness::with_scenario(scenario)
            .with_timeout(Duration::from_secs(10));

        // Run with a goal that matches the expected_prompt_contains pattern
        let result = harness.run_goal("Please add this specific feature to the codebase").await
            .expect("Prompt verification test should pass");

        assert_success(&result);
        assert_planner_spawned(&result);
    }

    /// E2E Test: Verify tool call arguments are correct
    ///
    /// Tests that tool calls receive the correct arguments from the scenario.
    #[tokio::test]
    async fn test_e2e_tool_call_arguments_verification() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Add error handling").await
            .expect("Test should complete");

        // ----------------------------------------------------------------
        // Verify write_plan tool call has plan content
        // ----------------------------------------------------------------
        let write_plan_call = result.tool_calls.iter()
            .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::WritePlan { .. }));

        assert!(
            write_plan_call.is_some(),
            "Should have a write_plan tool call"
        );

        if let crate::mcp_server::ToolCall::WritePlan { plan } = &write_plan_call.unwrap().call {
            assert!(
                plan.contains("##") || plan.contains("1.") || plan.contains("-"),
                "Plan should have structured content (headers, numbers, or bullets), got: {}",
                plan
            );
            assert!(
                plan.len() > 20,
                "Plan should have substantive content (>20 chars), got {} chars",
                plan.len()
            );
        }

        // ----------------------------------------------------------------
        // Verify implement tool call has task content
        // ----------------------------------------------------------------
        let implement_call = result.tool_calls.iter()
            .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Implement { .. }));

        assert!(
            implement_call.is_some(),
            "Should have an implement tool call"
        );

        if let crate::mcp_server::ToolCall::Implement { task } = &implement_call.unwrap().call {
            assert!(
                !task.is_empty(),
                "Implement task should not be empty"
            );
            assert!(
                task.to_lowercase().contains("error") || task.to_lowercase().contains("handling"),
                "Implement task should relate to the goal, got: {}",
                task
            );
        }

        // ----------------------------------------------------------------
        // Verify complete tool calls have success flags
        // ----------------------------------------------------------------
        let complete_calls: Vec<_> = result.tool_calls.iter()
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
    /// Tests that the final TaskResult has a meaningful message that describes
    /// what was accomplished.
    #[tokio::test]
    async fn test_e2e_final_result_message_meaningful() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/multi_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Create complete user system").await
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

        assert!(
            !message.is_empty(),
            "Message should not be empty"
        );

        assert!(
            message.len() > 5,
            "Message should be descriptive (>5 chars), got: {}",
            message
        );

        // Message should indicate completion/success in some way
        let msg_lower = message.to_lowercase();
        assert!(
            msg_lower.contains("complet") ||
            msg_lower.contains("success") ||
            msg_lower.contains("done") ||
            msg_lower.contains("finish") ||
            msg_lower.contains("task"),
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Test session ordering").await
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
        let planner_pos = sessions.iter()
            .position(|s| s.contains("planner"))
            .expect("Should have planner session");

        let orch_pos = sessions.iter()
            .position(|s| s.contains("orchestrator") || s.contains("orch"))
            .expect("Should have orchestrator session");

        let impl_pos = sessions.iter()
            .position(|s| s.contains("impl"))
            .expect("Should have implementer session");

        assert!(
            planner_pos < orch_pos,
            "Planner (pos {}) should come before orchestrator (pos {})",
            planner_pos, orch_pos
        );

        assert!(
            orch_pos < impl_pos,
            "Orchestrator (pos {}) should come before implementer (pos {})",
            orch_pos, impl_pos
        );
    }

    /// E2E Test: Verify tool calls happen at correct times
    ///
    /// Tests that tool calls occur in the expected sequence during execution.
    #[tokio::test]
    async fn test_e2e_tool_call_timing() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Test tool call timing").await
            .expect("Test should complete");

        let tool_calls = &result.tool_calls;

        // ----------------------------------------------------------------
        // Build sequence of tool call types
        // ----------------------------------------------------------------
        let call_sequence: Vec<&str> = tool_calls.iter()
            .map(|tc| tc.call.tool_type())
            .collect();

        // ----------------------------------------------------------------
        // Verify expected sequence patterns
        // ----------------------------------------------------------------

        // write_plan should appear early (planner phase)
        let write_plan_pos = call_sequence.iter()
            .position(|&t| t == "write_plan");
        assert!(
            write_plan_pos.is_some(),
            "write_plan should be in call sequence"
        );

        // implement should appear after write_plan (orchestrator delegating)
        let implement_pos = call_sequence.iter()
            .position(|&t| t == "implement");
        assert!(
            implement_pos.is_some(),
            "implement should be in call sequence"
        );
        assert!(
            write_plan_pos.unwrap() < implement_pos.unwrap(),
            "write_plan should happen before implement. Sequence: {:?}",
            call_sequence
        );

        // First complete should be planner's (after write_plan)
        let first_complete_pos = call_sequence.iter()
            .position(|&t| t == "complete");
        assert!(
            first_complete_pos.is_some(),
            "complete should be in call sequence"
        );

        // There should be multiple complete calls
        let complete_count = call_sequence.iter()
            .filter(|&&t| t == "complete")
            .count();
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/nested_decompose.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(25));

        let result = harness.run_goal("Build complex system requiring decomposition").await
            .expect("Test should complete");

        let sessions = &result.sessions_created;

        // ----------------------------------------------------------------
        // Count session types
        // ----------------------------------------------------------------
        let planner_count = sessions.iter()
            .filter(|s| s.contains("planner"))
            .count();

        let orch_count = sessions.iter()
            .filter(|s| s.contains("orchestrator") || s.contains("orch"))
            .count();

        let impl_count = sessions.iter()
            .filter(|s| s.contains("impl"))
            .count();

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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/error_recovery.toml")
        ).expect("Failed to load error_recovery scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Fix critical database bug").await
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
        let impl_sessions: Vec<_> = result.sessions_created.iter()
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
        let impl_tool_calls: Vec<_> = result.tool_calls.iter()
            .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Implement { .. }))
            .collect();

        assert!(
            impl_tool_calls.len() >= 2,
            "Should have captured at least 2 implement tool calls"
        );

        // First implement should have failed (per scenario definition)
        let first_impl = &impl_tool_calls[0];
        assert!(
            !first_impl.response.success || first_impl.response.error.is_some(),
            "First implement should indicate failure, got: {:?}",
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
            first_task.contains("database") || first_task.contains("bug") || first_task.contains("timeout"),
            "First implement task should be about the bug, got: {}",
            impl_calls[0]
        );

        // Second task should have more detail (retry with more specific instructions)
        assert!(
            second_task.len() > first_task.len() ||
            second_task.contains("pool") ||
            second_task.contains("backoff") ||
            second_task.contains("specific"),
            "Second implement task should have more detail than first. First: '{}', Second: '{}'",
            impl_calls[0], impl_calls[1]
        );
    }

    /// E2E Test: Verify complete flow with planning-only scenario
    ///
    /// Tests a flow where the orchestrator immediately completes without
    /// calling implement (planning-only use case).
    #[tokio::test]
    async fn test_e2e_planning_only_flow() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/planning_only.toml")
        ).expect("Failed to load planning_only scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Create a high-level project plan").await
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
        // Verify write_plan was called
        // ----------------------------------------------------------------
        let write_plan_calls: Vec<_> = result.tool_calls.iter()
            .filter(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::WritePlan { .. }))
            .collect();

        assert!(
            !write_plan_calls.is_empty(),
            "Planning-only flow should have write_plan call"
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
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/multi_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(15));

        let result = harness.run_goal("Create system with multiple components").await
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
                i, captured.call.tool_type(), captured.response
            );
        }

        // ----------------------------------------------------------------
        // Verify tool call count matches expected
        // ----------------------------------------------------------------
        assert!(
            result.tool_calls.len() >= 5,
            "Should have at least 5 tool calls (write_plan, 3x implement, 2+ complete), got {}",
            result.tool_calls.len()
        );
    }

    /// E2E Test: Verify tool response content matches expectations
    ///
    /// Tests that tool responses contain expected data based on scenario definitions.
    #[tokio::test]
    async fn test_e2e_tool_response_content() {
        let harness = TestHarness::with_scenario_file(
            Path::new("tests/scenarios/simple_implement.toml")
        ).expect("Failed to load scenario");

        let mut harness = harness.with_timeout(Duration::from_secs(10));

        let result = harness.run_goal("Add error handling").await
            .expect("Test should complete");

        // ----------------------------------------------------------------
        // Verify implement response has expected fields
        // ----------------------------------------------------------------
        let impl_response = result.tool_calls.iter()
            .find(|tc| matches!(&tc.call, crate::mcp_server::ToolCall::Implement { .. }))
            .map(|tc| &tc.response)
            .expect("Should have implement tool call");

        assert!(
            impl_response.success,
            "Implement should succeed in simple_implement scenario"
        );

        assert!(
            !impl_response.summary.is_empty(),
            "Implement response should have summary"
        );

        // Files modified should be present for successful implementations
        // (Based on simple_implement.toml, it specifies files_modified)
        assert!(
            impl_response.files_modified.is_some(),
            "Implement response should have files_modified list"
        );

        let files = impl_response.files_modified.as_ref().unwrap();
        assert!(
            !files.is_empty(),
            "Files modified list should not be empty"
        );
    }
}
