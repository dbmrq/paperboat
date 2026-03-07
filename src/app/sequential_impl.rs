//! Sequential implementer handling for test mode.
//!
//! This module provides synchronous implement handling used in sequential test mode
//! where the `SessionRouter` is not active.

use super::types::{format_duration_human, truncate_for_log};
use super::App;
use crate::mcp_server::ToolResponse;
use anyhow::{Context, Result};

impl App {
    /// Handle implement tool call, returning a `ToolResponse`.
    /// Used by sequential mode in tests where the `SessionRouter` is not active.
    pub(crate) async fn handle_implement_with_response(
        &mut self,
        task: &str,
        request_id: &str,
    ) -> ToolResponse {
        match self.handle_implement_inner(task).await {
            Ok(summary) => ToolResponse::success(request_id.to_string(), summary),
            Err(e) => ToolResponse::failure(request_id.to_string(), e.to_string()),
        }
    }

    /// Inner implement logic that can fail.
    pub(crate) async fn handle_implement_inner(&mut self, task: &str) -> Result<String> {
        let start_time = std::time::Instant::now();
        let depth = self.current_scope.depth();

        // Create implementer writer (this assigns the implementer number)
        let mut impl_writer = self
            .current_scope
            .implementer_writer()
            .await
            .context("Failed to create implementer writer")?;

        // Get the implementer name for logging
        let impl_name = impl_writer.agent_name();
        tracing::info!(
            "[L{}] 🔨 [{}] Starting: {}",
            depth,
            impl_name,
            truncate_for_log(task, 100)
        );

        // Spawn implementer
        let (impl_session, impl_prompt) = match self.spawn_implementer(task).await {
            Ok(result) => result,
            Err(e) => {
                // Write error to implementer log so it's not empty
                tracing::error!(
                    "[L{}] ❌ [{}] Failed to spawn implementer: {:#}",
                    depth,
                    impl_name,
                    e
                );
                if let Err(write_err) = impl_writer.write_spawn_error(&e).await {
                    tracing::warn!(
                        "Failed to write spawn error to implementer log: {}",
                        write_err
                    );
                }
                if let Err(finalize_err) = impl_writer.finalize(false).await {
                    tracing::warn!(
                        "Failed to finalize implementer log after spawn error: {}",
                        finalize_err
                    );
                }
                return Err(e);
            }
        };
        impl_writer.set_session_id(impl_session.clone());
        if let Err(e) = impl_writer
            .write_header_with_prompt(task, &impl_prompt)
            .await
        {
            tracing::warn!("Failed to write implementer header: {}", e);
        }
        // Emit AgentStarted event for TUI
        impl_writer.emit_agent_started(task);

        // Wait for implementer to finish (with timeout)
        let result = self
            .wait_for_session_output(&impl_session, &mut impl_writer)
            .await;

        let success = result.is_ok();
        let elapsed = start_time.elapsed();
        let elapsed_str = format_duration_human(elapsed);

        // Finalize implementer log
        if let Err(e) = impl_writer.finalize(success).await {
            tracing::warn!("Failed to finalize implementer log: {}", e);
        }

        if !success {
            tracing::error!(
                "❌ [{}] Implementation FAILED after {} - task: {}",
                impl_name,
                elapsed_str,
                truncate_for_log(task, 80)
            );
            return Err(anyhow::anyhow!("Implementation failed for task: {task}"));
        }

        tracing::info!(
            "✅ [{}] Implementation complete ({}) - {}",
            impl_name,
            elapsed_str,
            truncate_for_log(task, 60)
        );

        Ok(format!("Task completed: {task}"))
    }
}
