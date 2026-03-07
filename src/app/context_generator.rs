//! Context generation for implementer agents.
//!
//! Generates task context including neighboring tasks and dependency summaries.

use super::types::truncate_for_log;
use super::App;
use crate::mcp_server::AgentSpec;
use crate::tasks::Task;

impl App {
    /// Generate context about neighboring tasks and dependency summaries for an implementer.
    pub(crate) async fn generate_task_context(
        &self,
        agents: &[AgentSpec],
        index: usize,
        spec: &AgentSpec,
    ) -> String {
        let mut sections = Vec::new();

        // Acquire TaskManager lock once for all lookups
        let tm = self.task_manager.read().await;

        // 1. Dependency summaries from TaskManager (if task_id is provided)
        if let Some(ref task_id) = spec.task_id {
            if let Some(dep_summary) = tm.format_dependency_summaries(task_id) {
                sections.push(dep_summary);
            }
        }

        // 2. Neighboring tasks context
        if agents.len() > 1 {
            let mut neighbor_lines = Vec::new();

            // Helper to format task label from spec, looking up name from TaskManager if needed
            let format_task_label = |spec: &AgentSpec, task: Option<&Task>| -> String {
                match (&spec.task_id, &spec.task, task) {
                    // If we have a task_id and found the task, show "[task001] Name"
                    (Some(id), _, Some(t)) => format!("[{id}] {}", t.name),
                    // If we have a task_id but no task found, and no task text, just show the id
                    (Some(id), None, None) => format!("[{id}]"),
                    // If we have a task_id but no task found, show id with fallback text
                    (Some(id), Some(task_text), None) => format!("[{id}] {task_text}"),
                    // If we only have task text (no task_id), show it directly
                    (None, Some(task_text), _) => task_text.clone(),
                    // No information available
                    (None, None, _) => "(unknown task)".to_string(),
                }
            };

            // Previous tasks (up to 2)
            let prev_start = index.saturating_sub(2);
            if prev_start < index {
                neighbor_lines.push("## Previous Tasks".to_string());
                for agent in agents.iter().take(index).skip(prev_start) {
                    // Use get_by_id_or_name for flexible lookup
                    let task = agent
                        .task_id
                        .as_ref()
                        .and_then(|id| tm.get_by_id_or_name(id));
                    neighbor_lines.push(format!(
                        "- {}",
                        truncate_for_log(&format_task_label(agent, task), 100)
                    ));
                }
            }

            // Next tasks (up to 2)
            let next_end = (index + 3).min(agents.len());
            if index + 1 < next_end {
                neighbor_lines.push("## Next Tasks".to_string());
                for agent in agents.iter().take(next_end).skip(index + 1) {
                    // Use get_by_id_or_name for flexible lookup
                    let task = agent
                        .task_id
                        .as_ref()
                        .and_then(|id| tm.get_by_id_or_name(id));
                    neighbor_lines.push(format!(
                        "- {}",
                        truncate_for_log(&format_task_label(agent, task), 100)
                    ));
                }
            }

            if !neighbor_lines.is_empty() {
                sections.push(neighbor_lines.join("\n"));
            }
        }

        sections.join("\n\n")
    }
}
