//! Logging scope for hierarchy management.

use super::stream::LogEvent;
use super::writer::{AgentType, AgentWriter};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// A logging scope representing one level in the agent hierarchy.
///
/// Each scope can create writers for agents at this level and
/// child scopes for nested decomposition (subtasks).
#[derive(Clone)]
pub struct LogScope {
    /// Directory for this scope's logs
    dir: PathBuf,
    /// Broadcast channel for log events
    event_tx: broadcast::Sender<LogEvent>,
    /// Depth in the hierarchy (0 = root)
    depth: u32,
    /// Counter for implementers at this level
    implementer_count: Arc<RwLock<u32>>,
    /// Counter for subtasks (decomposed work) at this level
    subtask_count: Arc<RwLock<u32>>,
}

impl LogScope {
    /// Create a new logging scope.
    pub fn new(dir: PathBuf, event_tx: broadcast::Sender<LogEvent>, depth: u32) -> Self {
        Self {
            dir,
            event_tx,
            depth,
            implementer_count: Arc::new(RwLock::new(0)),
            subtask_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Create a writer for the planner at this scope level.
    /// In the "always planner first" architecture, this is called first.
    pub async fn planner_writer(&self) -> std::io::Result<AgentWriter> {
        AgentWriter::new(
            self.dir.join("planner.log"),
            AgentType::Planner,
            self.event_tx.clone(),
            self.depth,
        )
        .await
    }

    /// Create a writer for the orchestrator at this scope level.
    /// Called after planner creates the plan.
    pub async fn orchestrator_writer(&self) -> std::io::Result<AgentWriter> {
        AgentWriter::new(
            self.dir.join("orchestrator.log"),
            AgentType::Orchestrator,
            self.event_tx.clone(),
            self.depth,
        )
        .await
    }

    /// Create a writer for a new implementer at this scope level.
    /// Returns a numbered writer (implementer-001.log, etc.)
    pub async fn implementer_writer(&self) -> std::io::Result<AgentWriter> {
        let mut count = self.implementer_count.write().await;
        *count += 1;
        let filename = format!("implementer-{:03}.log", *count);

        AgentWriter::new(
            self.dir.join(&filename),
            AgentType::Implementer { index: *count },
            self.event_tx.clone(),
            self.depth,
        )
        .await
    }

    /// Create a writer for the self-improver agent.
    /// Creates a self-improver.log file in the scope directory.
    pub async fn self_improver_writer(&self) -> std::io::Result<AgentWriter> {
        AgentWriter::new(
            self.dir.join("self-improver.log"),
            AgentType::SelfImprover,
            self.event_tx.clone(),
            self.depth,
        )
        .await
    }

    /// Create a child scope for a decompose operation (subtask).
    /// Creates a new subdirectory and returns a `LogScope` for it.
    pub async fn child_scope(&self, task_description: &str) -> std::io::Result<Self> {
        let mut count = self.subtask_count.write().await;
        *count += 1;
        let subdir = self.dir.join(format!("subtask-{:03}", *count));

        std::fs::create_dir_all(&subdir)?;

        // Broadcast subtask creation event
        let _ = self.event_tx.send(LogEvent::SubtaskCreated {
            parent_depth: self.depth,
            new_depth: self.depth + 1,
            path: subdir.to_string_lossy().to_string(),
            task_description: task_description.to_string(),
        });

        Ok(Self::new(subdir, self.event_tx.clone(), self.depth + 1))
    }

    /// Get the directory path for this scope.
    pub const fn dir(&self) -> &PathBuf {
        &self.dir
    }

    /// Get the depth of this scope in the hierarchy.
    #[allow(dead_code)]
    pub const fn depth(&self) -> u32 {
        self.depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_concurrent_implementers() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);
        let scope = LogScope::new(dir.path().to_path_buf(), tx, 0);

        let w1 = scope.implementer_writer().await.unwrap();
        let w2 = scope.implementer_writer().await.unwrap();
        let w3 = scope.implementer_writer().await.unwrap();

        assert!(dir.path().join("implementer-001.log").exists());
        assert!(dir.path().join("implementer-002.log").exists());
        assert!(dir.path().join("implementer-003.log").exists());

        // Verify correct paths
        assert!(w1.path().ends_with("implementer-001.log"));
        assert!(w2.path().ends_with("implementer-002.log"));
        assert!(w3.path().ends_with("implementer-003.log"));
    }

    #[tokio::test]
    async fn test_child_scope_creates_subdirectory() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);
        let scope = LogScope::new(dir.path().to_path_buf(), tx, 0);

        let child1 = scope.child_scope("First subtask").await.unwrap();
        let child2 = scope.child_scope("Second subtask").await.unwrap();

        assert!(dir.path().join("subtask-001").exists());
        assert!(dir.path().join("subtask-002").exists());

        // Verify depth increment
        assert_eq!(child1.depth(), 1);
        assert_eq!(child2.depth(), 1);

        // Create nested child
        let grandchild = child1.child_scope("Nested subtask").await.unwrap();
        assert!(dir.path().join("subtask-001/subtask-001").exists());
        assert_eq!(grandchild.depth(), 2);
    }

    #[tokio::test]
    async fn test_planner_and_orchestrator_writers() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);
        let scope = LogScope::new(dir.path().to_path_buf(), tx, 0);

        let _planner = scope.planner_writer().await.unwrap();
        let _orchestrator = scope.orchestrator_writer().await.unwrap();

        assert!(dir.path().join("planner.log").exists());
        assert!(dir.path().join("orchestrator.log").exists());
    }

    #[tokio::test]
    async fn test_self_improver_writer() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);
        let scope = LogScope::new(dir.path().to_path_buf(), tx, 0);

        let writer = scope.self_improver_writer().await.unwrap();

        assert!(dir.path().join("self-improver.log").exists());
        assert_eq!(writer.agent_name(), "self-improver");
    }

    #[tokio::test]
    async fn test_full_hierarchy() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);
        let root = LogScope::new(dir.path().to_path_buf(), tx, 0);

        // Root level: planner → orchestrator
        let _planner = root.planner_writer().await.unwrap();
        let _orchestrator = root.orchestrator_writer().await.unwrap();
        let _impl1 = root.implementer_writer().await.unwrap();

        // First subtask
        let subtask1 = root.child_scope("Feature implementation").await.unwrap();
        let _sub_planner = subtask1.planner_writer().await.unwrap();
        let _sub_orch = subtask1.orchestrator_writer().await.unwrap();
        let _sub_impl = subtask1.implementer_writer().await.unwrap();

        // Nested subtask
        let nested = subtask1.child_scope("Detailed work").await.unwrap();
        let _nested_impl = nested.implementer_writer().await.unwrap();

        // Verify full structure
        assert!(dir.path().join("planner.log").exists());
        assert!(dir.path().join("orchestrator.log").exists());
        assert!(dir.path().join("implementer-001.log").exists());
        assert!(dir.path().join("subtask-001/planner.log").exists());
        assert!(dir.path().join("subtask-001/orchestrator.log").exists());
        assert!(dir.path().join("subtask-001/implementer-001.log").exists());
        assert!(dir
            .path()
            .join("subtask-001/subtask-001/implementer-001.log")
            .exists());
    }
}
