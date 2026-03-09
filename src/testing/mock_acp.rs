//! Mock ACP client for testing.
//!
//! Provides a mock implementation of `AcpClientTrait` that returns scripted
//! responses from a `MockScenario`, enabling deterministic testing without
//! requiring a live agent process.

use crate::acp::{AcpClientTrait, SessionNewResponse};
use crate::app::ToolMessage;
use crate::mcp_server::{ToolCall, ToolRequest};
use crate::testing::{
    AgentType, MockAgentSession, MockMcpToolCall, MockScenario, MockSessionUpdate,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// A session update paired with its optional associated tool call.
/// Tool calls are injected when the corresponding update is returned.
#[derive(Debug)]
struct QueuedUpdate {
    /// The ACP notification to return from `recv()`.
    notification: Value,
    /// Optional tool call to inject when this update is returned.
    tool_call: Option<MockMcpToolCall>,
}

/// Mock ACP client that returns scripted responses.
///
/// This client simulates ACP behavior using data from a `MockScenario`,
/// allowing tests to run without spawning actual agent processes.
pub struct MockAcpClient {
    /// Session-specific update queues, keyed by session ID.
    /// This ensures updates for different sessions don't get interleaved,
    /// which is critical for nested orchestrator scenarios (e.g., decompose).
    session_queues: std::collections::HashMap<String, VecDeque<QueuedUpdate>>,

    /// Stack of active session IDs. The top of the stack is the current session.
    /// This tracks which session's updates should be returned by `recv()`.
    active_sessions: Vec<String>,

    /// Counter for generating unique session IDs.
    session_counter: usize,

    /// Captured prompts for assertion: (`session_id`, prompt).
    captured_prompts: Vec<(String, String)>,

    /// Reference sessions from the scenario (by agent type).
    planner_sessions: Vec<MockAgentSession>,
    orchestrator_sessions: Vec<MockAgentSession>,
    implementer_sessions: Vec<MockAgentSession>,

    /// Index tracking for each session type.
    planner_index: usize,
    orchestrator_index: usize,
    implementer_index: usize,

    /// Track created sessions for test assertions.
    sessions_created: Vec<String>,

    /// Flag indicating shutdown was called.
    shutdown_called: bool,

    /// Channel to send tool calls to the App (for test mode).
    tool_tx: Option<mpsc::Sender<ToolMessage>>,

    /// Mock tool interceptor for getting responses (shared with test harness).
    tool_interceptor: Option<Arc<Mutex<super::MockToolInterceptor>>>,
}

impl MockAcpClient {
    /// Create a new mock ACP client from a scenario.
    pub fn from_scenario(scenario: &MockScenario) -> Self {
        Self {
            session_queues: std::collections::HashMap::new(),
            active_sessions: Vec::new(),
            session_counter: 0,
            captured_prompts: Vec::new(),
            planner_sessions: scenario.planner_sessions.clone(),
            orchestrator_sessions: scenario.orchestrator_sessions.clone(),
            implementer_sessions: scenario.implementer_sessions.clone(),
            planner_index: 0,
            orchestrator_index: 0,
            implementer_index: 0,
            sessions_created: Vec::new(),
            shutdown_called: false,
            tool_tx: None,
            tool_interceptor: None,
        }
    }

    /// Create an empty mock client (no sessions).
    pub fn empty() -> Self {
        Self::from_scenario(&MockScenario::default())
    }

    /// Set the tool channel for injecting tool calls.
    pub fn with_tool_channel(
        mut self,
        tool_tx: mpsc::Sender<ToolMessage>,
        tool_interceptor: Arc<Mutex<super::MockToolInterceptor>>,
    ) -> Self {
        self.tool_tx = Some(tool_tx);
        self.tool_interceptor = Some(tool_interceptor);
        self
    }

    /// Get all captured prompts as (`session_id`, prompt) pairs.
    pub fn captured_prompts(&self) -> &[(String, String)] {
        &self.captured_prompts
    }

    /// Get the number of sessions created.
    pub const fn sessions_created_count(&self) -> usize {
        self.sessions_created.len()
    }

    /// Check if all sessions have been used.
    pub fn is_exhausted(&self) -> bool {
        self.planner_index >= self.planner_sessions.len()
            && self.orchestrator_index >= self.orchestrator_sessions.len()
            && self.implementer_index >= self.implementer_sessions.len()
            && self.session_queues.values().all(VecDeque::is_empty)
    }

    /// Get the next session for the given agent type.
    fn next_session(&mut self, agent_type: AgentType) -> Option<MockAgentSession> {
        match agent_type {
            AgentType::Planner => {
                if self.planner_index < self.planner_sessions.len() {
                    let session = self.planner_sessions[self.planner_index].clone();
                    self.planner_index += 1;
                    Some(session)
                } else {
                    None
                }
            }
            AgentType::Orchestrator => {
                if self.orchestrator_index < self.orchestrator_sessions.len() {
                    let session = self.orchestrator_sessions[self.orchestrator_index].clone();
                    self.orchestrator_index += 1;
                    Some(session)
                } else {
                    None
                }
            }
            AgentType::Implementer => {
                if self.implementer_index < self.implementer_sessions.len() {
                    let session = self.implementer_sessions[self.implementer_index].clone();
                    self.implementer_index += 1;
                    Some(session)
                } else {
                    None
                }
            }
        }
    }

    /// Determine agent type from model name and MCP server configuration.
    ///
    /// The agent type is primarily determined from the MCP server's environment
    /// variables (`PAPERBOAT_AGENT_TYPE`), with fallback to model name heuristics.
    fn agent_type_from_config(&self, model: &str, mcp_servers: &[Value]) -> AgentType {
        // First, try to detect from MCP server environment variables
        for server in mcp_servers {
            if let Some(env_array) = server.get("env").and_then(|e| e.as_array()) {
                for env_var in env_array {
                    let name = env_var.get("name").and_then(|n| n.as_str());
                    let value = env_var.get("value").and_then(|v| v.as_str());

                    if name == Some("PAPERBOAT_AGENT_TYPE") {
                        match value {
                            Some("planner") => return AgentType::Planner,
                            Some("implementer") => return AgentType::Implementer,
                            Some("orchestrator") => return AgentType::Orchestrator,
                            _ => {}
                        }
                    }
                }
            }

            // Also check the server name
            if let Some(name) = server.get("name").and_then(|n| n.as_str()) {
                if name.contains("planner") {
                    return AgentType::Planner;
                } else if name.contains("implementer") {
                    return AgentType::Implementer;
                } else if name.contains("orchestrator") {
                    return AgentType::Orchestrator;
                }
            }
        }

        // Fallback to model name heuristics
        let model_lower = model.to_lowercase();
        if model_lower.contains("planner") {
            AgentType::Planner
        } else if model_lower.contains("orchestrat") {
            AgentType::Orchestrator
        } else {
            // Default to implementer for unknown models
            AgentType::Implementer
        }
    }

    /// Queue updates from a session for the given `session_id`.
    /// Each update is paired with its optional associated tool call.
    /// Updates are stored in a session-specific queue to prevent interleaving.
    fn queue_updates(&mut self, session_id: &str, updates: &[MockSessionUpdate]) {
        let queue = self
            .session_queues
            .entry(session_id.to_string())
            .or_default();
        for update in updates {
            let notification = mock_session_update_to_notification(session_id, update);
            queue.push_back(QueuedUpdate {
                notification,
                tool_call: update.inject_mcp_tool_call.clone(),
            });
        }
        // Push this session to the active sessions stack
        self.active_sessions.push(session_id.to_string());
    }

    /// Inject a tool call through the tool channel.
    /// This is fire-and-forget - we don't wait for the App's response.
    /// The tool interceptor has already provided the mock response.
    async fn inject_tool_call(&mut self, mock_tool_call: &MockMcpToolCall) -> Result<()> {
        let tool_tx = self
            .tool_tx
            .as_ref()
            .ok_or_else(|| anyhow!("No tool channel configured for MockAcpClient"))?;
        let interceptor = self
            .tool_interceptor
            .as_ref()
            .ok_or_else(|| anyhow!("No tool interceptor configured for MockAcpClient"))?;

        // Convert MockMcpToolCall to ToolCall
        let tool_call = match mock_tool_call {
            MockMcpToolCall::CreateTask {
                name,
                description,
                dependencies,
            } => ToolCall::CreateTask {
                name: name.clone(),
                description: description.clone(),
                dependencies: dependencies.clone(),
            },
            MockMcpToolCall::Complete {
                success,
                message,
                notes,
                add_tasks,
            } => ToolCall::Complete {
                success: *success,
                message: message.clone(),
                notes: notes.clone(),
                add_tasks: add_tasks.as_ref().map(|tasks| {
                    tasks
                        .iter()
                        .map(|t| crate::mcp_server::SuggestedTask {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            depends_on: t.depends_on.clone(),
                        })
                        .collect()
                }),
            },
            MockMcpToolCall::SpawnAgents { task } => {
                // Create a single-agent spawn for backward compatibility
                ToolCall::SpawnAgents {
                    agents: vec![crate::mcp_server::AgentSpec {
                        role: Some("implementer".to_string()),
                        task: Some(task.clone()),
                        task_id: None,
                        prompt: None,
                        tools: None,
                        model_complexity: None,
                    }],
                    wait: crate::mcp_server::WaitMode::All,
                }
            }
            MockMcpToolCall::Decompose { task } => ToolCall::Decompose {
                task_id: None,
                task: Some(task.clone()),
            },
            MockMcpToolCall::SkipTasks { task_ids, reason } => ToolCall::SkipTasks {
                task_ids: task_ids.clone(),
                reason: reason.clone(),
            },
        };

        let request_id = uuid::Uuid::new_v4().to_string();

        // Record the tool call in the interceptor (this captures it for assertions)
        // For create_task and complete, the interceptor handles them specially.
        // For spawn_agents/decompose, the interceptor returns a mock response.
        {
            let mut guard = interceptor.lock().await;
            // Just record the call - we don't need the response here since
            // the App will handle it internally for create_task/complete
            let _ = guard.get_response(&tool_call, &request_id);
        }

        // Create a oneshot channel for the response
        // We spawn a task to handle the response so we don't block
        let (response_tx, _response_rx) = tokio::sync::oneshot::channel();

        // Send the tool request to the App
        let request = ToolRequest {
            request_id: request_id.clone(),
            tool_call,
        };

        // Use try_send to avoid blocking - if the channel is full, that's a test bug
        tool_tx
            .try_send(ToolMessage::Request {
                request,
                response_tx,
            })
            .map_err(|e| anyhow!("Failed to send tool call (channel full or closed): {e}"))?;

        Ok(())
    }
}

/// Convert a `MockSessionUpdate` to an ACP JSON-RPC notification format.
fn mock_session_update_to_notification(session_id: &str, update: &MockSessionUpdate) -> Value {
    let mut notification = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "sessionUpdate": update.session_update
            }
        }
    });

    // Add content if present (for message chunks)
    if let Some(content) = &update.content {
        notification["params"]["update"]["content"] = json!({
            "type": "text",
            "text": content
        });
    }

    // Add tool title if present (for tool_call updates)
    if let Some(tool_title) = &update.tool_title {
        notification["params"]["update"]["title"] = json!(tool_title);
    }

    // Add tool result if present (for tool_result updates)
    if let Some(tool_result) = &update.tool_result {
        notification["params"]["update"]["title"] = json!(tool_result.title);
        notification["params"]["update"]["isError"] = json!(tool_result.is_error);
        notification["params"]["update"]["content"] = json!(tool_result.content);
    }

    notification
}

#[async_trait]
impl AcpClientTrait for MockAcpClient {
    async fn initialize(&mut self) -> Result<()> {
        // Mock initialization always succeeds
        Ok(())
    }

    async fn session_new(
        &mut self,
        model: &str,
        mcp_servers: Vec<Value>,
        _cwd: &str,
    ) -> Result<SessionNewResponse> {
        // Determine agent type from model name and MCP server config
        let agent_type = self.agent_type_from_config(model, &mcp_servers);

        // Try to get the next session for this agent type
        if let Some(session) = self.next_session(agent_type) {
            let session_id = session.session_id.clone();
            self.sessions_created.push(session_id.clone());

            // Queue all updates from this session
            self.queue_updates(&session_id, &session.updates);

            Ok(SessionNewResponse { session_id })
        } else {
            // Fall back to generating a unique session ID
            self.session_counter += 1;
            let session_counter = self.session_counter;
            let session_id = format!("mock-session-{session_counter}");
            self.sessions_created.push(session_id.clone());
            Ok(SessionNewResponse { session_id })
        }
    }

    async fn session_prompt(&mut self, session_id: &str, prompt: &str) -> Result<()> {
        // Capture the prompt for test assertions
        self.captured_prompts
            .push((session_id.to_string(), prompt.to_string()));

        // Optionally validate against expected patterns
        // (Find the session that matches this session_id and check expected_prompt_contains)
        let all_sessions: Vec<&MockAgentSession> = self
            .planner_sessions
            .iter()
            .chain(self.orchestrator_sessions.iter())
            .chain(self.implementer_sessions.iter())
            .collect();

        for session in all_sessions {
            if session.session_id == session_id {
                if let Some(patterns) = &session.expected_prompt_contains {
                    for pattern in patterns {
                        if !prompt.contains(pattern) {
                            return Err(anyhow!(
                                "Prompt validation failed: expected prompt to contain '{pattern}', got: {prompt}"
                            ));
                        }
                    }
                }
                break;
            }
        }

        Ok(())
    }

    async fn recv(&mut self) -> Result<Value> {
        // Get the current active session (most recently started)
        // We use the LAST session in the stack - this is the most recently started session
        // which should be the one currently being processed
        let Some(current_session) = self.active_sessions.last().cloned() else {
            eprintln!("[MockAcpClient::recv] No active session, stack empty");
            return Err(anyhow!("No active session"));
        };

        eprintln!(
            "[MockAcpClient::recv] session={}, stack={:?}",
            current_session, self.active_sessions
        );

        // Get the next update from the current session's queue
        let queue = self.session_queues.get_mut(&current_session);
        let queued = queue.and_then(VecDeque::pop_front);

        if let Some(queued) = queued {
            // If this update has an associated tool call, inject it
            // The tool call is injected AFTER returning the update notification,
            // simulating the real behavior where the agent sends a message chunk
            // like "[calling create_task]" followed by the actual tool call.
            if let Some(ref tool_call) = queued.tool_call {
                if let Err(e) = self.inject_tool_call(tool_call).await {
                    tracing::warn!("Failed to inject mock tool call: {}", e);
                    // Continue anyway - the test may still work
                }
            }

            // Check if this session's queue is now empty AND it was the last update
            // (agent_turn_finished or similar)
            let queue_empty = self
                .session_queues
                .get(&current_session)
                .map_or(true, VecDeque::is_empty);

            // Extract session_update from notification to check if this is the end
            // Path is: params.update.sessionUpdate
            let session_update = queued
                .notification
                .get("params")
                .and_then(|p| p.get("update"))
                .and_then(|u| u.get("sessionUpdate"))
                .and_then(|s| s.as_str())
                .unwrap_or("");

            if queue_empty
                && (session_update == "agent_turn_finished" || session_update == "session_finished")
            {
                // This session is complete - remove it from active sessions
                eprintln!(
                    "[MockAcpClient::recv] Session {current_session} complete, removing from stack"
                );
                self.active_sessions.retain(|s| s != &current_session);
            }

            Ok(queued.notification)
        } else {
            // No more updates for this session - remove it from active
            self.active_sessions.retain(|s| s != &current_session);
            // Return an error to signal exhaustion of this session
            Err(anyhow!(
                "No more mock updates available for session {current_session}"
            ))
        }
    }

    fn take_notification_rx(&mut self) -> Option<mpsc::Receiver<Value>> {
        // Mock client doesn't have a real notification receiver to take.
        // For testing, we return None since mock clients use the update_queue directly.
        None
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.shutdown_called = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockSessionBuilder;

    #[tokio::test]
    async fn test_mock_acp_client_basic() {
        let scenario = MockScenario {
            planner_sessions: vec![MockSessionBuilder::new("planner-001")
                .with_message_chunk("Planning...", 100)
                .with_turn_finished(50)
                .build()],
            ..Default::default()
        };

        let mut client = MockAcpClient::from_scenario(&scenario);

        // Initialize should succeed
        client.initialize().await.unwrap();

        // Create a planner session
        let response = client
            .session_new("planner-model", vec![], "/tmp")
            .await
            .unwrap();
        assert_eq!(response.session_id, "planner-001");
        assert_eq!(client.sessions_created_count(), 1);

        // Should have queued 2 updates
        let update1 = client.recv().await.unwrap();
        assert_eq!(update1["method"], "session/update");
        assert_eq!(
            update1["params"]["update"]["sessionUpdate"],
            "agent_message_chunk"
        );

        let update2 = client.recv().await.unwrap();
        assert_eq!(
            update2["params"]["update"]["sessionUpdate"],
            "agent_turn_finished"
        );

        // No more updates
        assert!(client.recv().await.is_err());
        assert!(client.is_exhausted());
    }

    #[tokio::test]
    async fn test_mock_acp_client_prompt_capture() {
        let scenario = MockScenario {
            implementer_sessions: vec![MockSessionBuilder::new("impl-001").build()],
            ..Default::default()
        };

        let mut client = MockAcpClient::from_scenario(&scenario);
        client.initialize().await.unwrap();

        let response = client
            .session_new("implementer", vec![], "/tmp")
            .await
            .unwrap();
        client
            .session_prompt(&response.session_id, "Do something")
            .await
            .unwrap();

        let prompts = client.captured_prompts();
        assert_eq!(prompts.len(), 1);
        assert_eq!(
            prompts[0],
            ("impl-001".to_string(), "Do something".to_string())
        );
    }

    #[tokio::test]
    async fn test_mock_acp_client_shutdown() {
        let mut client = MockAcpClient::empty();
        assert!(!client.shutdown_called);
        client.shutdown().await.unwrap();
        assert!(client.shutdown_called);
    }
}
