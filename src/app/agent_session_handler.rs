//! Agent session handler for concurrent agent execution.
//!
//! This module provides helper functions for handling agent sessions,
//! including notification routing and completion processing.

use super::agent_handler::{run_agent_handler, update_task_completion, AgentCompletionData};
use super::router::SessionRouter;
use super::socket::AgentSocketHandle;
use super::spawn_config::AgentResult;
use super::types::{format_duration_human, truncate_for_log};
use crate::acp::{AcpClient, AcpClientTrait};
use crate::logging::AgentWriter;
use crate::tasks::TaskManager;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};

/// Spawns a notification routing task for an agent's ACP client.
///
/// Routes messages from the agent's ACP client to the shared session router.
/// Returns true if the router was started successfully.
pub fn spawn_notification_router(
    agent_acp: &mut AcpClient,
    session_router: Arc<RwLock<SessionRouter>>,
    agent_name: &str,
) -> bool {
    let agent_notification_rx = agent_acp.take_notification_rx();
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
            "[{}] Could not take notification channel from agent ACP client",
            agent_name
        );
        false
    }
}

/// Parameters for the agent handler task.
pub struct AgentHandlerParams {
    pub agent_acp: AcpClient,
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
        agent_acp,
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
        // Keep agent_acp alive for the duration of the agent's execution.
        let _agent_acp = agent_acp;

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
