//! Tool call handlers for the orchestrator.
//!
//! This module contains the implementation of MCP tool call handlers for the
//! orchestrator agent. These handlers are separated from the main orchestrator
//! loop (`orchestrator.rs`) to improve maintainability and reduce file size.
//!
//! # Handlers in this module
//!
//! - [`handle_create_task`](App::handle_create_task) - Creates a new task
//! - [`handle_list_tasks`](App::handle_list_tasks) - Lists tasks with optional status filter
//! - [`handle_report_human_action`](App::handle_report_human_action) - Records actions for humans
//! - [`handle_skip_tasks`](App::handle_skip_tasks) - Marks tasks as skipped
//!
//! # Helper methods
//!
//! - [`get_task_state`](App::get_task_state) - Creates task state snapshot for responses
//! - [`check_completion_blockers`](App::check_completion_blockers) - Checks for pending tasks
//! - [`resolve_task_description`](App::resolve_task_description) - Resolves task from ID or text
//! - [`validate_agent_specs`](App::validate_agent_specs) - Validates agent specs before spawn

use super::types::truncate_for_log;
use super::App;
use crate::logging::AgentWriter;
use crate::mcp_server::{TaskStateInfo, ToolResponse};
use crate::tasks::TaskStatus;

impl App {
    /// Handle the `create_task` tool call.
    ///
    /// Creates a new task in the task manager with the given name, description,
    /// and dependencies. Returns a response indicating the new task ID.
    pub(crate) async fn handle_create_task(
        &self,
        name: &str,
        description: &str,
        dependencies: &[String],
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        let depth = self.current_scope.depth();
        let _ = writer.write_mcp_tool_call("create_task", name).await;

        let task_id = {
            let mut tm = self.task_manager.write().await;
            tm.create(name, description, dependencies.to_vec())
        };

        tracing::info!(
            "[L{}] 📋 Orchestrator created task [{}]: {}",
            depth,
            task_id,
            name
        );

        let task_state = self.get_task_state().await;

        let response = ToolResponse::success(
            request_id.to_string(),
            format!(
                "Task '{name}' created with ID: {task_id}. \
                 Remember to use spawn_agents with task_id=\"{task_id}\" to execute it."
            ),
        )
        .with_task_state(task_state);

        let _ = writer
            .write_mcp_tool_result("create_task", true, &format!("Created {task_id}"))
            .await;

        response
    }

    /// Handle the `list_tasks` tool call.
    ///
    /// Returns a formatted list of tasks, optionally filtered by status.
    /// Supported filters: `all`, `pending`, `in_progress`, `completed`, `failed`, `skipped`.
    pub(crate) async fn handle_list_tasks(
        &self,
        status_filter: Option<&String>,
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        let depth = self.current_scope.depth();
        let filter = status_filter.map_or("all", String::as_str);
        let _ = writer
            .write_mcp_tool_call("list_tasks", &format!("filter={filter}"))
            .await;
        tracing::info!("[L{}] 📋 list_tasks(filter={filter})", depth);

        let task_list = {
            let tm = self.task_manager.read().await;
            let tasks = tm.all_tasks();

            // Filter by status if specified
            let filtered: Vec<_> = tasks
                .into_iter()
                .filter(|t| match filter {
                    "pending" => matches!(t.status, TaskStatus::NotStarted),
                    "in_progress" => matches!(t.status, TaskStatus::InProgress { .. }),
                    "completed" => matches!(t.status, TaskStatus::Complete { .. }),
                    "failed" => matches!(t.status, TaskStatus::Failed { .. }),
                    "skipped" => matches!(t.status, TaskStatus::Skipped { .. }),
                    _ => true,
                })
                .collect();

            // Format task list
            let mut lines = Vec::new();
            lines.push(format!(
                "## Tasks ({} total, filter={})\n",
                filtered.len(),
                filter
            ));
            for task in filtered {
                let status_str = match &task.status {
                    TaskStatus::NotStarted => "pending",
                    TaskStatus::InProgress { .. } => "in_progress",
                    TaskStatus::Complete { .. } => "completed",
                    TaskStatus::Failed { .. } => "failed",
                    TaskStatus::Skipped { .. } => "skipped",
                };
                lines.push(format!(
                    "- **[{}]** {} ({}): {}",
                    task.id, task.name, status_str, task.description
                ));
            }
            lines.join("\n")
        };

        let response = ToolResponse::success(request_id.to_string(), task_list.clone());
        let _ = writer
            .write_mcp_tool_result(
                "list_tasks",
                true,
                &format!("{} tasks returned", task_list.lines().count() - 1),
            )
            .await;

        response
    }

    /// Handle the `report_human_action` tool call.
    ///
    /// Records an action that requires manual user intervention.
    /// These are displayed prominently at the end of the run.
    pub(crate) async fn handle_report_human_action(
        &self,
        description: &str,
        task_id: Option<&String>,
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        let depth = self.current_scope.depth();
        let preview = truncate_for_log(description, 50);
        let _ = writer
            .write_mcp_tool_call("report_human_action", &preview)
            .await;
        tracing::info!("[L{}] 📋 report_human_action: {}", depth, preview);

        // Add to task manager
        {
            let mut tm = self.task_manager.write().await;
            tm.add_human_action(description.to_string(), task_id.cloned());
        }

        let response = ToolResponse::success(
            request_id.to_string(),
            "Human action recorded. It will be displayed prominently at the end of the run."
                .to_string(),
        );
        let _ = writer
            .write_mcp_tool_result("report_human_action", true, "Action recorded")
            .await;

        response
    }

    /// Handle the `skip_tasks` tool call.
    ///
    /// Marks tasks as skipped if they are in `NotStarted` status.
    /// Tasks that are already complete, failed, or in progress cannot be skipped.
    pub(crate) async fn handle_skip_tasks(
        &self,
        task_ids: &[String],
        reason: Option<&String>,
        request_id: &str,
        writer: &mut AgentWriter,
    ) -> ToolResponse {
        let depth = self.current_scope.depth();
        let task_count = task_ids.len();
        let reason_str = reason.map_or("No reason provided", String::as_str);
        let _ = writer
            .write_mcp_tool_call("skip_tasks", &format!("{task_count} task(s): {reason_str}"))
            .await;
        tracing::info!(
            "[L{}] ⏭️ skip_tasks({} tasks, reason={:?})",
            depth,
            task_count,
            reason
        );

        // Track results for each task
        let mut skipped_tasks: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        // Validate and update task statuses
        {
            let mut tm = self.task_manager.write().await;
            for task_id in task_ids {
                if let Some(task) = tm.get_by_id_or_name(task_id) {
                    let actual_id = task.id.clone();
                    let task_name = task.name.clone();

                    match &task.status {
                        TaskStatus::NotStarted => {
                            let status = TaskStatus::Skipped {
                                reason: reason
                                    .cloned()
                                    .unwrap_or_else(|| "No reason provided".to_string()),
                            };
                            tm.update_status(&actual_id, &status);
                            tracing::info!(
                                "⏭️ Task '{}' ({}) marked as skipped",
                                actual_id,
                                task_name
                            );
                            skipped_tasks.push(format!("{actual_id}:{task_name}"));
                        }
                        TaskStatus::InProgress { .. } => {
                            let msg = format!(
                                "Task '{task_id}' is currently in progress and cannot be skipped"
                            );
                            tracing::warn!("⚠️ {}", msg);
                            errors.push(msg);
                        }
                        TaskStatus::Complete { .. } => {
                            let msg = format!(
                                "Task '{task_id}' is already complete and cannot be skipped"
                            );
                            tracing::warn!("⚠️ {}", msg);
                            errors.push(msg);
                        }
                        TaskStatus::Failed { .. } => {
                            // Failed tasks don't need to be skipped - they already have a
                            // definitive status. This is not an error, just informational.
                            tracing::info!(
                                "ℹ️ Task '{}' has already failed (no skip needed)",
                                task_id
                            );
                            // Don't add to errors - failed tasks are already accounted for
                            // in task reconciliation, so the orchestrator can proceed.
                        }
                        TaskStatus::Skipped { .. } => {
                            tracing::info!("⏭️ Task '{}' is already skipped", task_id);
                            skipped_tasks
                                .push(format!("{actual_id}:{task_name} (already skipped)"));
                        }
                    }
                } else {
                    let msg = format!("Task '{task_id}' not found");
                    tracing::warn!("⚠️ {}", msg);
                    errors.push(msg);
                }
            }
        }

        let task_state = self.get_task_state().await;
        let skipped_count = skipped_tasks.len();

        let (response, result_msg) = if errors.is_empty() {
            let summary = format!(
                "Skipped {} task(s): [{}]",
                skipped_count,
                skipped_tasks.join(", ")
            );
            (
                ToolResponse::success(request_id.to_string(), summary.clone())
                    .with_task_state(task_state),
                format!("✓ {summary}"),
            )
        } else if skipped_count > 0 {
            let summary = format!(
                "Skipped {} task(s): [{}]. Errors: {}",
                skipped_count,
                skipped_tasks.join(", "),
                errors.join("; ")
            );
            (
                ToolResponse::success(request_id.to_string(), summary.clone())
                    .with_task_state(task_state),
                format!("⚠️ {summary}"),
            )
        } else {
            let error_msg = errors.join("; ");
            (
                ToolResponse::failure(request_id.to_string(), error_msg.clone())
                    .with_task_state(task_state),
                format!("✗ Failed to skip tasks: {error_msg}"),
            )
        };

        let _ = writer
            .write_mcp_tool_result("skip_tasks", response.success, &result_msg)
            .await;

        response
    }

    /// Fetch current task state for context-aware responses.
    ///
    /// Creates a [`TaskStateInfo`] snapshot using the `TaskManager` helper methods.
    /// This is efficient because it only acquires a read lock and collects
    /// the minimal information needed for response building.
    pub(crate) async fn get_task_state(&self) -> TaskStateInfo {
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

    /// Check for pending tasks that would block completion.
    ///
    /// Returns `Some(error_message)` if there are pending tasks, `None` if completion is allowed.
    pub(crate) async fn check_completion_blockers(&self) -> Option<String> {
        let pending_tasks: Vec<(String, String)> = {
            let tm = self.task_manager.read().await;
            tm.all_tasks()
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::NotStarted))
                .map(|t| (t.id.clone(), t.name.clone()))
                .collect()
        };

        if pending_tasks.is_empty() {
            return None;
        }

        tracing::info!(
            "⚠️ Reconciliation check: {} pending task(s) remain",
            pending_tasks.len()
        );

        let task_list: Vec<String> = pending_tasks
            .iter()
            .map(|(id, name)| format!("- {id}: {name}"))
            .collect();

        Some(format!(
            "Cannot complete: {} pending task(s) remain:\n{}\n\n\
             Please either:\n\
             - Use spawn_agents to execute remaining tasks, or\n\
             - Use skip_tasks to explicitly skip tasks that are not needed",
            pending_tasks.len(),
            task_list.join("\n")
        ))
    }

    /// Resolve a task description from `task_id` or task field.
    ///
    /// Returns the task description if found, or a fallback message.
    pub(crate) async fn resolve_task_description(
        &self,
        task_id: Option<&String>,
        task: Option<&String>,
    ) -> String {
        if let Some(tid) = task_id {
            let tm = self.task_manager.read().await;
            tm.get_by_id_or_name(tid).map_or_else(
                || {
                    tracing::warn!(
                        "Task '{}' not found in TaskManager. Available: {:?}",
                        tid,
                        tm.list_task_ids()
                    );
                    format!("(task {tid} not found)")
                },
                |t| t.description.clone(),
            )
        } else {
            task.cloned().unwrap_or_else(|| "(no task)".to_string())
        }
    }

    /// Validate agent specifications before spawning.
    ///
    /// Returns `Some(error_message)` if validation fails, `None` if all agents are valid.
    pub(crate) fn validate_agent_specs(agents: &[crate::mcp_server::AgentSpec]) -> Option<String> {
        for agent in agents {
            let role = agent.role.as_deref().unwrap_or("implementer");
            if role.to_lowercase() == "custom" && agent.prompt.is_none() {
                return Some("Custom agent requires 'prompt' field".to_string());
            }
            if agent.task.is_none() && agent.task_id.is_none() {
                return Some("Agent requires either 'task' or 'task_id' field".to_string());
            }
        }
        None
    }
}
