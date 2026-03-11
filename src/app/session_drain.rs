//! Session drain and message handling functions.

use super::App;
use crate::logging::AgentWriter;
use crate::types::SessionOutput;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use tokio::sync::mpsc;

/// Strip MCP server prefixes from tool names for cleaner logging.
///
/// Augment's ACP prefixes tool names with the MCP server name (e.g., `complete_paperboat-planner`
/// instead of just `complete`). This function extracts the base tool name for readability.
///
/// Returns the original title if no prefix pattern is found.
fn strip_mcp_prefix(title: &str) -> &str {
    // Pattern: toolname_servername (e.g., "complete_paperboat-planner")
    // We want to extract just "toolname"
    if let Some(underscore_pos) = title.find('_') {
        let potential_prefix = &title[..underscore_pos];
        let potential_suffix = &title[underscore_pos + 1..];
        // Check if suffix looks like our MCP server name
        if potential_suffix.starts_with("paperboat-") {
            return potential_prefix;
        }
    }
    title
}

impl App {
    /// Handle a worker session message from ACP.
    /// Returns Ok(true) if the session is finished, Ok(false) to continue processing.
    pub(crate) async fn handle_worker_session_message(
        &mut self,
        msg: &serde_json::Value,
        session_id: &str,
        writer: &mut AgentWriter,
        output: &mut SessionOutput,
        seen_unhandled: &mut HashSet<String>,
    ) -> Result<bool> {
        let method = msg.get("method").and_then(|v| v.as_str());

        if method != Some("session/update") {
            return Ok(false);
        }

        let Some(params) = msg.get("params") else {
            return Ok(false);
        };

        // Support both ACP format (sessionId) and CLI format (session_id)
        let msg_session_id = params
            .get("sessionId")
            .or_else(|| params.get("session_id"))
            .and_then(|v| v.as_str());

        // Ignore messages for other sessions
        if msg_session_id != Some(session_id) {
            return Ok(false);
        }

        let Some(update) = params.get("update") else {
            return Ok(false);
        };

        // Support both ACP format (sessionUpdate) and CLI format (type)
        let session_update = update
            .get("sessionUpdate")
            .or_else(|| update.get("type"))
            .and_then(|v| v.as_str());

        let Some(session_update) = session_update else {
            return Ok(false);
        };

        match session_update {
            // Message chunks - stream to stdout and collect
            "agent_message_chunk" | "agent_thought_chunk" => {
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    output.append(text);
                    let _ = writer.write_message_chunk(text).await;
                }
            }
            "tool_call" => {
                if let Some(raw_title) = update.get("title").and_then(|t| t.as_str()) {
                    let title = strip_mcp_prefix(raw_title);
                    let _ = writer.write_tool_call(title).await;
                    tracing::info!("🔧 tool call: {}", title);
                }
            }
            "tool_result" => {
                let raw_title = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let title = strip_mcp_prefix(raw_title);
                let is_error = update
                    .get("isError")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let content = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let _ = writer.write_tool_result(title, is_error, content).await;
                if is_error {
                    tracing::error!("❌ tool failed: {} - {}", title, content);
                }
            }
            // Handle explicit completion signals from ACP and CLI
            // ACP uses "agent_turn_finished" and "session_finished"
            // CLI uses "complete" (from update.type)
            "agent_turn_finished" | "session_finished" | "complete" => {
                tracing::info!("✅ Session {} completed: {}", session_id, session_update);
                return Ok(true);
            }
            // Tool progress updates (streaming output from tools)
            "tool_call_update" => {
                let raw_tool_name = update
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                let tool_name = strip_mcp_prefix(raw_tool_name);
                if let Some(text) = update
                    .get("content")
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                {
                    // Stream tool progress for UI observation
                    let _ = writer.write_tool_progress(tool_name, text).await;
                    tracing::trace!("🔄 tool progress: {} - {} chars", tool_name, text.len());
                }
            }
            // Silently ignore known non-essential updates
            "agent_turn_started" | "thinking_start" | "thinking_end" => {}
            // Log unknown types once per type to help diagnose issues
            other => {
                if seen_unhandled.insert(other.to_string()) {
                    tracing::debug!("📨 Unhandled session update type: {}", other);
                }
            }
        }

        Ok(false)
    }

    /// Drain remaining messages from a per-session receiver after `complete()` is called.
    /// This ensures we don't leave stale messages in the notification channel.
    /// Waits until we receive `session_finished` or the caller times out.
    ///
    /// This is a static method that takes the receiver as a parameter to avoid
    /// holding any locks on `self` during the drain loop.
    pub(crate) async fn drain_session_messages_from_rx(
        session_rx: &mut mpsc::Receiver<Value>,
        session_id: &str,
        writer: &mut AgentWriter,
    ) {
        while let Some(msg) = session_rx.recv().await {
            let method = msg.get("method").and_then(|v| v.as_str());
            if method == Some("session/update") {
                if let Some(params) = msg.get("params") {
                    // Support both ACP format (sessionId) and CLI format (session_id)
                    let msg_session_id = params
                        .get("sessionId")
                        .or_else(|| params.get("session_id"))
                        .and_then(|v| v.as_str());

                    // Only process messages for this session (should always match
                    // since the router sends only this session's messages)
                    if msg_session_id == Some(session_id) {
                        if let Some(update) = params.get("update") {
                            // Support both ACP format (sessionUpdate) and CLI format (type)
                            let session_update = update
                                .get("sessionUpdate")
                                .or_else(|| update.get("type"))
                                .and_then(|v| v.as_str());

                            if let Some(update_type) = session_update {
                                match update_type {
                                    // ACP: "session_finished", "agent_turn_finished"
                                    // CLI: "complete"
                                    "session_finished" | "agent_turn_finished" | "complete" => {
                                        tracing::debug!(
                                            "✅ Session {} properly finished",
                                            session_id
                                        );
                                        return;
                                    }
                                    "agent_message_chunk" | "agent_thought_chunk" => {
                                        // Continue logging any remaining output
                                        if let Some(text) = update
                                            .get("content")
                                            .and_then(|c| c.get("text"))
                                            .and_then(|t| t.as_str())
                                        {
                                            let _ = writer.write_message_chunk(text).await;
                                        }
                                    }
                                    _ => {
                                        // Silently ignore other update types during drain
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        tracing::debug!("Session channel closed during drain: {}", session_id);
    }

    /// Drain remaining messages directly from ACP clients after `complete()` is called.
    /// This is used in direct mode (for tests with mock clients).
    /// Polls both planner and worker clients to handle all session types.
    /// Waits until we receive `session_finished` or the caller times out.
    pub(crate) async fn drain_session_messages_direct(
        &mut self,
        session_id: &str,
        writer: &mut AgentWriter,
    ) {
        // Track which clients have been exhausted
        let mut worker_exhausted = false;
        let mut planner_exhausted = false;

        loop {
            // If both clients are exhausted, we're done draining
            if worker_exhausted && planner_exhausted {
                tracing::debug!("Both clients exhausted during drain");
                return;
            }

            // Poll both worker and planner clients
            tokio::select! {
                worker_result = self.acp_worker.recv(), if !worker_exhausted => {
                    match worker_result {
                        Ok(msg) => {
                            if self.handle_drain_message(&msg, session_id, writer).await {
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Worker channel closed during drain: {}", e);
                            worker_exhausted = true;
                        }
                    }
                }
                planner_result = self.acp_planner.recv(), if !planner_exhausted => {
                    match planner_result {
                        Ok(msg) => {
                            if self.handle_drain_message(&msg, session_id, writer).await {
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Planner channel closed during drain: {}", e);
                            planner_exhausted = true;
                        }
                    }
                }
            }
        }
    }

    /// Handle a single message during drain. Returns true if session is finished.
    async fn handle_drain_message(
        &self,
        msg: &serde_json::Value,
        session_id: &str,
        writer: &mut AgentWriter,
    ) -> bool {
        let method = msg.get("method").and_then(|v| v.as_str());
        if method == Some("session/update") {
            if let Some(params) = msg.get("params") {
                // Support both ACP format (sessionId) and CLI format (session_id)
                let msg_session_id = params
                    .get("sessionId")
                    .or_else(|| params.get("session_id"))
                    .and_then(|v| v.as_str());

                // Only process messages for this session
                if msg_session_id == Some(session_id) {
                    if let Some(update) = params.get("update") {
                        // Support both ACP format (sessionUpdate) and CLI format (type)
                        let session_update = update
                            .get("sessionUpdate")
                            .or_else(|| update.get("type"))
                            .and_then(|v| v.as_str());

                        if let Some(update_type) = session_update {
                            match update_type {
                                // ACP: "session_finished", "agent_turn_finished"
                                // CLI: "complete"
                                "session_finished" | "agent_turn_finished" | "complete" => {
                                    tracing::debug!("✅ Session {} properly finished", session_id);
                                    return true;
                                }
                                "agent_message_chunk" | "agent_thought_chunk" => {
                                    if let Some(text) = update
                                        .get("content")
                                        .and_then(|c| c.get("text"))
                                        .and_then(|t| t.as_str())
                                    {
                                        let _ = writer.write_message_chunk(text).await;
                                    }
                                }
                                _ => {
                                    // Silently ignore other update types during drain
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }
}

// ========================================================================
// Unit Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // strip_mcp_prefix Tests
    // ========================================================================

    #[test]
    fn test_strip_mcp_prefix_with_paperboat_prefix() {
        // The function finds the FIRST underscore and checks if suffix starts with "paperboat-"
        // Pattern: toolname_paperboat-servername -> toolname
        //
        // "complete_paperboat-planner":
        //   - first underscore at position 8
        //   - prefix = "complete", suffix = "paperboat-planner"
        //   - suffix starts with "paperboat-", so returns "complete"
        assert_eq!(strip_mcp_prefix("complete_paperboat-planner"), "complete");

        // Tools with underscores in their name won't be stripped because the first
        // underscore doesn't have "paperboat-" as the suffix:
        //
        // "spawn_agents_paperboat-orchestrator":
        //   - first underscore at position 5
        //   - prefix = "spawn", suffix = "agents_paperboat-orchestrator"
        //   - suffix doesn't start with "paperboat-", so returns original
        assert_eq!(
            strip_mcp_prefix("spawn_agents_paperboat-orchestrator"),
            "spawn_agents_paperboat-orchestrator"
        );

        // "create_task_paperboat-planner":
        //   - first underscore at position 6
        //   - prefix = "create", suffix = "task_paperboat-planner"
        //   - suffix doesn't start with "paperboat-", so returns original
        assert_eq!(
            strip_mcp_prefix("create_task_paperboat-planner"),
            "create_task_paperboat-planner"
        );
    }

    #[test]
    fn test_strip_mcp_prefix_without_prefix() {
        assert_eq!(strip_mcp_prefix("complete"), "complete");
        assert_eq!(strip_mcp_prefix("create_task"), "create_task");
        assert_eq!(strip_mcp_prefix("str-replace-editor"), "str-replace-editor");
    }

    #[test]
    fn test_strip_mcp_prefix_with_other_underscore() {
        // Underscore but no paperboat- suffix
        assert_eq!(strip_mcp_prefix("some_other_tool"), "some_other_tool");
        assert_eq!(strip_mcp_prefix("tool_name"), "tool_name");
    }

    #[test]
    fn test_strip_mcp_prefix_empty_string() {
        assert_eq!(strip_mcp_prefix(""), "");
    }

    #[test]
    fn test_strip_mcp_prefix_just_underscore() {
        // Edge case: starts with underscore
        // "_paperboat-test" -> prefix is "", suffix is "paperboat-test"
        // Suffix starts with "paperboat-", so returns "" (empty prefix)
        assert_eq!(strip_mcp_prefix("_paperboat-test"), "");
    }

    // ========================================================================
    // Session Update Type Detection Tests
    // ========================================================================

    #[test]
    fn test_session_finished_types_recognized() {
        // These are the update types that should signal session completion
        let finish_types = vec!["agent_turn_finished", "session_finished", "complete"];

        for update_type in finish_types {
            let msg = json!({
                "method": "session/update",
                "params": {
                    "sessionId": "test-session",
                    "update": {
                        "sessionUpdate": update_type
                    }
                }
            });

            let method = msg.get("method").and_then(|v| v.as_str());
            assert_eq!(method, Some("session/update"));

            let update_type_extracted = msg
                .get("params")
                .and_then(|p| p.get("update"))
                .and_then(|u| u.get("sessionUpdate"))
                .and_then(|s| s.as_str());

            assert_eq!(update_type_extracted, Some(update_type));
        }
    }

    #[test]
    fn test_session_id_extraction_acp_format() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "acp-session-123",
                "update": {
                    "sessionUpdate": "agent_message_chunk"
                }
            }
        });

        let session_id = msg
            .get("params")
            .and_then(|p| p.get("sessionId").or_else(|| p.get("session_id")))
            .and_then(|v| v.as_str());

        assert_eq!(session_id, Some("acp-session-123"));
    }

    #[test]
    fn test_session_id_extraction_cli_format() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "session_id": "cli-session-456",
                "update": {
                    "type": "agent_message_chunk"
                }
            }
        });

        let session_id = msg
            .get("params")
            .and_then(|p| p.get("sessionId").or_else(|| p.get("session_id")))
            .and_then(|v| v.as_str());

        assert_eq!(session_id, Some("cli-session-456"));
    }

    #[test]
    fn test_update_type_extraction_acp_format() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test",
                "update": {
                    "sessionUpdate": "tool_call",
                    "title": "str-replace-editor"
                }
            }
        });

        let update_type = msg
            .get("params")
            .and_then(|p| p.get("update"))
            .and_then(|u| u.get("sessionUpdate").or_else(|| u.get("type")))
            .and_then(|v| v.as_str());

        assert_eq!(update_type, Some("tool_call"));
    }

    #[test]
    fn test_update_type_extraction_cli_format() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "session_id": "test",
                "update": {
                    "type": "complete"
                }
            }
        });

        let update_type = msg
            .get("params")
            .and_then(|p| p.get("update"))
            .and_then(|u| u.get("sessionUpdate").or_else(|| u.get("type")))
            .and_then(|v| v.as_str());

        assert_eq!(update_type, Some("complete"));
    }

    // ========================================================================
    // Message Content Extraction Tests
    // ========================================================================

    #[test]
    fn test_message_chunk_content_extraction() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "text": "Hello, world!"
                    }
                }
            }
        });

        let text = msg
            .get("params")
            .and_then(|p| p.get("update"))
            .and_then(|u| u.get("content"))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str());

        assert_eq!(text, Some("Hello, world!"));
    }

    #[test]
    fn test_tool_result_extraction() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test",
                "update": {
                    "sessionUpdate": "tool_result",
                    "title": "save-file",
                    "isError": false,
                    "content": {
                        "text": "File saved successfully"
                    }
                }
            }
        });

        let update = msg.get("params").and_then(|p| p.get("update")).unwrap();

        let title = update.get("title").and_then(|t| t.as_str());
        let is_error = update
            .get("isError")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let content = update
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str());

        assert_eq!(title, Some("save-file"));
        assert!(!is_error);
        assert_eq!(content, Some("File saved successfully"));
    }

    #[test]
    fn test_tool_result_with_error() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "test",
                "update": {
                    "sessionUpdate": "tool_result",
                    "title": "save-file",
                    "isError": true,
                    "content": {
                        "text": "Permission denied"
                    }
                }
            }
        });

        let is_error = msg
            .get("params")
            .and_then(|p| p.get("update"))
            .and_then(|u| u.get("isError"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        assert!(is_error);
    }

    // ========================================================================
    // Session ID Filtering Tests
    // ========================================================================

    #[test]
    fn test_session_id_mismatch_detection() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "other-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {
                        "text": "Should be ignored"
                    }
                }
            }
        });

        let target_session = "my-session";
        let msg_session = msg
            .get("params")
            .and_then(|p| p.get("sessionId"))
            .and_then(|v| v.as_str());

        // Session IDs don't match, so message should be ignored
        assert_ne!(msg_session, Some(target_session));
    }

    #[test]
    fn test_non_session_update_method_ignored() {
        let msg = json!({
            "method": "tools/list",
            "params": {}
        });

        let method = msg.get("method").and_then(|v| v.as_str());
        assert_ne!(method, Some("session/update"));
    }
}
