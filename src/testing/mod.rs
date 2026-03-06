//! Mock data system for testing Villalobos.
//!
//! This module provides core mock types, scenario loading, builder helpers,
//! and a test harness for unit testing, integration testing, and end-to-end
//! testing without requiring live AI agents or external services.

// Core submodules
mod assertions;
mod builders;
mod harness;
mod interceptor;
mod mock_acp;
mod scenario;
mod types;

// Test modules
#[cfg(test)]
mod e2e_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod unit_tests;

// Re-export all public types and functions

// From types module
pub use types::{
    AgentType, MockAcpError, MockAcpResponse, MockAgentSession, MockMcpToolCall, MockSessionUpdate,
    MockToolCallResponse, MockToolResponseData, MockToolResult, MockToolType,
};

// From scenario module
pub use scenario::{MockScenario, ScenarioMetadata};

// From builders module
pub use builders::{MockSessionBuilder, MockToolResponseBuilder};

// From mock_acp module
pub use mock_acp::MockAcpClient;

// From assertions module
pub use assertions::{
    assert_decompose_called, assert_failure, assert_implement_called, assert_implementer_spawned,
    assert_orchestrator_spawned, assert_planner_spawned, assert_success, CapturedToolCall,
    TestRunResult,
};

// From interceptor module
pub use interceptor::MockToolInterceptor;

// From harness module
pub use harness::TestHarness;
