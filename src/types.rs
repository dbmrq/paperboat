//! Core types for the orchestrator

use serde::{Deserialize, Serialize};

/// Result of completing a task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskResult {
    pub success: bool,
    pub message: Option<String>,
}

/// Output collected from an agent session (planner, implementer, etc.)
/// Contains all message chunks concatenated together.
#[derive(Debug, Clone, Default)]
pub struct SessionOutput {
    /// The full text output from the agent
    pub text: String,
}

impl SessionOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a message chunk to the output
    pub fn append(&mut self, chunk: &str) {
        self.text.push_str(chunk);
    }

    /// Check if there's any output
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // TaskResult Tests
    // ========================================================================

    #[test]
    fn test_task_result_success_with_message() {
        let result = TaskResult {
            success: true,
            message: Some("All tasks completed".to_string()),
        };
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["success"], true);
        assert_eq!(json["message"], "All tasks completed");
    }

    #[test]
    fn test_task_result_success_without_message() {
        let result = TaskResult {
            success: true,
            message: None,
        };
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["success"], true);
        assert!(json["message"].is_null());
    }

    #[test]
    fn test_task_result_failure_with_message() {
        let result = TaskResult {
            success: false,
            message: Some("Build failed".to_string()),
        };
        let json = serde_json::to_value(&result).unwrap();

        assert_eq!(json["success"], false);
        assert_eq!(json["message"], "Build failed");
    }

    #[test]
    fn test_task_result_round_trip() {
        let original = TaskResult {
            success: true,
            message: Some("Done!".to_string()),
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: TaskResult = serde_json::from_str(&json_str).unwrap();

        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_task_result_deserialize_from_json() {
        let json = json!({
            "success": false,
            "message": "Error occurred"
        });
        let result: TaskResult = serde_json::from_value(json).unwrap();

        assert!(!result.success);
        assert_eq!(result.message, Some("Error occurred".to_string()));
    }

    #[test]
    fn test_task_result_deserialize_null_message() {
        let json = json!({
            "success": true,
            "message": null
        });
        let result: TaskResult = serde_json::from_value(json).unwrap();

        assert!(result.success);
        assert!(result.message.is_none());
    }

    // ========================================================================
    // SessionOutput Tests
    // ========================================================================

    #[test]
    fn test_session_output_new() {
        let output = SessionOutput::new();
        assert!(output.is_empty());
        assert_eq!(output.text, "");
    }

    #[test]
    fn test_session_output_append() {
        let mut output = SessionOutput::new();
        output.append("Hello ");
        output.append("world!");
        assert_eq!(output.text, "Hello world!");
        assert!(!output.is_empty());
    }

    #[test]
    fn test_session_output_is_empty_with_whitespace() {
        let mut output = SessionOutput::new();
        output.append("   \n\t  ");
        assert!(output.is_empty());
    }
}
