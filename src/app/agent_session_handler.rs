//! Agent session handler for concurrent agent execution.
//!
//! This module provides helper functions for handling agent sessions,
//! including notification routing and completion processing.

use super::agent_handler::{run_agent_handler, update_task_completion, AgentCompletionData};
use super::router::SessionRouter;
use super::socket::AgentSocketHandle;
use super::spawn_config::AgentResult;
use super::types::{format_duration_human, truncate_for_log};
use crate::backend::transport::AgentTransport;
use crate::logging::AgentWriter;
use crate::tasks::TaskManager;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};

/// Spawns a notification routing task for an agent transport.
///
/// Routes messages from the agent's transport to the shared session router.
/// Returns true if the router was started successfully.
///
/// This works with any `AgentTransport` implementation (ACP or CLI),
/// using the `take_notification_rx()` method which all transports implement.
pub fn spawn_notification_router(
    agent_transport: &mut dyn AgentTransport,
    session_router: Arc<RwLock<SessionRouter>>,
    agent_name: &str,
) -> bool {
    let agent_notification_rx = agent_transport.take_notification_rx();
    if let Some(notification_rx) = agent_notification_rx {
        let agent_id_for_log = agent_name.to_string();
        tokio::spawn(async move {
            let mut rx: mpsc::Receiver<serde_json::Value> = notification_rx;
            while let Some(msg) = rx.recv().await {
                let routed = {
                    let router = session_router.read().await;
                    router.route(msg.clone())
                };
                if !routed {
                    tracing::trace!(
                        "[{}] Agent message not routed (session may have ended)",
                        agent_id_for_log
                    );
                }
            }
            tracing::debug!("[{}] Agent notification channel closed", agent_id_for_log);
        });
        true
    } else {
        tracing::warn!(
            "[{}] Could not take notification channel from agent transport",
            agent_name
        );
        false
    }
}

/// Parameters for the agent handler task.
pub struct AgentHandlerParams {
    /// The agent transport (boxed for ownership transfer to async task).
    /// This is kept alive for the duration of the agent's execution.
    pub agent_transport: Box<dyn AgentTransport>,
    pub socket_handle: AgentSocketHandle,
    pub session_rx: mpsc::Receiver<serde_json::Value>,
    pub timeout_duration: Duration,
    pub role: String,
    pub task: String,
    pub impl_name: String,
    pub task_id: Option<String>,
    pub session_id: String,
    pub session_router: Arc<RwLock<SessionRouter>>,
    pub task_manager: Arc<RwLock<TaskManager>>,
    pub result_tx: oneshot::Sender<AgentResult>,
}

/// Spawns the agent handler task that manages completion.
pub fn spawn_agent_handler_task(params: AgentHandlerParams, mut impl_writer: AgentWriter) {
    let AgentHandlerParams {
        agent_transport,
        socket_handle,
        session_rx,
        timeout_duration,
        role,
        task,
        impl_name,
        task_id,
        session_id,
        session_router,
        task_manager,
        result_tx,
    } = params;

    tokio::spawn(async move {
        // Keep agent_transport alive for the duration of the agent's execution.
        let _agent_transport = agent_transport;

        let start_time = std::time::Instant::now();

        // Wait for the agent to complete, handling tool calls
        let completion = run_agent_handler(
            socket_handle,
            session_rx,
            timeout_duration,
            &role,
            &task,
            &impl_name,
            &mut impl_writer,
        )
        .await;

        let elapsed = start_time.elapsed();
        let elapsed_str = format_duration_human(elapsed);

        // Record agent completion metrics
        crate::metrics::record_agent_completed(&role, completion.success, elapsed);

        // Process completion (store notes, create tasks, update status)
        // Returns the IDs of any tasks created from add_tasks suggestions
        let suggested_task_ids =
            process_agent_completion(&completion, &role, task_id.as_ref(), &task_manager).await;

        // Finalize the writer
        if let Err(e) = impl_writer.finalize(completion.success).await {
            tracing::warn!("Failed to finalize implementer log: {}", e);
        }

        // Unregister from router
        {
            let mut router = session_router.write().await;
            router.unregister(&session_id);
        }

        log_completion_status(&completion, &role, &elapsed_str, &task);

        let result = AgentResult {
            role,
            task,
            success: completion.success,
            message: completion.message,
            suggested_task_ids,
        };

        // Send result (ignore if receiver dropped)
        let _ = result_tx.send(result);
    });
}

/// Processes agent completion: stores notes, creates suggested tasks, updates status.
///
/// Returns the IDs of any tasks that were created from the agent's `add_tasks` suggestion.
async fn process_agent_completion(
    completion: &AgentCompletionData,
    role: &str,
    task_id: Option<&String>,
    task_manager: &Arc<RwLock<TaskManager>>,
) -> Vec<String> {
    // Store notes if provided
    if let Some(ref notes) = completion.notes {
        let mut tm = task_manager.write().await;
        tm.add_note(role, task_id.cloned(), notes.clone());
    }

    // Create suggested tasks if provided, tracking the IDs
    let mut suggested_task_ids = Vec::new();
    if let Some(ref suggested_tasks) = completion.add_tasks {
        let mut tm = task_manager.write().await;
        for suggested in suggested_tasks {
            let deps = suggested.depends_on.clone().unwrap_or_default();
            let new_id = tm.create(&suggested.name, &suggested.description, deps);
            tracing::info!("📋 Created suggested task [{}]: {}", new_id, suggested.name);
            suggested_task_ids.push(new_id);
        }
    }

    // Update task status if linked to a tracked task
    if let Some(tid) = task_id {
        update_task_completion(
            task_manager,
            tid,
            completion.success,
            completion.message.as_deref(),
        )
        .await;
    }

    suggested_task_ids
}

/// Logs the completion status of an agent.
fn log_completion_status(
    completion: &AgentCompletionData,
    role: &str,
    elapsed_str: &str,
    task: &str,
) {
    if completion.success {
        tracing::info!(
            "✅ [concurrent] Agent {} completed ({}) - {}",
            role,
            elapsed_str,
            truncate_for_log(task, 60)
        );
    } else {
        tracing::error!(
            "❌ [concurrent] Agent {} FAILED after {} - {}",
            role,
            elapsed_str,
            truncate_for_log(task, 80)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::socket::setup_agent_socket;
    use crate::logging::{AgentType, AgentWriter};
    use crate::tasks::TaskStatus;
    use crate::testing::MockTransport;
    use tempfile::tempdir;
    use tokio::sync::{broadcast, oneshot};

    #[tokio::test]
    async fn spawn_agent_handler_task_marks_task_failed_and_unregisters_session() {
        let dir = tempdir().expect("create temp dir");
        let log_path = dir.path().join("implementer-001.log");
        let (event_tx, _) = broadcast::channel(16);
        let writer = AgentWriter::new(
            log_path,
            AgentType::Implementer { index: 1 },
            event_tx.clone(),
            0,
        )
        .await
        .expect("create writer");

        let session_router = Arc::new(RwLock::new(SessionRouter::new()));
        let task_manager = Arc::new(RwLock::new(TaskManager::new(event_tx)));

        let task_id = {
            let mut tm = task_manager.write().await;
            let task_id = tm.create(
                "Retry implementer startup",
                "Propagate concurrent agent startup failures",
                vec![],
            );
            tm.update_status(
                &task_id,
                &TaskStatus::InProgress {
                    agent_session: Some("implementer-001".to_string()),
                },
            );
            task_id
        };

        let session_id = "impl-session-001".to_string();
        let session_rx = {
            let mut router = session_router.write().await;
            router.register(&session_id)
        };

        let socket_handle = setup_agent_socket("session-handler-failure")
            .await
            .expect("create socket");
        let (result_tx, result_rx) = oneshot::channel();

        spawn_agent_handler_task(
            AgentHandlerParams {
                agent_transport: Box::new(MockTransport::empty()),
                socket_handle,
                session_rx,
                timeout_duration: Duration::from_secs(1),
                role: "implementer".to_string(),
                task: "Retry implementer startup".to_string(),
                impl_name: "implementer-001".to_string(),
                task_id: Some(task_id.clone()),
                session_id: session_id.clone(),
                session_router: Arc::clone(&session_router),
                task_manager: Arc::clone(&task_manager),
                result_tx,
            },
            writer,
        );

        let routed = {
            let router = session_router.read().await;
            router.route(serde_json::json!({
                "method": "session/update",
                "params": {
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "session_finished"
                    }
                }
            }))
        };
        assert!(routed, "session update should reach the registered handler");

        let result = result_rx.await.expect("receive handler result");
        assert!(!result.success);
        assert!(
            result
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("without calling complete()")
        );

        let task = task_manager
            .read()
            .await
            .get(&task_id)
            .cloned()
            .expect("tracked task should remain available");
        match task.status {
            TaskStatus::Failed { error } => {
                assert!(error.contains("without calling complete()"));
            }
            other => panic!("expected failed task status, got {other:?}"),
        }

        let routed_after_cleanup = {
            let router = session_router.read().await;
            router.route(serde_json::json!({
                "method": "session/update",
                "params": {
                    "sessionId": "impl-session-001",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {
                            "text": "late message"
                        }
                    }
                }
            }))
        };
        assert!(
            !routed_after_cleanup,
            "session should be unregistered once the handler exits"
        );
    }
}
