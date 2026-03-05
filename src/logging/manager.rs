//! Run log manager for coordinating logging.

use super::scope::LogScope;
use super::stream::LogEvent;
use chrono::Local;
use std::path::PathBuf;
use tokio::sync::broadcast;

/// Manages the log directory for a single application run.
///
/// Creates the run folder on initialization and provides factory methods
/// for creating scoped loggers for the agent hierarchy.
pub struct RunLogManager {
    /// Root directory for this run (e.g., logs/2026-03-05_14-32-15_abc123/)
    run_dir: PathBuf,
    /// Broadcast channel for streaming log events to observers (e.g., UI)
    event_tx: broadcast::Sender<LogEvent>,
}

impl RunLogManager {
    /// Create a new run log manager.
    ///
    /// Creates the run directory with format: `{base_dir}/{timestamp}_{short_uuid}/`
    pub fn new(base_dir: &str) -> std::io::Result<Self> {
        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        let short_uuid = &uuid::Uuid::new_v4().to_string()[..8];
        let run_dir = PathBuf::from(base_dir).join(format!("{}_{}", timestamp, short_uuid));

        std::fs::create_dir_all(&run_dir)?;

        let (event_tx, _) = broadcast::channel(1000);

        Ok(Self { run_dir, event_tx })
    }

    /// Create a RunLogManager with a specific run directory (for testing).
    #[cfg(test)]
    pub fn with_run_dir(run_dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&run_dir)?;
        let (event_tx, _) = broadcast::channel(1000);
        Ok(Self { run_dir, event_tx })
    }

    /// Get the run directory path.
    pub fn run_dir(&self) -> &PathBuf {
        &self.run_dir
    }

    /// Create the root LogScope for this run.
    ///
    /// The root scope is where the app and root orchestrator logs live.
    pub fn root_scope(&self) -> LogScope {
        LogScope::new(self.run_dir.clone(), self.event_tx.clone(), 0)
    }

    /// Subscribe to log events for streaming to UI.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEvent> {
        self.event_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_run_dir_creation() {
        let base_dir = tempdir().unwrap();
        let manager = RunLogManager::new(base_dir.path().to_str().unwrap()).unwrap();

        // Verify run directory was created
        assert!(manager.run_dir().exists());

        // Verify naming format (YYYY-MM-DD_HH-MM-SS_xxxxxxxx)
        let dir_name = manager.run_dir().file_name().unwrap().to_str().unwrap();
        let parts: Vec<&str> = dir_name.split('_').collect();
        // Format: "2026-03-05_14-32-15_abc12345"
        // Splits into: ["2026-03-05", "14-32-15", "abc12345"]
        assert_eq!(parts.len(), 3);

        // Verify date format
        assert_eq!(parts[0].len(), 10); // YYYY-MM-DD
        assert!(parts[0].contains('-'));

        // Verify time format
        assert_eq!(parts[1].len(), 8); // HH-MM-SS
        assert!(parts[1].contains('-'));

        // Verify short UUID (8 chars)
        assert_eq!(parts[2].len(), 8);
    }

    #[test]
    fn test_root_scope() {
        let base_dir = tempdir().unwrap();
        let manager = RunLogManager::new(base_dir.path().to_str().unwrap()).unwrap();

        let root = manager.root_scope();
        assert_eq!(root.depth(), 0);
        assert_eq!(root.dir(), manager.run_dir());
    }

    #[tokio::test]
    async fn test_subscribe() {
        let base_dir = tempdir().unwrap();
        let manager = RunLogManager::new(base_dir.path().to_str().unwrap()).unwrap();

        let mut rx = manager.subscribe();
        let root = manager.root_scope();

        // Trigger an event via child scope creation
        let _child = root.child_scope("Test subtask").await.unwrap();

        // Should receive the SubtaskCreated event
        let event = rx.recv().await.unwrap();
        match event {
            LogEvent::SubtaskCreated { parent_depth, new_depth, .. } => {
                assert_eq!(parent_depth, 0);
                assert_eq!(new_depth, 1);
            }
            _ => panic!("Expected SubtaskCreated event"),
        }
    }

    #[tokio::test]
    async fn test_always_planner_first_structure() {
        let dir = tempdir().unwrap();
        let run_dir = dir.path().join("test-run");
        let manager = RunLogManager::with_run_dir(run_dir.clone()).unwrap();
        let root = manager.root_scope();

        // Root level: planner → orchestrator (always present)
        let _root_planner = root.planner_writer().await.unwrap();
        let _root_orch = root.orchestrator_writer().await.unwrap();
        let _impl1 = root.implementer_writer().await.unwrap();

        // Verify root structure
        assert!(run_dir.join("planner.log").exists());
        assert!(run_dir.join("orchestrator.log").exists());
        assert!(run_dir.join("implementer-001.log").exists());
    }
}

