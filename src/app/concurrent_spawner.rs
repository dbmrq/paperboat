//! Concurrent agent spawning and wait mode handling.
//!
//! This module provides the logic for spawning multiple agents concurrently
//! and handling different wait modes (All, Any, None).

use super::spawn_config::AgentResult;
use crate::mcp_server::AgentSpec;
use tokio::sync::oneshot;

/// Handles spawn errors during concurrent agent creation.
///
/// Creates an `AgentResult` with detailed error information and recovery suggestions
/// so the orchestrator can make informed decisions about how to proceed.
pub fn create_spawn_error(spec: &AgentSpec, error: &anyhow::Error) -> AgentResult {
    let role = spec
        .role
        .clone()
        .unwrap_or_else(|| "implementer".to_string());
    let task = spec
        .task
        .clone()
        .or_else(|| spec.task_id.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    let task_id = spec.task_id.clone();

    tracing::error!("Failed to spawn agent [{}]: {:#}", role, error);

    // Analyze the error and provide actionable guidance
    let error_str = format!("{error:#}").to_lowercase();
    let recovery_hint = if error_str.contains("mcp server startup") {
        "This may be a transient MCP server startup issue. The system attempted automatic retries. \
         Consider: (1) waiting a moment and retrying with spawn_agents, \
         (2) checking system resources, or (3) reducing concurrent agent count."
    } else if error_str.contains("timeout") {
        "The operation timed out. This could indicate system overload or network issues. \
         Consider retrying with fewer concurrent agents or after a brief wait."
    } else if error_str.contains("socket") {
        "Socket communication error. This may indicate the main process is overloaded. \
         Consider retrying the task."
    } else {
        "An unexpected error occurred. Review the error details and consider retrying \
         or creating a recovery task."
    };

    let message = format!(
        "SPAWN FAILED: {}\n\nError details: {:#}\n\nRecovery suggestion: {}{}",
        task,
        error,
        recovery_hint,
        task_id
            .as_ref()
            .map(|id| format!("\n\nTask ID for retry: {}", id))
            .unwrap_or_default()
    );

    AgentResult {
        role,
        task,
        success: false,
        message: Some(message),
        suggested_task_ids: task_id.into_iter().collect(),
    }
}

/// Extracts role and task from an agent spec for receiver tracking.
pub fn extract_role_and_task(spec: &AgentSpec) -> (String, String) {
    let role = spec
        .role
        .clone()
        .unwrap_or_else(|| "implementer".to_string());
    let task = spec
        .task
        .clone()
        .or_else(|| spec.task_id.clone())
        .unwrap_or_else(|| "(unknown)".to_string());
    (role, task)
}

/// Waits for all agents to complete and collects their results.
pub async fn wait_for_all(
    receivers: Vec<(String, String, oneshot::Receiver<AgentResult>)>,
    spawn_errors: Vec<AgentResult>,
) -> Vec<AgentResult> {
    let mut results = spawn_errors;

    for (role, task, receiver) in receivers {
        match receiver.await {
            Ok(result) => {
                results.push(result);
            }
            Err(_) => {
                results.push(AgentResult {
                    role,
                    task,
                    success: false,
                    message: Some("Agent channel closed".to_string()),
                    suggested_task_ids: vec![],
                });
            }
        }
    }

    let success_count = results.iter().filter(|r| r.success).count();
    tracing::info!(
        "✅ All {} agents completed: {}/{} successful",
        results.len(),
        success_count,
        results.len()
    );

    results
}

/// Waits for the first agent to complete and returns that result.
pub async fn wait_for_any(
    receivers: Vec<(String, String, oneshot::Receiver<AgentResult>)>,
    spawn_errors: Vec<AgentResult>,
) -> Vec<AgentResult> {
    if receivers.is_empty() {
        return spawn_errors;
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    for (role, task, receiver) in receivers {
        let tx = tx.clone();
        tokio::spawn(async move {
            match receiver.await {
                Ok(result) => {
                    let _ = tx.send(result).await;
                }
                Err(_) => {
                    let _ = tx
                        .send(AgentResult {
                            role,
                            task,
                            success: false,
                            message: Some("Agent channel closed".to_string()),
                            suggested_task_ids: vec![],
                        })
                        .await;
                }
            }
        });
    }
    drop(tx); // Drop our copy so channel closes when all senders done

    if let Some(first_result) = rx.recv().await {
        tracing::info!(
            "⚡ First agent completed: [{}] success={}",
            first_result.role,
            first_result.success
        );
        let mut results = spawn_errors;
        results.push(first_result);
        results
    } else {
        spawn_errors
    }
}

/// Handles the fire-and-forget wait mode.
pub fn handle_fire_and_forget(
    receiver_count: usize,
    spawn_errors: Vec<AgentResult>,
) -> Vec<AgentResult> {
    tracing::info!("🔥 Fire-and-forget mode: {} agents spawned", receiver_count);
    spawn_errors
}
