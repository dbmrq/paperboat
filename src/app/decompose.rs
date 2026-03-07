//! Decompose task handling - creates child orchestrators for subtasks.

use super::types::{format_duration_human, truncate_for_log};
use super::App;
use crate::tasks::TaskManagerSnapshot;
use anyhow::{Context, Result};

impl App {
    /// Inner decompose logic that can fail.
    pub(crate) async fn handle_decompose_inner(&mut self, task: &str) -> Result<String> {
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
        tracing::debug!("[L{}] 📁 Created subtask directory: {}", child_depth, subtask_dir);
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
        let (planner_session, planner_prompt) = match self.spawn_planner(task).await {
            Ok(result) => result,
            Err(e) => {
                // Write error to planner log so it's not empty
                tracing::error!("[L{}] ❌ Failed to spawn subtask planner: {:#}", child_depth, e);
                if let Err(write_err) = planner_writer.write_spawn_error(&e).await {
                    tracing::warn!("Failed to write spawn error to planner log: {}", write_err);
                }
                if let Err(finalize_err) = planner_writer.finalize(false).await {
                    tracing::warn!("Failed to finalize planner log after spawn error: {}", finalize_err);
                }
                // Restore scope before returning error
                self.current_scope = previous_scope;
                return Err(e);
            }
        };
        planner_writer.set_session_id(planner_session.clone());
        if let Err(e) = planner_writer
            .write_header_with_prompt(task, &planner_prompt)
            .await
        {
            tracing::warn!("Failed to write subtask planner header: {}", e);
        }
        // Emit AgentStarted event for TUI
        planner_writer.emit_agent_started(task);

        // 2. Wait for planner to complete and collect output (with timeout)
        let planner_output = match self
            .wait_for_session_output(&planner_session, &mut planner_writer)
            .await
        {
            Ok(output) => output,
            Err(e) => {
                // Finalize planner log with failure status before returning
                tracing::error!("[L{}] ❌ Subtask planner session failed: {}", child_depth, e);
                if let Err(finalize_err) = planner_writer.finalize(false).await {
                    tracing::warn!("Failed to finalize planner log after session error: {}", finalize_err);
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
        let result = match self
            .run_orchestrator_with_writer(&plan_to_execute, &mut orchestrator_writer)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                // Finalize orchestrator log with failure status before returning
                tracing::error!("[L{}] ❌ Subtask orchestrator failed: {}", child_depth, e);
                if let Err(finalize_err) = orchestrator_writer.finalize(false).await {
                    tracing::warn!("Failed to finalize orchestrator log after error: {}", finalize_err);
                }
                // Restore parent's task state
                {
                    let mut tm = self.task_manager.write().await;
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

        // Restore parent's task state
        {
            let mut tm = self.task_manager.write().await;
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

        Ok(format!(
            "Decomposed and executed. Result: {}",
            if result.success { "success" } else { "failure" }
        ))
    }
}
