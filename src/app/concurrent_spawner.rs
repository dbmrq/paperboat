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
    let recovery_hint = if error_str.contains("not found") && error_str.contains("task") {
        "The task_id you provided does not exist. This often happens when using a fabricated ID \
         instead of actual task IDs from the plan. Call list_tasks() to see available task IDs \
         (e.g., 'task001', 'task002'), then retry spawn_agents with the correct IDs."
    } else if error_str.contains("mcp server startup") {
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
            .map(|id| format!("\n\nTask ID for retry: {id}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn agent_result(role: &str, task: &str, success: bool, message: &str) -> AgentResult {
        AgentResult {
            role: role.to_string(),
            task: task.to_string(),
            success,
            message: Some(message.to_string()),
            suggested_task_ids: vec![],
        }
    }

    #[tokio::test]
    async fn test_wait_for_all_collects_mixed_success_and_failure_results() {
        let (success_tx, success_rx) = oneshot::channel();
        let (failure_tx, failure_rx) = oneshot::channel();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = success_tx.send(agent_result(
                "implementer",
                "task-success",
                true,
                "implemented successfully",
            ));
        });

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = failure_tx.send(agent_result(
                "verifier",
                "task-failure",
                false,
                "verification failed",
            ));
        });

        let spawn_error = agent_result(
            "implementer",
            "task-spawn-error",
            false,
            "spawn error before execution",
        );
        let results = wait_for_all(
            vec![
                (
                    "implementer".to_string(),
                    "task-success".to_string(),
                    success_rx,
                ),
                (
                    "verifier".to_string(),
                    "task-failure".to_string(),
                    failure_rx,
                ),
            ],
            vec![spawn_error.clone()],
        )
        .await;

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].task, spawn_error.task);
        assert!(!results[0].success);

        assert_eq!(results.iter().filter(|result| result.success).count(), 1);
        assert_eq!(results.iter().filter(|result| !result.success).count(), 2);
        assert!(
            results
                .iter()
                .any(|result| result.task == "task-success" && result.success),
            "wait_for_all should retain the successful completion"
        );
        assert!(
            results
                .iter()
                .any(|result| result.task == "task-failure" && !result.success),
            "wait_for_all should retain the failed completion"
        );
    }

    #[tokio::test]
    async fn test_wait_for_any_returns_first_completion_even_when_it_fails() {
        let (slow_success_tx, slow_success_rx) = oneshot::channel();
        let (fast_failure_tx, fast_failure_rx) = oneshot::channel();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let _ = slow_success_tx.send(agent_result(
                "implementer",
                "task-success",
                true,
                "implemented successfully",
            ));
        });

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            let _ = fast_failure_tx.send(agent_result(
                "verifier",
                "task-failure",
                false,
                "verification failed first",
            ));
        });

        let spawn_error = agent_result(
            "implementer",
            "task-spawn-error",
            false,
            "spawn error before execution",
        );
        let results = wait_for_any(
            vec![
                (
                    "implementer".to_string(),
                    "task-success".to_string(),
                    slow_success_rx,
                ),
                (
                    "verifier".to_string(),
                    "task-failure".to_string(),
                    fast_failure_rx,
                ),
            ],
            vec![spawn_error.clone()],
        )
        .await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].task, spawn_error.task);
        assert_eq!(results[1].task, "task-failure");
        assert!(
            !results[1].success,
            "wait_for_any should return the first completion even if it is a failure"
        );
        assert_eq!(
            results[1].message.as_deref(),
            Some("verification failed first")
        );
    }
}
