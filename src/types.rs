//! Core types for the orchestrator

use serde::{Deserialize, Serialize};

/// Result of completing a task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskResult {
    pub success: bool,
    pub message: Option<String>,
}

/// A plan entry from ACP plan updates
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

/// A complete plan from ACP
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
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
    // PlanEntry Tests
    // ========================================================================

    #[test]
    fn test_plan_entry_deserialization() {
        let json = json!({
            "content": "Implement user authentication",
            "priority": "high",
            "status": "not_started"
        });
        let entry: PlanEntry = serde_json::from_value(json).unwrap();

        assert_eq!(entry.content, "Implement user authentication");
        assert_eq!(entry.priority, "high");
        assert_eq!(entry.status, "not_started");
    }

    #[test]
    fn test_plan_entry_various_statuses() {
        for status in ["not_started", "in_progress", "complete", "blocked"] {
            let json = json!({
                "content": "Task",
                "priority": "medium",
                "status": status
            });
            let entry: PlanEntry = serde_json::from_value(json).unwrap();
            assert_eq!(entry.status, status);
        }
    }

    #[test]
    fn test_plan_entry_various_priorities() {
        for priority in ["low", "medium", "high", "critical"] {
            let json = json!({
                "content": "Task",
                "priority": priority,
                "status": "not_started"
            });
            let entry: PlanEntry = serde_json::from_value(json).unwrap();
            assert_eq!(entry.priority, priority);
        }
    }

    #[test]
    fn test_plan_entry_round_trip() {
        let original = PlanEntry {
            content: "Write tests".to_string(),
            priority: "high".to_string(),
            status: "in_progress".to_string(),
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: PlanEntry = serde_json::from_str(&json_str).unwrap();

        assert_eq!(original, deserialized);
    }

    // ========================================================================
    // Plan Tests
    // ========================================================================

    #[test]
    fn test_plan_with_multiple_entries() {
        let json = json!({
            "entries": [
                {"content": "Task 1", "priority": "high", "status": "complete"},
                {"content": "Task 2", "priority": "medium", "status": "in_progress"},
                {"content": "Task 3", "priority": "low", "status": "not_started"}
            ]
        });
        let plan: Plan = serde_json::from_value(json).unwrap();

        assert_eq!(plan.entries.len(), 3);
        assert_eq!(plan.entries[0].content, "Task 1");
        assert_eq!(plan.entries[1].status, "in_progress");
        assert_eq!(plan.entries[2].priority, "low");
    }

    #[test]
    fn test_empty_plan() {
        let json = json!({
            "entries": []
        });
        let plan: Plan = serde_json::from_value(json).unwrap();

        assert!(plan.entries.is_empty());
    }

    #[test]
    fn test_plan_from_acp_update_format() {
        // Simulates what wait_for_plan extracts from ACP session/update
        let entries_json = json!([
            {"content": "Design API", "priority": "high", "status": "not_started"},
            {"content": "Implement endpoints", "priority": "high", "status": "not_started"},
            {"content": "Write tests", "priority": "medium", "status": "not_started"}
        ]);

        let entries: Vec<PlanEntry> = serde_json::from_value(entries_json).unwrap();
        let plan = Plan { entries };

        assert_eq!(plan.entries.len(), 3);
        assert_eq!(plan.entries[0].content, "Design API");
    }

    #[test]
    fn test_plan_round_trip() {
        let original = Plan {
            entries: vec![
                PlanEntry {
                    content: "First task".to_string(),
                    priority: "high".to_string(),
                    status: "complete".to_string(),
                },
                PlanEntry {
                    content: "Second task".to_string(),
                    priority: "low".to_string(),
                    status: "not_started".to_string(),
                },
            ],
        };
        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: Plan = serde_json::from_str(&json_str).unwrap();

        assert_eq!(original, deserialized);
    }
}
