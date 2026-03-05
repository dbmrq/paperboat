//! Agent log writer for individual log files.

use super::stream::LogEvent;
use chrono::Local;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::broadcast;

/// Type of agent for log identification.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentType {
    Orchestrator,
    Planner,
    Implementer { index: u32 },
}

impl AgentType {
    pub fn name(&self) -> String {
        match self {
            AgentType::Orchestrator => "orchestrator".to_string(),
            AgentType::Planner => "planner".to_string(),
            AgentType::Implementer { index } => format!("implementer-{:03}", index),
        }
    }
}

/// Writes agent output to a log file and broadcasts events.
pub struct AgentWriter {
    file: BufWriter<File>,
    path: PathBuf,
    agent_type: AgentType,
    event_tx: broadcast::Sender<LogEvent>,
    depth: u32,
    /// Session ID this writer is associated with (set after session creation)
    session_id: Option<String>,
    /// Timestamp when the writer was created (for duration calculation)
    start_time: std::time::Instant,
}

impl AgentWriter {
    pub async fn new(
        path: PathBuf,
        agent_type: AgentType,
        event_tx: broadcast::Sender<LogEvent>,
        depth: u32,
    ) -> std::io::Result<Self> {
        let file = File::create(&path).await?;

        Ok(Self {
            file: BufWriter::new(file),
            path,
            agent_type,
            event_tx,
            depth,
            session_id: None,
            start_time: std::time::Instant::now(),
        })
    }

    /// Get the path to the log file.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Associate this writer with a session ID.
    pub fn set_session_id(&mut self, session_id: String) {
        self.session_id = Some(session_id);
    }

    /// Get the agent name (e.g., "implementer-001", "planner", "orchestrator").
    pub fn agent_name(&self) -> String {
        self.agent_type.name()
    }

    /// Write a header at the start of the log file.
    /// The `task` parameter is a brief description; `prompt` is the full prompt sent to the agent.
    pub async fn write_header(&mut self, task: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let session_info = self
            .session_id
            .as_ref()
            .map(|id| format!("\nSession: {}", id))
            .unwrap_or_default();
        let header = format!(
            "=== {} Log ===\nStarted: {}{}\nTask: {}\n{}\n\n",
            self.agent_type.name().to_uppercase(),
            timestamp,
            session_info,
            task,
            "=".repeat(60)
        );
        self.file.write_all(header.as_bytes()).await?;
        self.file.flush().await
    }

    /// Write a header with the full prompt at the start of the log file.
    /// This is useful for debugging to see exactly what prompt was sent to the agent.
    pub async fn write_header_with_prompt(&mut self, task: &str, prompt: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let session_info = self
            .session_id
            .as_ref()
            .map(|id| format!("\nSession: {}", id))
            .unwrap_or_default();
        let header = format!(
            "=== {} Log ===\nStarted: {}{}\nTask: {}\n{}\n\n## Full Prompt Sent to Agent\n\n{}\n\n{}\n\n",
            self.agent_type.name().to_uppercase(),
            timestamp,
            session_info,
            task,
            "=".repeat(60),
            prompt,
            "=".repeat(60)
        );
        self.file.write_all(header.as_bytes()).await?;
        self.file.flush().await
    }

    /// Write an agent message chunk (streaming text).
    pub async fn write_message_chunk(&mut self, text: &str) -> std::io::Result<()> {
        self.file.write_all(text.as_bytes()).await?;
        // Flush immediately to prevent data loss
        self.file.flush().await?;

        // Broadcast event for UI streaming
        let _ = self.event_tx.send(LogEvent::AgentMessage {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            content: text.to_string(),
        });

        Ok(())
    }

    /// Write a tool call event.
    pub async fn write_tool_call(&mut self, tool_name: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%H:%M:%S");
        let line = format!("\n[{}] 🔧 Tool: {}\n", timestamp, tool_name);
        self.file.write_all(line.as_bytes()).await?;

        let _ = self.event_tx.send(LogEvent::ToolCall {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            tool_name: tool_name.to_string(),
        });

        Ok(())
    }

    /// Write a tool progress update (streaming output from a tool).
    /// This is for real-time progress of long-running tools.
    pub async fn write_tool_progress(
        &mut self,
        tool_name: &str,
        progress_text: &str,
    ) -> std::io::Result<()> {
        // Don't write to file to avoid log spam - just broadcast for UI
        let _ = self.event_tx.send(LogEvent::ToolProgress {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            tool_name: tool_name.to_string(),
            progress_text: progress_text.to_string(),
        });

        Ok(())
    }

    /// Write a tool result (only logs errors to keep logs clean).
    pub async fn write_tool_result(
        &mut self,
        tool_name: &str,
        is_error: bool,
        content: &str,
    ) -> std::io::Result<()> {
        // Only write to file if it's an error - success results are noise
        if is_error {
            let timestamp = Local::now().format("%H:%M:%S");
            let line = format!(
                "[{}] ❌ Tool failed: {}\n{}\n",
                timestamp, tool_name, content
            );
            self.file.write_all(line.as_bytes()).await?;
        }

        let _ = self.event_tx.send(LogEvent::ToolResult {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            tool_name: tool_name.to_string(),
            is_error,
        });

        Ok(())
    }

    /// Write an MCP tool result (always logs both success and failure for MCP tools).
    /// This is used for villalobos MCP tools (implement, decompose) which are more important to track.
    pub async fn write_mcp_tool_result(
        &mut self,
        tool_name: &str,
        success: bool,
        summary: &str,
    ) -> std::io::Result<()> {
        let timestamp = Local::now().format("%H:%M:%S");
        let (icon, status) = if success { ("✅", "success") } else { ("❌", "FAILED") };
        let line = format!(
            "[{}] {} MCP result: {} - {} ({})\n",
            timestamp, icon, tool_name, summary, status
        );
        self.file.write_all(line.as_bytes()).await?;
        self.file.flush().await?;

        let _ = self.event_tx.send(LogEvent::ToolResult {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            tool_name: tool_name.to_string(),
            is_error: !success,
        });

        Ok(())
    }

    /// Write a completion/result message from the agent.
    pub async fn write_result(&mut self, message: &str) -> std::io::Result<()> {
        let timestamp = Local::now().format("%H:%M:%S");
        let line = format!("\n[{}] ✅ Result: {}\n", timestamp, message);
        self.file.write_all(line.as_bytes()).await
    }

    /// Write completion marker and flush.
    pub async fn finalize(&mut self, success: bool) -> std::io::Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let status = if success { "SUCCESS" } else { "FAILURE" };
        let duration = self.start_time.elapsed();
        let duration_str = format_duration(duration);
        let footer = format!(
            "\n{}\nCompleted: {} - {} ({})\n",
            "=".repeat(60),
            timestamp,
            status,
            duration_str
        );
        self.file.write_all(footer.as_bytes()).await?;
        self.file.flush().await?;

        let _ = self.event_tx.send(LogEvent::AgentComplete {
            agent_type: self.agent_type.clone(),
            session_id: self.session_id.clone(),
            depth: self.depth,
            success,
        });

        Ok(())
    }
}

/// Format a duration as a human-readable string.
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();
    if secs >= 60 {
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{}m {}s", mins, secs)
    } else if secs > 0 {
        format!("{}.{:03}s", secs, millis)
    } else {
        format!("{}ms", millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_agent_writer_creates_file() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);

        let mut writer = AgentWriter::new(
            dir.path().join("test.log"),
            AgentType::Implementer { index: 1 },
            tx,
            0,
        )
        .await
        .unwrap();

        writer.write_header("Test task").await.unwrap();
        writer.write_message_chunk("Hello world").await.unwrap();
        writer.finalize(true).await.unwrap();

        let content = std::fs::read_to_string(dir.path().join("test.log")).unwrap();
        assert!(content.contains("IMPLEMENTER-001"));
        assert!(content.contains("Test task"));
        assert!(content.contains("Hello world"));
        assert!(content.contains("SUCCESS"));
    }

    #[tokio::test]
    async fn test_agent_type_names() {
        assert_eq!(AgentType::Orchestrator.name(), "orchestrator");
        assert_eq!(AgentType::Planner.name(), "planner");
        assert_eq!(AgentType::Implementer { index: 1 }.name(), "implementer-001");
        assert_eq!(AgentType::Implementer { index: 42 }.name(), "implementer-042");
    }

    #[tokio::test]
    async fn test_tool_call_logging() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);

        let mut writer = AgentWriter::new(
            dir.path().join("tools.log"),
            AgentType::Planner,
            tx,
            1,
        )
        .await
        .unwrap();

        writer.write_tool_call("view").await.unwrap();
        writer.write_tool_result("view", false, "file contents...").await.unwrap();  // Success - not logged
        writer.write_tool_result("save-file", true, "permission denied").await.unwrap();  // Error - logged
        writer.finalize(false).await.unwrap();

        let content = std::fs::read_to_string(dir.path().join("tools.log")).unwrap();
        assert!(content.contains("Tool: view"));
        // Success results are no longer logged to file (only errors)
        assert!(!content.contains("Tool result: view"));
        assert!(content.contains("❌ Tool failed: save-file"));
        assert!(content.contains("FAILURE"));
    }

    #[tokio::test]
    async fn test_broadcast_events() {
        let dir = tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::broadcast::channel(10);

        let mut writer = AgentWriter::new(
            dir.path().join("events.log"),
            AgentType::Orchestrator,
            tx,
            0,
        )
        .await
        .unwrap();

        writer.set_session_id("test-session-123".to_string());
        writer.write_message_chunk("Hello").await.unwrap();

        // Verify event was broadcast
        let event = rx.recv().await.unwrap();
        match event {
            LogEvent::AgentMessage { agent_type, session_id, depth, content } => {
                assert_eq!(agent_type, AgentType::Orchestrator);
                assert_eq!(session_id, Some("test-session-123".to_string()));
                assert_eq!(depth, 0);
                assert_eq!(content, "Hello");
            }
            _ => panic!("Expected AgentMessage event"),
        }
    }

    #[tokio::test]
    async fn test_write_header_with_prompt() {
        let dir = tempdir().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(10);

        let mut writer = AgentWriter::new(
            dir.path().join("with_prompt.log"),
            AgentType::Planner,
            tx,
            0,
        )
        .await
        .unwrap();

        writer.set_session_id("session-abc".to_string());
        let task = "Create a test plan";
        let prompt = "You are a planner agent.\n\nCreate a comprehensive plan for:\n\nCreate a test plan";
        writer.write_header_with_prompt(task, prompt).await.unwrap();
        writer.finalize(true).await.unwrap();

        let content = std::fs::read_to_string(dir.path().join("with_prompt.log")).unwrap();
        assert!(content.contains("PLANNER"));
        assert!(content.contains("Session: session-abc"));
        assert!(content.contains("Task: Create a test plan"));
        assert!(content.contains("## Full Prompt Sent to Agent"));
        assert!(content.contains("You are a planner agent"));
        assert!(content.contains("Create a comprehensive plan for"));
        assert!(content.contains("SUCCESS"));
    }
}

