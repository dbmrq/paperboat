//! Context builder for the self-improvement agent.
//!
//! Analyzes completed runs and builds rich, structured context that helps
//! the self-improvement agent understand what happened and where to focus.

use crate::tasks::{TaskManager, TaskStatus};
use crate::types::TaskResult;
use std::path::Path;

/// Information about a log file for analysis.
#[derive(Debug)]
pub struct LogFileInfo {
    /// Relative path from run directory
    pub path: String,
    /// File size in bytes
    pub size: u64,
    /// Brief description of what this log contains
    pub description: &'static str,
    /// Whether this file exists
    pub exists: bool,
}

/// Statistics extracted from the run.
#[derive(Debug, Default)]
pub struct RunStats {
    /// Total number of tasks
    pub total_tasks: usize,
    /// Completed tasks
    pub completed_tasks: usize,
    /// Failed tasks
    pub failed_tasks: usize,
    /// Skipped tasks
    pub skipped_tasks: usize,
    /// Number of implementer log files found
    pub agents_spawned: usize,
    /// Number of error patterns found in logs
    pub error_count: usize,
    /// Number of warning patterns found in logs
    pub warning_count: usize,
}

/// Outcome classification for determining what the self-improver should focus on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Run completed successfully - focus on optimization
    Success,
    /// Run partially succeeded - focus on error patterns and recovery
    PartialSuccess,
    /// Run failed - should not trigger self-improvement (or error analysis only)
    Failed,
}

impl RunOutcome {
    /// Determine the outcome based on task result and stats.
    pub const fn from_result_and_stats(result: &TaskResult, stats: &RunStats) -> Self {
        if !result.success {
            return Self::Failed;
        }

        if stats.failed_tasks > 0 || stats.error_count > 0 {
            return Self::PartialSuccess;
        }

        Self::Success
    }

    /// Get focus areas description for this outcome.
    pub const fn focus_areas(&self) -> &'static str {
        match self {
            Self::Success => {
                r"**Focus: Optimization Opportunities**
1. Identify patterns that could be streamlined
2. Look for prompts that could be clearer or more concise
3. Find opportunities to reduce token usage
4. Suggest documentation improvements
5. Consider edge cases that could be handled proactively"
            }
            Self::PartialSuccess => {
                r"**Focus: Error Recovery and Robustness**
1. Analyze error patterns and their root causes
2. Identify which prompts led to confusion
3. Look for retry patterns that eventually succeeded
4. Find edge cases that weren't handled well
5. Suggest improvements to error messages and recovery guidance"
            }
            Self::Failed => {
                r"**Focus: Error Analysis Only**
1. Analyze why the run failed
2. Identify the root cause of failures
3. Do NOT make changes - only document findings
4. Report patterns that need human review
5. Suggest which areas of the codebase need attention"
            }
        }
    }
}

/// Build rich context for the self-improvement agent from a completed run.
///
/// This function analyzes the run directory, task results, and task manager state
/// to produce a structured markdown context that can be appended to the agent's
/// task description.
///
/// # Arguments
///
/// * `log_dir` - Path to the run's log directory
/// * `result` - The final task result from the run
/// * `task_manager` - Reference to the `TaskManager` with final state
///
/// # Returns
///
/// A markdown-formatted string suitable for inclusion in an agent prompt.
///
/// # Errors
///
/// This function is designed to be resilient. It will not fail if logs are
/// missing or unreadable - it will simply note what's unavailable.
pub async fn build_self_improvement_context(
    log_dir: &Path,
    result: &TaskResult,
    task_manager: &TaskManager,
) -> Result<String, std::io::Error> {
    let mut sections = Vec::new();

    // 1. Extract run statistics from task manager
    let stats = extract_run_stats(task_manager, log_dir).await;

    // 2. Determine run outcome and appropriate focus
    let outcome = RunOutcome::from_result_and_stats(result, &stats);

    // 3. Build run summary section
    sections.push(build_run_summary(log_dir, result, &stats, outcome));

    // 4. Build log file inventory
    sections.push(build_log_inventory(log_dir).await);

    // 5. Build quick stats section
    sections.push(build_quick_stats(&stats));

    // 6. Add focus areas based on outcome
    sections.push(outcome.focus_areas().to_string());

    // 7. Add task summary if available
    if let Some(task_section) = build_task_summary(task_manager) {
        sections.push(task_section);
    }

    // 8. Add error patterns if found
    if let Some(error_section) = extract_error_patterns(log_dir).await {
        sections.push(error_section);
    }

    Ok(sections.join("\n\n---\n\n"))
}

/// Extract statistics from the task manager and log directory.
async fn extract_run_stats(task_manager: &TaskManager, log_dir: &Path) -> RunStats {
    let mut stats = RunStats::default();

    // Count tasks by status
    for task in task_manager.all_tasks() {
        stats.total_tasks += 1;
        match &task.status {
            TaskStatus::Complete { success, .. } => {
                if *success {
                    stats.completed_tasks += 1;
                } else {
                    stats.failed_tasks += 1;
                }
            }
            TaskStatus::Failed { .. } => stats.failed_tasks += 1,
            TaskStatus::Skipped { .. } => stats.skipped_tasks += 1,
            _ => {}
        }
    }

    // Count implementer logs to estimate agents spawned
    stats.agents_spawned = count_implementer_logs(log_dir);

    // Count errors and warnings in logs
    let (errors, warnings) = count_log_patterns(log_dir).await;
    stats.error_count = errors;
    stats.warning_count = warnings;

    stats
}

/// Count implementer log files in the directory (recursive).
fn count_implementer_logs(dir: &Path) -> usize {
    let mut count = 0;

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("implementer-")
                        && path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
                    {
                        count += 1;
                    }
                }
            } else if path.is_dir() {
                // Recurse into subdirectories (subtask-001, etc.)
                count += count_implementer_logs(&path);
            }
        }
    }

    count
}

/// Count error and warning patterns in log files.
async fn count_log_patterns(dir: &Path) -> (usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;

    // Patterns that indicate errors or warnings in our logs
    let error_patterns = ["❌", "ERROR", "error:", "Failed:", "Tool failed:"];
    let warning_patterns = ["⚠", "WARN", "warning:"];

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("log"))
            {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    for pattern in &error_patterns {
                        errors += content.matches(pattern).count();
                    }
                    for pattern in &warning_patterns {
                        warnings += content.matches(pattern).count();
                    }
                }
            } else if path.is_dir() {
                let (sub_errors, sub_warnings) = Box::pin(count_log_patterns(&path)).await;
                errors += sub_errors;
                warnings += sub_warnings;
            }
        }
    }

    (errors, warnings)
}

/// Build the run summary section.
fn build_run_summary(
    log_dir: &Path,
    result: &TaskResult,
    stats: &RunStats,
    outcome: RunOutcome,
) -> String {
    let outcome_emoji = match outcome {
        RunOutcome::Success => "✅",
        RunOutcome::PartialSuccess => "⚠️",
        RunOutcome::Failed => "❌",
    };

    let outcome_label = match outcome {
        RunOutcome::Success => "SUCCESS",
        RunOutcome::PartialSuccess => "PARTIAL SUCCESS",
        RunOutcome::Failed => "FAILED",
    };

    format!(
        r"## Run Summary

{outcome_emoji} **Outcome**: {outcome_label}
**Result**: {}
**Message**: {}

**Run Directory**: `{}`

**Task Statistics**:
- Total tasks: {}
- Completed: {}
- Failed: {}
- Skipped: {}",
        if result.success { "Success" } else { "Failed" },
        result.message.as_deref().unwrap_or("(no message)"),
        log_dir.display(),
        stats.total_tasks,
        stats.completed_tasks,
        stats.failed_tasks,
        stats.skipped_tasks
    )
}

/// Build the log file inventory section.
async fn build_log_inventory(log_dir: &Path) -> String {
    let log_files = discover_log_files(log_dir);

    let mut lines = vec!["## Log File Inventory".to_string()];
    lines.push(String::new());

    if log_files.is_empty() {
        lines.push("No log files found.".to_string());
    } else {
        for info in &log_files {
            let status = if info.exists {
                format!("({} bytes)", info.size)
            } else {
                "(missing)".to_string()
            };
            lines.push(format!(
                "- `{}` - {} {}",
                info.path, info.description, status
            ));
        }
    }

    lines.join("\n")
}

/// Discover log files in the run directory.
fn discover_log_files(log_dir: &Path) -> Vec<LogFileInfo> {
    let mut files = Vec::new();

    // Standard log files to look for
    let standard_files = [
        ("planner.log", "Planning phase decisions"),
        ("orchestrator.log", "Task orchestration and agent spawning"),
        ("app.log", "Application-level events and errors"),
    ];

    for (name, description) in standard_files {
        let path = log_dir.join(name);
        let (exists, size) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0)
        };

        files.push(LogFileInfo {
            path: name.to_string(),
            size,
            description,
            exists,
        });
    }

    // Find implementer logs
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        let mut impl_files: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_str()?.to_string();
                if name.starts_with("implementer-")
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
                {
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    Some(LogFileInfo {
                        path: name,
                        size,
                        description: "Individual task execution",
                        exists: true,
                    })
                } else {
                    None
                }
            })
            .collect();

        impl_files.sort_by(|a, b| a.path.cmp(&b.path));
        files.extend(impl_files);
    }

    // Note subtask directories
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("subtask-") {
                        files.push(LogFileInfo {
                            path: format!("{name}/"),
                            size: 0,
                            description: "Nested subtask logs (directory)",
                            exists: true,
                        });
                    }
                }
            }
        }
    }

    files
}

/// Build the quick stats section.
fn build_quick_stats(stats: &RunStats) -> String {
    format!(
        r"## Quick Stats

| Metric | Value |
|--------|-------|
| Agents Spawned | {} |
| Errors Found | {} |
| Warnings Found | {} |",
        stats.agents_spawned, stats.error_count, stats.warning_count
    )
}

/// Build task summary section from task manager state.
fn build_task_summary(task_manager: &TaskManager) -> Option<String> {
    let tasks: Vec<_> = task_manager.all_tasks();
    if tasks.is_empty() {
        return None;
    }

    let mut lines = vec!["## Task Summary".to_string()];
    lines.push(String::new());

    // Sort tasks by ID
    let mut sorted: Vec<_> = tasks.into_iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));

    // Limit to first 20 tasks to keep context size reasonable
    let limit = 20;
    let truncated = sorted.len() > limit;

    for task in sorted.iter().take(limit) {
        let status_emoji = match &task.status {
            TaskStatus::Complete { success: true, .. } => "✅",
            TaskStatus::Complete { success: false, .. } => "⚠️",
            TaskStatus::Failed { .. } => "❌",
            TaskStatus::Skipped { .. } => "⏭️",
            TaskStatus::InProgress { .. } => "🔄",
            TaskStatus::NotStarted => "⏸️",
        };

        let summary = match &task.status {
            TaskStatus::Complete { summary, .. } => {
                // Truncate long summaries
                if summary.len() > 80 {
                    format!("{}...", &summary[..77])
                } else {
                    summary.clone()
                }
            }
            TaskStatus::Failed { error } => {
                let err_display = if error.len() > 60 {
                    &error[..57]
                } else {
                    error
                };
                format!("FAILED: {err_display}")
            }
            TaskStatus::Skipped { reason } => {
                let reason_display = if reason.len() > 60 {
                    &reason[..57]
                } else {
                    reason
                };
                format!("Skipped: {reason_display}")
            }
            _ => task.status.as_display_str().to_string(),
        };

        lines.push(format!(
            "- {} **[{}]** {}: {}",
            status_emoji, task.id, task.name, summary
        ));
    }

    if truncated {
        lines.push(format!("\n... and {} more tasks", sorted.len() - limit));
    }

    Some(lines.join("\n"))
}

/// Extract error patterns from log files for analysis.
///
/// Returns a markdown section summarizing notable errors if found.
async fn extract_error_patterns(log_dir: &Path) -> Option<String> {
    const MAX_ERRORS: usize = 15;
    let mut error_lines = Vec::new();

    // Patterns that indicate errors worth highlighting
    let error_markers = ["❌", "ERROR", "error:", "Tool failed:", "Failed:", "panic"];

    // Read each log file and extract error lines
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file()
                || path
                    .extension()
                    .is_none_or(|e| !e.eq_ignore_ascii_case("log"))
            {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    // Check for error indicators
                    let is_error = error_markers.iter().any(|marker| line.contains(marker));
                    if is_error {
                        // Truncate long lines
                        let display_line = if line.len() > 120 {
                            format!("{}...", &line[..117])
                        } else {
                            line.to_string()
                        };

                        error_lines.push(format!("[{file_name}] {display_line}"));

                        if error_lines.len() >= MAX_ERRORS {
                            break;
                        }
                    }
                }
            }

            if error_lines.len() >= MAX_ERRORS {
                break;
            }
        }
    }

    if error_lines.is_empty() {
        return None;
    }

    let mut section = String::from("## Error Patterns Detected\n\n");
    section.push_str("The following errors were found in the logs:\n\n");
    section.push_str("```\n");
    for line in &error_lines {
        section.push_str(line);
        section.push('\n');
    }
    section.push_str("```\n");

    Some(section)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::LogEvent;
    use tempfile::tempdir;
    use tokio::sync::broadcast;

    fn create_test_task_manager() -> TaskManager {
        let (tx, _) = broadcast::channel::<LogEvent>(10);
        let mut tm = TaskManager::new(tx);

        // Create some test tasks
        let id1 = tm.create("Setup", "Set up the project", vec![]);
        tm.update_status(
            &id1,
            &TaskStatus::Complete {
                success: true,
                summary: "Project setup completed".to_string(),
            },
        );

        let id2 = tm.create("Build", "Build the project", vec![id1.clone()]);
        tm.update_status(
            &id2,
            &TaskStatus::Complete {
                success: true,
                summary: "Build successful".to_string(),
            },
        );

        let id3 = tm.create("Test", "Run tests", vec![id2.clone()]);
        tm.update_status(
            &id3,
            &TaskStatus::Failed {
                error: "Test failed: assertion error".to_string(),
            },
        );

        tm
    }

    #[test]
    fn test_run_outcome_from_success() {
        let result = TaskResult {
            success: true,
            message: Some("All done".to_string()),
        };
        let stats = RunStats {
            total_tasks: 5,
            completed_tasks: 5,
            failed_tasks: 0,
            skipped_tasks: 0,
            agents_spawned: 3,
            error_count: 0,
            warning_count: 0,
        };

        let outcome = RunOutcome::from_result_and_stats(&result, &stats);
        assert_eq!(outcome, RunOutcome::Success);
    }

    #[test]
    fn test_run_outcome_partial_success() {
        let result = TaskResult {
            success: true,
            message: Some("Completed with issues".to_string()),
        };
        let stats = RunStats {
            total_tasks: 5,
            completed_tasks: 4,
            failed_tasks: 1,
            skipped_tasks: 0,
            agents_spawned: 3,
            error_count: 2,
            warning_count: 1,
        };

        let outcome = RunOutcome::from_result_and_stats(&result, &stats);
        assert_eq!(outcome, RunOutcome::PartialSuccess);
    }

    #[test]
    fn test_run_outcome_failed() {
        let result = TaskResult {
            success: false,
            message: Some("Run failed".to_string()),
        };
        let stats = RunStats::default();

        let outcome = RunOutcome::from_result_and_stats(&result, &stats);
        assert_eq!(outcome, RunOutcome::Failed);
    }

    #[tokio::test]
    async fn test_build_context_with_empty_dir() {
        let dir = tempdir().unwrap();
        let result = TaskResult {
            success: true,
            message: Some("Test completed".to_string()),
        };
        let (tx, _) = broadcast::channel::<LogEvent>(10);
        let tm = TaskManager::new(tx);

        let context = build_self_improvement_context(dir.path(), &result, &tm)
            .await
            .unwrap();

        // Should contain key sections
        assert!(context.contains("## Run Summary"));
        assert!(context.contains("## Log File Inventory"));
        assert!(context.contains("## Quick Stats"));
        assert!(context.contains("Focus:"));
    }

    #[tokio::test]
    async fn test_build_context_with_log_files() {
        let dir = tempdir().unwrap();

        // Create some log files
        std::fs::write(dir.path().join("planner.log"), "Planning phase output").unwrap();
        std::fs::write(dir.path().join("orchestrator.log"), "Orchestration output").unwrap();
        std::fs::write(
            dir.path().join("implementer-001.log"),
            "Implementer output\n❌ Tool failed: test",
        )
        .unwrap();

        let result = TaskResult {
            success: true,
            message: Some("Test completed".to_string()),
        };
        let tm = create_test_task_manager();

        let context = build_self_improvement_context(dir.path(), &result, &tm)
            .await
            .unwrap();

        // Should mention the log files
        assert!(context.contains("planner.log"));
        assert!(context.contains("orchestrator.log"));
        assert!(context.contains("implementer-001.log"));

        // Should have task summary
        assert!(context.contains("## Task Summary"));

        // Should detect error patterns
        assert!(context.contains("Error Patterns Detected") || context.contains("Errors Found"));
    }

    #[tokio::test]
    async fn test_build_context_with_subtask_dirs() {
        let dir = tempdir().unwrap();

        // Create a subtask directory
        let subtask_dir = dir.path().join("subtask-001");
        std::fs::create_dir_all(&subtask_dir).unwrap();
        std::fs::write(subtask_dir.join("planner.log"), "Subtask planning").unwrap();

        let result = TaskResult {
            success: true,
            message: None,
        };
        let (tx, _) = broadcast::channel::<LogEvent>(10);
        let tm = TaskManager::new(tx);

        let context = build_self_improvement_context(dir.path(), &result, &tm)
            .await
            .unwrap();

        // Should mention subtask directory
        assert!(context.contains("subtask-001"));
    }

    #[test]
    fn test_count_implementer_logs() {
        let dir = tempdir().unwrap();

        // Create implementer logs
        std::fs::write(dir.path().join("implementer-001.log"), "").unwrap();
        std::fs::write(dir.path().join("implementer-002.log"), "").unwrap();
        std::fs::write(dir.path().join("planner.log"), "").unwrap(); // Not counted

        // Create subtask with more implementers
        let subtask = dir.path().join("subtask-001");
        std::fs::create_dir_all(&subtask).unwrap();
        std::fs::write(subtask.join("implementer-001.log"), "").unwrap();

        let count = count_implementer_logs(dir.path());
        assert_eq!(count, 3);
    }

    #[test]
    fn test_build_task_summary_truncates() {
        let tm = create_test_task_manager();
        let summary = build_task_summary(&tm).unwrap();

        // Should have task entries
        assert!(summary.contains("[task001]"));
        assert!(summary.contains("Setup"));
        assert!(summary.contains("✅")); // Success marker
        assert!(summary.contains("❌")); // Failure marker
    }

    #[test]
    fn test_context_size_reasonable() {
        // Create a task manager with many tasks
        let (tx, _) = broadcast::channel::<LogEvent>(10);
        let mut tm = TaskManager::new(tx);

        for i in 0..50 {
            let id = tm.create(
                &format!("Task {i}"),
                &format!("Description for task {i}"),
                vec![],
            );
            tm.update_status(
                &id,
                &TaskStatus::Complete {
                    success: true,
                    summary: format!(
                        "Task {i} completed successfully with some additional details"
                    ),
                },
            );
        }

        let summary = build_task_summary(&tm).unwrap();

        // Should be truncated to 20 tasks
        assert!(summary.contains("... and 30 more tasks"));
    }
}
