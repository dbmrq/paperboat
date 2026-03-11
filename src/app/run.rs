//! Main run logic for the orchestrator.

use super::App;
use crate::types::TaskResult;
use anyhow::{Context, Result};

impl App {
    /// Run the orchestrator with a goal
    ///
    /// This first spawns a Planner to create a high-level plan, then spawns
    /// an Orchestrator to execute the plan by delegating to implementers.
    ///
    /// Socket setup and MCP configuration happens in the App constructor.
    #[tracing::instrument(skip(self))]
    pub async fn run(&mut self, goal: &str) -> Result<TaskResult> {
        tracing::info!("Starting with goal: {}", goal);

        // Store the original goal for context passing to implementers
        self.original_goal = goal.to_string();

        // Start the worker router for message routing
        self.start_worker_router();

        // Run the planning phase
        let plan_to_execute = match self.run_planning_phase(goal).await {
            Ok(plan) => plan,
            Err(e) => {
                self.cleanup();
                return Err(e);
            }
        };

        // Run the execution phase
        let result = self.run_execution_phase(&plan_to_execute).await;

        // Always clean up, even if orchestrator failed
        self.cleanup();

        result
    }

    /// Clean up resources (socket and MCP configuration).
    fn cleanup(&mut self) {
        // Clean up socket - take ownership to avoid borrow issues
        if let Some(address) = self.socket_address.take() {
            self.cleanup_socket(&address);
        }
        // Clean up MCP configuration
        if let Err(e) = self.backend.cleanup_mcp() {
            tracing::warn!("Failed to cleanup MCP: {}", e);
        }
    }

    /// Run the planning phase: spawn a planner and collect its plan.
    #[tracing::instrument(skip(self), fields(phase = "planning"))]
    async fn run_planning_phase(&mut self, goal: &str) -> Result<String> {
        // Create planner writer at root scope
        let mut planner_writer = self
            .current_scope
            .planner_writer()
            .await
            .context("Failed to create planner writer")?;

        // First, spawn a Planner to create a plan from the goal
        tracing::info!("📝 Planning phase: spawning planner agent");
        let planner_session = match self.spawn_planner(goal).await {
            Ok((session, model, prompt)) => {
                planner_writer.set_session_id(session.clone());
                planner_writer.set_model(model);
                if let Err(e) = planner_writer.write_header_with_prompt(goal, &prompt).await {
                    tracing::warn!("Failed to write planner header: {}", e);
                }
                // Emit AgentStarted event for TUI
                planner_writer.emit_agent_started(goal);
                session
            }
            Err(e) => {
                // Write error to planner log so it's not empty
                tracing::error!("❌ Failed to spawn planner: {:#}", e);
                if let Err(write_err) = planner_writer.write_spawn_error(&e).await {
                    tracing::warn!("Failed to write spawn error to planner log: {}", write_err);
                }
                if let Err(finalize_err) = planner_writer.finalize(false).await {
                    tracing::warn!(
                        "Failed to finalize planner log after spawn error: {}",
                        finalize_err
                    );
                }
                return Err(e);
            }
        };

        // Wait for planner to complete and collect its output (with timeout)
        let planner_output = match self
            .wait_for_session_output(&planner_session, &mut planner_writer)
            .await
        {
            Ok(output) => output,
            Err(e) => {
                // Finalize planner log with failure status before returning
                tracing::error!("❌ Planner session failed: {}", e);
                if let Err(finalize_err) = planner_writer.finalize(false).await {
                    tracing::warn!(
                        "Failed to finalize planner log after session error: {}",
                        finalize_err
                    );
                }
                return Err(anyhow::anyhow!("{e}"));
            }
        };

        // Finalize planner log
        if let Err(e) = planner_writer.finalize(true).await {
            tracing::warn!("Failed to finalize planner log: {}", e);
        }

        // Renumber tasks by execution order so IDs match execution sequence
        {
            let mut tm = self.task_manager.write().await;
            tm.renumber_by_execution_order();
        }

        // Use structured tasks from TaskManager
        let formatted_tasks = {
            let tm = self.task_manager.read().await;
            tm.format_for_orchestrator()
        };

        if let Some((count, plan)) = formatted_tasks {
            tracing::info!("📋 Using {} structured tasks from create_task calls", count);
            Ok(plan)
        } else if !planner_output.is_empty() {
            // Fallback: raw planner output (not recommended)
            tracing::warn!("⚠️  Planner did not create tasks, falling back to raw output");
            Ok(planner_output.text.clone())
        } else {
            Err(anyhow::anyhow!(
                "Planner produced no plan (no create_task calls, no text output)"
            ))
        }
    }

    /// Run the execution phase: spawn an orchestrator to execute the plan.
    #[tracing::instrument(skip(self, plan_to_execute), fields(phase = "execution"))]
    async fn run_execution_phase(&mut self, plan_to_execute: &str) -> Result<TaskResult> {
        // Create orchestrator writer
        let mut orchestrator_writer = self
            .current_scope
            .orchestrator_writer()
            .await
            .context("Failed to create orchestrator writer")?;

        // Now spawn the orchestrator to execute the plan
        // Pass the clean plan (not the planner's full stream-of-consciousness)
        tracing::info!("🎭 Execution phase: spawning orchestrator agent");
        let result = self
            .run_orchestrator_with_writer(plan_to_execute, &mut orchestrator_writer)
            .await;

        // Finalize orchestrator log
        let success = result.as_ref().map(|r| r.success).unwrap_or(false);
        if let Err(e) = orchestrator_writer.finalize(success).await {
            tracing::warn!("Failed to finalize orchestrator log: {}", e);
        }

        // Save the final task list to the logs folder
        self.save_task_list().await;

        result
    }

    /// Save the task list to a tasks.json file in the logs folder.
    async fn save_task_list(&self) {
        let task_list_path = self.current_scope.dir().join("tasks.json");
        let task_manager = self.task_manager.read().await;

        match task_manager.export_as_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&task_list_path, &json) {
                    tracing::warn!("Failed to write task list to {:?}: {}", task_list_path, e);
                } else {
                    tracing::info!(
                        "📋 Saved task list ({} tasks) to {:?}",
                        task_manager.all_tasks().len(),
                        task_list_path
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Failed to serialize task list: {}", e);
            }
        }
    }
}
