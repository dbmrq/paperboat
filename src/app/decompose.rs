//! Decompose task handling - creates child orchestrators for subtasks.
//!
//! When a task is too complex for a single agent, the orchestrator can decompose it
//! into subtasks using a nested planner-orchestrator pair. This module handles:
//!
//! 1. Snapshot/restore of parent task state to prevent ID conflicts
//! 2. Creation of child scope (subdirectory) for logging isolation
//! 3. Running nested planner to create subtask plan
//! 4. Running nested orchestrator to execute subtasks
//! 5. Finalizing child tasks before restoring parent state
//!
//! # Child Task Finalization
//!
//! When the nested orchestrator completes, any child tasks still in `NotStarted`
//! status are automatically marked as `Skipped` with a reason indicating they
//! were not addressed by the nested orchestrator. This ensures:
//!
//! - All child tasks have a definitive final status
//! - `TaskStateChanged` events are emitted before tasks are removed
//! - The TUI can display accurate completion status

use super::types::{format_duration_human, truncate_for_log};
use super::App;
use crate::mcp_server::{TaskStateInfo, ToolResponse};
use crate::tasks::{TaskManager, TaskManagerSnapshot, TaskStatus};
use anyhow::{Context, Result};

/// Finalize all child tasks by marking `NotStarted` tasks as `Skipped`.
///
/// Called before restoring the parent's task snapshot to ensure all child tasks
/// have a terminal status. This emits `TaskStateChanged` events that update the
/// TUI before the child tasks are removed from the `TaskManager`.
///
/// # Arguments
/// * `tm` - The task manager containing child tasks
/// * `reason` - The reason to record for skipped tasks
///
/// # Returns
/// The number of tasks that were finalized (marked as Skipped).
fn finalize_child_tasks(tm: &mut TaskManager, reason: &str) -> usize {
    // Collect IDs of tasks that need to be skipped (to avoid borrow issues)
    let not_started_ids: Vec<String> = tm
        .all_tasks()
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::NotStarted))
        .map(|task| task.id.clone())
        .collect();

    let count = not_started_ids.len();
    for task_id in not_started_ids {
        tm.update_status(
            &task_id,
            &TaskStatus::Skipped {
                reason: reason.to_string(),
            },
        );
    }
    count
}

impl App {
    /// Handle decompose tool call, returning a `ToolResponse`.
    pub(crate) async fn handle_decompose_with_response(
        &mut self,
        task: &str,
        request_id: &str,
    ) -> ToolResponse {
        let result = self.handle_decompose_inner(task).await;

        // Fetch task state for context-aware response
        let task_state = self.get_decompose_task_state().await;

        match result {
            Ok(summary) => {
                ToolResponse::success(request_id.to_string(), summary).with_task_state(task_state)
            }
            Err(e) => ToolResponse::failure(request_id.to_string(), e.to_string())
                .with_task_state(task_state),
        }
    }

    /// Fetch current task state for context-aware decompose responses.
    async fn get_decompose_task_state(&self) -> TaskStateInfo {
        let tm = self.task_manager.read().await;
        let pending = tm.get_pending_tasks();
        let parallel = tm.get_parallel_tasks();
        let blocked = tm.get_blocked_tasks();

        TaskStateInfo {
            pending_count: pending.len(),
            parallel_tasks: parallel,
            blocked_tasks: blocked,
        }
    }

    /// Inner decompose logic that can fail.
    pub(crate) async fn handle_decompose_inner(&mut self, task: &str) -> Result<String> {
        eprintln!(
            "[handle_decompose_inner] START: {}",
            truncate_for_log(task, 50)
        );
        let start_time = std::time::Instant::now();
        let parent_depth = self.current_scope.depth();
        tracing::info!(
            "[L{}] 🔄 Starting decomposition: {}",
            parent_depth,
            truncate_for_log(task, 100)
        );

        // Create child scope (subtask folder) for this decomposition
        let child_scope = self
            .current_scope
            .child_scope(task)
            .await
            .context("Failed to create child scope")?;

        let subtask_dir = child_scope.dir().display().to_string();
        let child_depth = child_scope.depth();
        tracing::debug!(
            "[L{}] 📁 Created subtask directory: {}",
            child_depth,
            subtask_dir
        );
        let previous_scope = std::mem::replace(&mut self.current_scope, child_scope);

        // Save parent's task state and clear for nested planner
        // This prevents task ID conflicts between parent and nested levels
        let task_snapshot: TaskManagerSnapshot = {
            let mut tm = self.task_manager.write().await;
            let snapshot = tm.snapshot();
            tm.clear_tasks_for_nested(child_depth);
            snapshot
        };

        // Create planner writer for subtask
        let mut planner_writer = self
            .current_scope
            .planner_writer()
            .await
            .context("Failed to create subtask planner writer")?;

        // 1. Spawn planner to create plan
        let (planner_session, planner_model, planner_prompt) = match self.spawn_planner(task).await
        {
            Ok(result) => result,
            Err(e) => {
                // Write error to planner log so it's not empty
                tracing::error!(
                    "[L{}] ❌ Failed to spawn subtask planner: {:#}",
                    child_depth,
                    e
                );
                if let Err(write_err) = planner_writer.write_spawn_error(&e).await {
                    tracing::warn!("Failed to write spawn error to planner log: {}", write_err);
                }
                if let Err(finalize_err) = planner_writer.finalize(false).await {
                    tracing::warn!(
                        "Failed to finalize planner log after spawn error: {}",
                        finalize_err
                    );
                }
                // Restore scope before returning error
                self.current_scope = previous_scope;
                return Err(e);
            }
        };
        planner_writer.set_session_id(planner_session.clone());
        planner_writer.set_model(planner_model);
        if let Err(e) = planner_writer
            .write_header_with_prompt(task, &planner_prompt)
            .await
        {
            tracing::warn!("Failed to write subtask planner header: {}", e);
        }
        // Emit AgentStarted event for TUI
        planner_writer.emit_agent_started(task);

        // 2. Wait for planner to complete and collect output (with timeout)
        eprintln!("[handle_decompose_inner] Waiting for sub-planner session: {planner_session}");
        let planner_output = match self
            .wait_for_session_output(&planner_session, &mut planner_writer)
            .await
        {
            Ok(output) => {
                eprintln!("[handle_decompose_inner] Sub-planner completed successfully");
                output
            }
            Err(e) => {
                eprintln!("[handle_decompose_inner] Sub-planner FAILED: {e}");
                // Finalize planner log with failure status before returning
                tracing::error!(
                    "[L{}] ❌ Subtask planner session failed: {}",
                    child_depth,
                    e
                );
                if let Err(finalize_err) = planner_writer.finalize(false).await {
                    tracing::warn!(
                        "Failed to finalize planner log after session error: {}",
                        finalize_err
                    );
                }
                // Restore scope before returning error
                self.current_scope = previous_scope;
                return Err(anyhow::anyhow!("{e}"));
            }
        };

        // Finalize planner log
        if let Err(e) = planner_writer.finalize(true).await {
            tracing::warn!("Failed to finalize subtask planner log: {}", e);
        }

        // Renumber tasks by execution order so IDs match execution sequence
        {
            let mut tm = self.task_manager.write().await;
            tm.renumber_by_execution_order();
        }

        // Use structured tasks from TaskManager, or fall back to full planner output
        let formatted_tasks = {
            let tm = self.task_manager.read().await;
            tm.format_for_orchestrator()
        };

        let plan_to_execute = if let Some((count, plan)) = formatted_tasks {
            tracing::info!("📋 Using {} structured tasks from create_task calls", count);
            plan
        } else if !planner_output.is_empty() {
            tracing::warn!("⚠️  Planner did not create tasks, falling back to full output");
            planner_output.text.clone()
        } else {
            self.current_scope = previous_scope;
            return Err(anyhow::anyhow!(
                "Planner produced no plan for decomposition"
            ));
        };

        // 3. Create orchestrator writer for subtask
        let mut orchestrator_writer = self
            .current_scope
            .orchestrator_writer()
            .await
            .context("Failed to create subtask orchestrator writer")?;

        // 4. Spawn child orchestrator with clean plan (not full planner output)
        eprintln!("[handle_decompose_inner] Starting sub-orchestrator with plan");
        let result = match self
            .run_orchestrator_with_writer(&plan_to_execute, &mut orchestrator_writer)
            .await
        {
            Ok(res) => {
                eprintln!("[handle_decompose_inner] Sub-orchestrator completed successfully");
                res
            }
            Err(e) => {
                eprintln!("[handle_decompose_inner] Sub-orchestrator FAILED: {e}");
                // Finalize orchestrator log with failure status before returning
                tracing::error!("[L{}] ❌ Subtask orchestrator failed: {}", child_depth, e);
                if let Err(finalize_err) = orchestrator_writer.finalize(false).await {
                    tracing::warn!(
                        "Failed to finalize orchestrator log after error: {}",
                        finalize_err
                    );
                }
                // Finalize child tasks and restore parent's task state
                {
                    let mut tm = self.task_manager.write().await;
                    let finalized_count =
                        finalize_child_tasks(&mut tm, "Parent decomposition failed");
                    if finalized_count > 0 {
                        tracing::info!(
                            "[L{}] 📋 Finalized {} child tasks before restore (error path)",
                            child_depth,
                            finalized_count
                        );
                    }
                    tm.restore_from_snapshot(task_snapshot);
                }
                // Restore scope before returning error
                self.current_scope = previous_scope;
                return Err(e);
            }
        };

        // Finalize orchestrator log
        if let Err(e) = orchestrator_writer.finalize(result.success).await {
            tracing::warn!("Failed to finalize subtask orchestrator log: {}", e);
        }

        // Finalize child tasks and restore parent's task state
        {
            let mut tm = self.task_manager.write().await;
            let finalized_count = finalize_child_tasks(&mut tm, "Parent decomposition completed");
            if finalized_count > 0 {
                tracing::info!(
                    "[L{}] 📋 Finalized {} child tasks before restore",
                    child_depth,
                    finalized_count
                );
            }
            tm.restore_from_snapshot(task_snapshot);
        }

        // Restore previous scope
        self.current_scope = previous_scope;

        let elapsed = start_time.elapsed();
        let elapsed_str = format_duration_human(elapsed);
        let current_depth = self.current_scope.depth();

        if result.success {
            tracing::info!(
                "[L{}] ✅ Decomposition complete ({}) - {}",
                current_depth,
                elapsed_str,
                truncate_for_log(task, 60)
            );
        } else {
            tracing::error!(
                "[L{}] ❌ Decomposition FAILED ({}) - {}",
                current_depth,
                elapsed_str,
                truncate_for_log(task, 60)
            );
        }

        eprintln!("[handle_decompose_inner] DONE - returning");
        Ok(format!(
            "Decomposed and executed. Result: {}",
            if result.success { "success" } else { "failure" }
        ))
    }
}
