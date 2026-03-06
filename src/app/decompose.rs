//! Decompose task handling - creates child orchestrators for subtasks.

use super::types::{format_duration_human, truncate_for_log};
use super::App;
use anyhow::{Context, Result};

impl App {
    /// Inner decompose logic that can fail.
    pub(crate) async fn handle_decompose_inner(&mut self, task: &str) -> Result<String> {
        let start_time = std::time::Instant::now();
        tracing::info!("🔄 Starting decomposition: {}", truncate_for_log(task, 100));

        // Create child scope (subtask folder) for this decomposition
        let child_scope = self
            .current_scope
            .child_scope(task)
            .await
            .context("Failed to create child scope")?;

        let subtask_dir = child_scope.dir().display().to_string();
        tracing::debug!("📁 Created subtask directory: {}", subtask_dir);
        let previous_scope = std::mem::replace(&mut self.current_scope, child_scope);

        // Create planner writer for subtask
        let mut planner_writer = self
            .current_scope
            .planner_writer()
            .await
            .context("Failed to create subtask planner writer")?;

        // 1. Spawn planner to create plan
        let (planner_session, planner_prompt) = self.spawn_planner(task).await?;
        planner_writer.set_session_id(planner_session.clone());
        if let Err(e) = planner_writer
            .write_header_with_prompt(task, &planner_prompt)
            .await
        {
            tracing::warn!("Failed to write subtask planner header: {}", e);
        }

        // 2. Wait for planner to complete and collect output (with timeout)
        let planner_output = self
            .wait_for_session_output(&planner_session, &mut planner_writer)
            .await
            .map_err(|e| {
                // Restore scope before returning error
                self.current_scope = previous_scope.clone();
                anyhow::anyhow!("{e}")
            })?;

        // Finalize planner log
        if let Err(e) = planner_writer.finalize(true).await {
            tracing::warn!("Failed to finalize subtask planner log: {}", e);
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
        let result = self
            .run_orchestrator_with_writer(&plan_to_execute, &mut orchestrator_writer)
            .await?;

        // Finalize orchestrator log
        if let Err(e) = orchestrator_writer.finalize(result.success).await {
            tracing::warn!("Failed to finalize subtask orchestrator log: {}", e);
        }

        // Restore previous scope
        self.current_scope = previous_scope;

        let elapsed = start_time.elapsed();
        let elapsed_str = format_duration_human(elapsed);

        if result.success {
            tracing::info!(
                "✅ Decomposition complete ({}) - {}",
                elapsed_str,
                truncate_for_log(task, 60)
            );
        } else {
            tracing::error!(
                "❌ Decomposition FAILED ({}) - {}",
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
