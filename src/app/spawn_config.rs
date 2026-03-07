//! Configuration types and builders for agent spawning.
//!
//! This module provides the types and helper functions for building agent
//! spawn configurations, including prompt validation and completion instructions.

use anyhow::Result;

/// Result of an agent's execution.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// The role of the agent (e.g., "implementer")
    pub role: String,
    /// The task that was assigned to the agent.
    /// Note: This is part of the public API but may not be read directly in all cases.
    #[allow(dead_code)]
    pub task: String,
    /// Whether the agent completed successfully
    pub success: bool,
    /// Optional message from the agent.
    /// Note: Part of the API, may not be read directly in all cases.
    #[allow(dead_code)]
    pub message: Option<String>,
    /// Task IDs that were suggested by this agent via `add_tasks`.
    /// These tasks have been created in the TaskManager with `NotStarted` status
    /// and must be addressed (executed or skipped) before completion.
    pub suggested_task_ids: Vec<String>,
}

/// Known placeholders that should be replaced in prompt templates.
pub const KNOWN_PLACEHOLDERS: &[&str] = &["{task}", "{user_goal}", "{context}"];

/// Instructions appended to custom agent prompts for completion signaling.
pub const CUSTOM_AGENT_COMPLETION_INSTRUCTIONS: &str = r"

## When Done

Call `complete` with:
- **success**: Whether you accomplished your task (true/false)
- **message**: Brief summary of what you found or did
- **notes** (optional): Context for future agents or the orchestrator
- **add_tasks** (optional): Any work you discovered that should be done";

/// Validates that no known placeholders remain unreplaced in the prompt.
///
/// Returns an error if any placeholder from `KNOWN_PLACEHOLDERS` is found.
pub fn validate_no_unreplaced_placeholders(prompt: &str, role: &str) -> Result<()> {
    let unreplaced: Vec<&str> = KNOWN_PLACEHOLDERS
        .iter()
        .filter(|p| prompt.contains(*p))
        .copied()
        .collect();

    if !unreplaced.is_empty() {
        anyhow::bail!(
            "Prompt for role '{}' has unreplaced placeholders: {}",
            role,
            unreplaced.join(", ")
        );
    }

    Ok(())
}

/// Builds the full prompt for a custom agent.
///
/// Appends completion instructions to the custom prompt so agents know how to signal completion.
pub fn build_custom_prompt(custom_prompt: &str) -> String {
    format!("{custom_prompt}{CUSTOM_AGENT_COMPLETION_INSTRUCTIONS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_no_unreplaced_placeholders_success() {
        let prompt = "This is a clean prompt with no placeholders.";
        let result = validate_no_unreplaced_placeholders(prompt, "implementer");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_no_unreplaced_placeholders_task() {
        let prompt = "Please complete this {task} now.";
        let result = validate_no_unreplaced_placeholders(prompt, "implementer");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("{task}"));
        assert!(err.contains("implementer"));
    }

    #[test]
    fn test_validate_no_unreplaced_placeholders_multiple() {
        let prompt = "Goal: {user_goal}, Context: {context}";
        let result = validate_no_unreplaced_placeholders(prompt, "planner");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("{user_goal}"));
        assert!(err.contains("{context}"));
    }

    #[test]
    fn test_build_custom_prompt() {
        let custom = "You are a custom agent.";
        let result = build_custom_prompt(custom);

        assert!(result.starts_with(custom));
        assert!(result.contains("## When Done"));
        assert!(result.contains("success"));
        assert!(result.contains("message"));
        assert!(result.contains("notes"));
        assert!(result.contains("add_tasks"));
    }

    #[test]
    fn test_agent_result_creation() {
        let result = AgentResult {
            role: "implementer".to_string(),
            task: "Do something".to_string(),
            success: true,
            message: Some("Done".to_string()),
            suggested_task_ids: vec![],
        };

        assert_eq!(result.role, "implementer");
        assert!(result.success);
        assert_eq!(result.message, Some("Done".to_string()));
        assert!(result.suggested_task_ids.is_empty());
    }
}
