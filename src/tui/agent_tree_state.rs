//! Agent tree state management.
//!
//! This module contains [`AgentTreeState`], which manages the hierarchical
//! agent tree structure built from `LogEvent`s. It maintains:
//!
//! - A tree of [`AgentNode`]s built from `AgentStarted` and `SubtaskCreated` events
//! - Agent status updates from `AgentComplete` events
//! - The `tui-tree-widget` [`TreeState`] for navigation
//! - Message buffers per agent for the output pane
//!
//! The tree is used by the TUI to display the agent hierarchy in the left panel
//! and to track which agent's output to show in the center panel.

use std::collections::HashMap;

use tui_tree_widget::{TreeItem, TreeState};

use crate::logging::AgentType;

// Re-export from agent_node module
pub use super::agent_node::{AgentNode, AgentStatus};

// ============================================================================
// Agent Tree State
// ============================================================================

/// Manages the agent hierarchy tree and associated metadata.
///
/// This struct maintains:
/// - A tree structure built from `AgentStarted` and `SubtaskCreated` events
/// - Agent status updates from `AgentComplete` events
/// - The `tui-tree-widget` [`TreeState`] for navigation
/// - Message buffers per agent for the output pane
#[derive(Debug)]
pub struct AgentTreeState {
    /// All agents indexed by `session_id`
    agents: HashMap<String, AgentNode>,
    /// Root session IDs (depth 0 agents)
    roots: Vec<String>,
    /// Messages per `session_id` for output display
    messages: HashMap<String, Vec<String>>,
    /// Tree widget navigation state
    pub tree_state: TreeState<String>,
    /// Mapping from depth to current "active" `session_id` at that depth.
    /// Used for determining parent when a new agent starts.
    depth_to_session: HashMap<u32, String>,
}

impl Default for AgentTreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTreeState {
    /// Creates a new empty agent tree state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            roots: Vec::new(),
            messages: HashMap::new(),
            tree_state: TreeState::default(),
            depth_to_session: HashMap::new(),
        }
    }

    /// Handles an `AgentStarted` event.
    pub fn handle_agent_started(
        &mut self,
        agent_type: AgentType,
        session_id: String,
        depth: u32,
        task: String,
    ) {
        let mut node = AgentNode::new(agent_type, session_id.clone(), depth, task);

        // Determine parent based on depth
        if depth > 0 {
            // Look for parent at depth - 1
            if let Some(parent_id) = self.depth_to_session.get(&(depth - 1)).cloned() {
                node.parent_session_id = Some(parent_id.clone());
                // Add as child of parent
                if let Some(parent) = self.agents.get_mut(&parent_id) {
                    parent.children.push(session_id.clone());
                }
            }
        } else {
            // Root level agent
            self.roots.push(session_id.clone());
        }

        // Update depth mapping
        self.depth_to_session.insert(depth, session_id.clone());

        // Store the agent
        self.agents.insert(session_id.clone(), node);

        // Initialize message buffer
        self.messages.insert(session_id, Vec::new());
    }

    /// Handles an `AgentComplete` event.
    pub fn handle_agent_complete(&mut self, session_id: Option<&str>, success: bool) {
        if let Some(id) = session_id {
            if let Some(agent) = self.agents.get_mut(id) {
                agent.status = if success {
                    AgentStatus::Completed
                } else {
                    AgentStatus::Failed
                };
            }
        }
    }

    /// Handles a `SubtaskCreated` event.
    ///
    /// This event signals that a decompose operation is starting at a new depth level.
    /// The `parent_depth` indicates the depth of the orchestrator that initiated the decompose,
    /// and `new_depth` is where the sub-agents will be created.
    ///
    /// Note: The actual parent-child relationship is established when `AgentStarted` events
    /// are received, using the `depth_to_session` mapping. This method prepares context
    /// for better tracking of nested decomposition scenarios.
    pub fn handle_subtask_created(&mut self, parent_depth: u32, _new_depth: u32, _path: &str) {
        // The depth_to_session mapping at parent_depth should already contain
        // the orchestrator that called decompose. When sub-agents start at new_depth,
        // they will correctly look up their parent at parent_depth.
        //
        // This method serves as a hook for future enhancements like:
        // - Tracking decomposition paths for display
        // - Adding markers in the tree for decomposition boundaries
        // - Collecting metrics on decomposition depth
        //
        // For now, we verify the parent depth mapping exists (defensive programming)
        if parent_depth > 0 && !self.depth_to_session.contains_key(&parent_depth) {
            // This shouldn't happen in normal operation, but log if it does
            tracing::warn!(
                "SubtaskCreated at parent_depth {} but no session registered at that depth",
                parent_depth
            );
        }
    }

    /// Handles an `AgentMessage` event (streaming content).
    ///
    /// Streaming chunks are concatenated together. Only when we encounter a newline
    /// do we split into separate messages for display.
    ///
    /// Note: If the last message is a "standalone" message (starts with `>`, `✓`, `✗`, or `+`),
    /// we always start a new message to avoid concatenating streaming content with tool calls/results.
    pub fn handle_agent_message(&mut self, session_id: Option<&str>, content: &str) {
        if let Some(id) = session_id {
            if let Some(messages) = self.messages.get_mut(id) {
                // Pre-process content to add line breaks where tool call descriptions
                // are immediately followed by regular text (e.g., 'Calling: tool "query"Let me...')
                let processed_content = Self::insert_missing_line_breaks(content);

                // Check if we need to start a new message:
                // 1. No messages yet
                // 2. Last message is a standalone message (tool call, result, etc.)
                // 3. This content starts with a standalone prefix (like "Calling:")
                let content_is_standalone = Self::is_standalone_message(&processed_content);
                let should_start_new = messages.is_empty()
                    || messages
                        .last()
                        .is_some_and(|last| Self::is_standalone_message(last))
                    || content_is_standalone;

                if should_start_new {
                    messages.push(String::new());
                }

                // Append content to the last message, handling newlines
                for (i, part) in processed_content.split('\n').enumerate() {
                    if i > 0 {
                        // Start a new message for each newline
                        messages.push(String::new());
                    }
                    let idx = messages.len() - 1;
                    messages[idx].push_str(part);
                }
            }
        }
    }

    /// Insert missing line breaks where tool call descriptions are joined with regular text.
    ///
    /// Detects patterns where a closing quote is followed by a capital letter,
    /// indicating a tool call description ending and regular agent text beginning.
    ///
    /// This only triggers for the LAST closing quote in a string that contains
    /// "Calling:" (indicating it's a tool call description), not for all quotes.
    fn insert_missing_line_breaks(content: &str) -> String {
        // Only apply this heuristic if content contains "Calling:" pattern
        if !content.contains("Calling:") {
            return content.to_string();
        }

        // Find the last closing quote that's followed by a capital letter
        // This is likely the end of a tool call query followed by agent text
        let bytes = content.as_bytes();
        for i in (0..bytes.len().saturating_sub(1)).rev() {
            if bytes[i] == b'"' {
                // Check if next char is uppercase
                if let Some(&next_byte) = bytes.get(i + 1) {
                    if next_byte.is_ascii_uppercase() {
                        // Split at this position
                        let (before, after) = content.split_at(i + 1);
                        return format!("{before}\n{after}");
                    }
                }
            }
        }

        content.to_string()
    }

    /// Returns true if the message is a "standalone" message that should not be appended to.
    ///
    /// Standalone messages are tool calls, tool results, and other formatted outputs that
    /// start with specific prefixes:
    /// - `>` - Tool call formatted by TUI (e.g., "> Calling: ...")
    /// - `✓` - Success result (e.g., "✓ `tool_name` completed")
    /// - `✗` - Error result (e.g., "✗ `tool_name` completed")
    /// - `+` - Added content (e.g., "+ Added file ...")
    /// - `Calling:` - Tool call from agent output (e.g., "Calling: Context engine: ...")
    fn is_standalone_message(msg: &str) -> bool {
        let trimmed = msg.trim_start();
        trimmed.starts_with('>')
            || trimmed.starts_with('✓')
            || trimmed.starts_with('✗')
            || trimmed.starts_with('+')
            || trimmed.starts_with("Calling:")
    }

    /// Adds a standalone message (like tool calls) that should always be on its own line.
    ///
    /// Unlike streaming content, this always starts on a new line.
    pub fn handle_standalone_message(&mut self, session_id: Option<&str>, content: &str) {
        if let Some(id) = session_id {
            if let Some(messages) = self.messages.get_mut(id) {
                // Always add as a new message on its own line
                messages.push(content.to_string());
            }
        }
    }

    /// Gets the messages for a specific agent.
    #[must_use]
    pub fn get_messages(&self, session_id: &str) -> Option<&Vec<String>> {
        self.messages.get(session_id)
    }

    /// Gets agent metadata by `session_id`.
    #[must_use]
    pub fn get_agent(&self, session_id: &str) -> Option<&AgentNode> {
        self.agents.get(session_id)
    }

    /// Returns the currently selected `session_id` from the tree state.
    #[must_use]
    pub fn selected_session_id(&self) -> Option<&str> {
        let selected = self.tree_state.selected();
        selected.last().map(String::as_str)
    }

    /// Returns the number of agents in the tree.
    #[must_use]
    #[allow(dead_code)] // Used in tests and public API
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Builds tree items for rendering with `tui-tree-widget`.
    ///
    /// Returns owned [`TreeItem`]s that can be used for rendering.
    ///
    /// # Arguments
    ///
    /// * `animation_frame` - Current animation frame for animated status indicators
    #[must_use]
    pub fn build_tree_items(&self, animation_frame: u32) -> Vec<TreeItem<'static, String>> {
        self.roots
            .iter()
            .filter_map(|root_id| self.build_tree_item_recursive(root_id, animation_frame))
            .collect()
    }

    /// Recursively builds a tree item and its children.
    fn build_tree_item_recursive(
        &self,
        session_id: &str,
        animation_frame: u32,
    ) -> Option<TreeItem<'static, String>> {
        let agent = self.agents.get(session_id)?;

        // Build children first
        let children: Vec<TreeItem<'static, String>> = agent
            .children
            .iter()
            .filter_map(|child_id| self.build_tree_item_recursive(child_id, animation_frame))
            .collect();

        let display_name = agent.display_name(animation_frame);

        if children.is_empty() {
            Some(TreeItem::new_leaf(session_id.to_string(), display_name))
        } else {
            TreeItem::new(session_id.to_string(), display_name, children).ok()
        }
    }

    /// Selects an agent by `session_id`.
    pub fn select(&mut self, session_id: &str) {
        // Build the path from root to this session
        let path = self.build_path_to_session(session_id);
        self.tree_state.select(path);
    }

    /// Builds the identifier path from root to a given session.
    fn build_path_to_session(&self, session_id: &str) -> Vec<String> {
        let mut path = Vec::new();
        let mut current = Some(session_id);

        while let Some(id) = current {
            path.push(id.to_string());
            current = self
                .agents
                .get(id)
                .and_then(|a| a.parent_session_id.as_deref());
        }

        path.reverse();
        path
    }

    /// Returns the most recently started agent's `session_id`.
    #[must_use]
    pub fn most_recent_session_id(&self) -> Option<&str> {
        self.agents
            .values()
            .filter(|a| a.status == AgentStatus::Running)
            .max_by_key(|a| a.start_time)
            .map(|a| a.session_id.as_str())
            .or_else(|| {
                // Fall back to most recently started agent overall
                self.agents
                    .values()
                    .max_by_key(|a| a.start_time)
                    .map(|a| a.session_id.as_str())
            })
    }

    /// Returns true if any agents have a Running status.
    #[must_use]
    pub fn has_running_agents(&self) -> bool {
        self.agents
            .values()
            .any(|agent| agent.status == AgentStatus::Running)
    }

    /// Returns agent counts by status: `(succeeded, failed, in_progress)`.
    #[must_use]
    pub fn count_agents_by_status(&self) -> (usize, usize, usize) {
        let mut succeeded = 0;
        let mut failed = 0;
        let mut in_progress = 0;

        for agent in self.agents.values() {
            match agent.status {
                AgentStatus::Completed => succeeded += 1,
                AgentStatus::Failed => failed += 1,
                AgentStatus::Running => in_progress += 1,
            }
        }

        (succeeded, failed, in_progress)
    }

    /// Returns `session_id`s in the order they appear visually (flattened tree).
    ///
    /// Only includes items that are currently visible (parent nodes must be expanded).
    /// This is useful for mapping visual row indices to `session_id`s (e.g., for mouse clicks).
    #[must_use]
    pub fn visible_items(&self) -> Vec<String> {
        let mut result = Vec::new();
        for root_id in &self.roots {
            self.collect_visible_recursive(root_id, std::slice::from_ref(root_id), &mut result);
        }
        result
    }

    /// Recursively collects visible `session_id`s in depth-first order.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The current node to process
    /// * `current_path` - The path from root to current node (for checking opened state)
    /// * `result` - Accumulator for visible session IDs
    fn collect_visible_recursive(
        &self,
        session_id: &str,
        current_path: &[String],
        result: &mut Vec<String>,
    ) {
        result.push(session_id.to_string());

        // Check if this node is expanded in tree_state
        // A node's children are visible if the node's path is in the opened set
        if self.tree_state.opened().contains(&current_path.to_vec()) {
            if let Some(agent) = self.agents.get(session_id) {
                for child_id in &agent.children {
                    // Build the path to this child
                    let mut child_path = current_path.to_owned();
                    child_path.push(child_id.clone());
                    self.collect_visible_recursive(child_id, &child_path, result);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_tree_state_handle_agent_started() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Main task".to_string(),
        );

        assert_eq!(tree.agent_count(), 1);
        let agent = tree.get_agent("session-1").unwrap();
        assert_eq!(agent.depth, 0);
        assert_eq!(agent.status, AgentStatus::Running);
    }

    #[test]
    fn test_agent_tree_state_parent_child_relationship() {
        let mut tree = AgentTreeState::new();

        // Add root agent
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "parent-1".to_string(),
            0,
            "Parent task".to_string(),
        );

        // Add child agent
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "child-1".to_string(),
            1,
            "Child task".to_string(),
        );

        assert_eq!(tree.agent_count(), 2);

        let parent = tree.get_agent("parent-1").unwrap();
        assert!(parent.children.contains(&"child-1".to_string()));

        let child = tree.get_agent("child-1").unwrap();
        assert_eq!(child.parent_session_id, Some("parent-1".to_string()));
    }

    #[test]
    fn test_agent_tree_state_handle_complete() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Planner,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        tree.handle_agent_complete(Some("session-1"), true);

        let agent = tree.get_agent("session-1").unwrap();
        assert_eq!(agent.status, AgentStatus::Completed);
    }

    #[test]
    fn test_agent_tree_state_messages() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Streaming chunks without newlines are concatenated
        tree.handle_agent_message(Some("session-1"), "Hello");
        tree.handle_agent_message(Some("session-1"), "World");

        let messages = tree.get_messages("session-1").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "HelloWorld");

        // Newlines create separate messages
        tree.handle_agent_message(Some("session-1"), "\nNew line");
        let messages = tree.get_messages("session-1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], "HelloWorld");
        assert_eq!(messages[1], "New line");
    }

    #[test]
    fn test_agent_message_newline_in_middle() {
        // Test content with newline in the middle: "Line 1\nLine 2"
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        tree.handle_agent_message(Some("session-1"), "Line 1\nLine 2");
        let messages = tree.get_messages("session-1").unwrap();

        // Should split into two separate messages
        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(messages[0], "Line 1");
        assert_eq!(messages[1], "Line 2");
    }

    #[test]
    fn test_agent_message_trailing_newline() {
        // Test content with trailing newline: "Hello\n"
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        tree.handle_agent_message(Some("session-1"), "Hello\n");
        let messages = tree.get_messages("session-1").unwrap();

        // Should produce: "Hello", "" (empty for the trailing newline)
        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(messages[0], "Hello");
        assert_eq!(messages[1], ""); // Empty string for the line break
    }

    #[test]
    fn test_agent_message_just_newline() {
        // Test just a newline: "\n"
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        tree.handle_agent_message(Some("session-1"), "First");
        tree.handle_agent_message(Some("session-1"), "\n");
        tree.handle_agent_message(Some("session-1"), "Second");
        let messages = tree.get_messages("session-1").unwrap();

        // When "\n" arrives, it creates an empty message slot that "Second" fills
        // So we get ["First", "Second"] - this is intentional streaming behavior
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], "First");
        assert_eq!(messages[1], "Second");
    }

    #[test]
    fn test_full_flow_content_with_newlines() {
        // Test the full flow: content "Line 1\nLine 2\n" arriving in one chunk
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Simulate content as it would arrive from the LLM
        tree.handle_agent_message(Some("session-1"), "Line 1\nLine 2\n");
        let messages = tree.get_messages("session-1").unwrap();

        // Should split into: "Line 1", "Line 2", "" (empty for trailing newline)
        assert_eq!(messages.len(), 3, "Expected 3 messages, got {:?}", messages);
        assert_eq!(messages[0], "Line 1");
        assert_eq!(messages[1], "Line 2");
        assert_eq!(messages[2], ""); // Trailing newline becomes empty message
    }

    #[test]
    fn test_agent_tree_state_build_tree_items() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Root task".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "child".to_string(),
            1,
            "Child task".to_string(),
        );

        let items = tree.build_tree_items(0);
        assert_eq!(items.len(), 1); // One root
        assert_eq!(items[0].children().len(), 1); // One child
    }

    #[test]
    fn test_nested_decomposition_hierarchy() {
        // Simulates the nested_decompose.toml scenario:
        // 1. Planner at depth 0 creates plan
        // 2. Orchestrator at depth 0 executes
        // 3. Decompose creates child scope, sub-agents at depth 1
        // 4. Sub-planner and sub-orchestrator at depth 1
        // 5. Implementers at depth 2 (under sub-orchestrator)

        let mut tree = AgentTreeState::new();

        // Main planner starts (depth 0)
        tree.handle_agent_started(
            AgentType::Planner,
            "planner-001".to_string(),
            0,
            "Main plan".to_string(),
        );

        // Planner completes
        tree.handle_agent_complete(Some("planner-001"), true);

        // Main orchestrator starts (depth 0)
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orchestrator-001".to_string(),
            0,
            "Execute main plan".to_string(),
        );

        // Decompose: SubtaskCreated event would be sent here
        // Then sub-planner starts at depth 1
        tree.handle_agent_started(
            AgentType::Planner,
            "planner-002".to_string(),
            1,
            "Sub plan for auth".to_string(),
        );

        // Sub-planner should be child of main orchestrator
        let sub_planner = tree.get_agent("planner-002").unwrap();
        assert_eq!(
            sub_planner.parent_session_id,
            Some("orchestrator-001".to_string()),
            "Sub-planner should be parented to main orchestrator"
        );

        // Sub-planner completes
        tree.handle_agent_complete(Some("planner-002"), true);

        // Sub-orchestrator starts at depth 1
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orchestrator-002".to_string(),
            1,
            "Execute sub plan".to_string(),
        );

        // Sub-orchestrator should also be child of main orchestrator
        let sub_orch = tree.get_agent("orchestrator-002").unwrap();
        assert_eq!(
            sub_orch.parent_session_id,
            Some("orchestrator-001".to_string()),
            "Sub-orchestrator should be parented to main orchestrator"
        );

        // Implementers at depth 2 (children of sub-orchestrator)
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "implementer-001".to_string(),
            2,
            "Implement login".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Implementer { index: 2 },
            "implementer-002".to_string(),
            2,
            "Implement register".to_string(),
        );

        // Implementers should be children of sub-orchestrator
        let impl1 = tree.get_agent("implementer-001").unwrap();
        assert_eq!(
            impl1.parent_session_id,
            Some("orchestrator-002".to_string()),
            "Implementer 1 should be parented to sub-orchestrator"
        );

        let impl2 = tree.get_agent("implementer-002").unwrap();
        assert_eq!(
            impl2.parent_session_id,
            Some("orchestrator-002".to_string()),
            "Implementer 2 should be parented to sub-orchestrator"
        );

        // Check tree structure
        let items = tree.build_tree_items(0);
        assert_eq!(
            items.len(),
            2,
            "Should have 2 roots (planner + orchestrator at depth 0)"
        );

        // Find the orchestrator root
        let orch_item = items
            .iter()
            .find(|i| i.identifier() == &"orchestrator-001".to_string());
        assert!(orch_item.is_some(), "Should find orchestrator-001 in tree");

        let orch_item = orch_item.unwrap();
        assert_eq!(
            orch_item.children().len(),
            2,
            "Main orchestrator should have 2 children (sub-planner + sub-orchestrator)"
        );

        // Find sub-orchestrator
        let sub_orch_item = orch_item
            .children()
            .iter()
            .find(|c| c.identifier() == &"orchestrator-002".to_string());
        assert!(sub_orch_item.is_some(), "Should find sub-orchestrator");

        let sub_orch_item = sub_orch_item.unwrap();
        assert_eq!(
            sub_orch_item.children().len(),
            2,
            "Sub-orchestrator should have 2 implementer children"
        );
    }

    #[test]
    fn test_multiple_implementers_same_depth() {
        // Test multiple implementers at the same depth getting proper parent
        let mut tree = AgentTreeState::new();

        // Orchestrator at depth 0
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch".to_string(),
            0,
            "Main task".to_string(),
        );

        // Multiple implementers at depth 1
        for i in 1..=5 {
            tree.handle_agent_started(
                AgentType::Implementer { index: i },
                format!("impl-{i:03}"),
                1,
                format!("Task {i}"),
            );
        }

        // All implementers should be children of orchestrator
        let orch = tree.get_agent("orch").unwrap();
        assert_eq!(
            orch.children.len(),
            5,
            "Orchestrator should have 5 children"
        );

        for i in 1..=5 {
            let impl_id = format!("impl-{i:03}");
            let impl_agent = tree.get_agent(&impl_id).unwrap();
            assert_eq!(
                impl_agent.parent_session_id,
                Some("orch".to_string()),
                "All implementers should be parented to orchestrator"
            );
        }

        // Check tree structure
        let items = tree.build_tree_items(0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].children().len(), 5);
    }

    #[test]
    fn test_deeply_nested_decomposition() {
        // Test 3+ levels of nesting
        let mut tree = AgentTreeState::new();

        // Level 0: Main orchestrator
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-0".to_string(),
            0,
            "Main".to_string(),
        );

        // Level 1: First decompose
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-1".to_string(),
            1,
            "Sub-1".to_string(),
        );

        let orch1 = tree.get_agent("orch-1").unwrap();
        assert_eq!(orch1.parent_session_id, Some("orch-0".to_string()));

        // Level 2: Second decompose
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-2".to_string(),
            2,
            "Sub-2".to_string(),
        );

        let orch2 = tree.get_agent("orch-2").unwrap();
        assert_eq!(orch2.parent_session_id, Some("orch-1".to_string()));

        // Level 3: Third decompose
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-3".to_string(),
            3,
            "Sub-3".to_string(),
        );

        let orch3 = tree.get_agent("orch-3").unwrap();
        assert_eq!(orch3.parent_session_id, Some("orch-2".to_string()));

        // Level 4: Implementer
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "impl-deep".to_string(),
            4,
            "Deep impl".to_string(),
        );

        let impl_deep = tree.get_agent("impl-deep").unwrap();
        assert_eq!(impl_deep.parent_session_id, Some("orch-3".to_string()));

        // Verify tree depth
        let items = tree.build_tree_items(0);
        assert_eq!(items.len(), 1);

        // Navigate down the tree
        let mut current_children = items[0].children();
        assert_eq!(current_children.len(), 1);

        current_children = current_children[0].children();
        assert_eq!(current_children.len(), 1);

        current_children = current_children[0].children();
        assert_eq!(current_children.len(), 1);

        // Level 3 orchestrator has implementer child
        current_children = current_children[0].children();
        assert_eq!(current_children.len(), 1);

        // Implementer is a leaf
        assert_eq!(current_children[0].children().len(), 0);
    }

    #[test]
    fn test_status_indicators_in_display_name() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "running".to_string(),
            0,
            "Task".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Planner,
            "completed".to_string(),
            0,
            "Task".to_string(),
        );
        tree.handle_agent_complete(Some("completed"), true);

        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "failed".to_string(),
            0,
            "Task".to_string(),
        );
        tree.handle_agent_complete(Some("failed"), false);

        let running = tree.get_agent("running").unwrap();
        // Test animation frames: frame 0 should show "~", frame 30 should show "-"
        assert!(
            running.display_name(0).contains('~'),
            "Running agent at frame 0 should have ~ indicator"
        );
        assert!(
            running.display_name(30).contains('-'),
            "Running agent at frame 30 should have - indicator"
        );
        // Frame 60 should wrap back to "~"
        assert!(
            running.display_name(60).contains('~'),
            "Running agent at frame 60 should have ~ indicator"
        );

        let completed = tree.get_agent("completed").unwrap();
        assert!(
            completed.display_name(0).contains('✓'),
            "Completed agent should have checkmark"
        );

        let failed = tree.get_agent("failed").unwrap();
        assert!(
            failed.display_name(0).contains('✗'),
            "Failed agent should have X mark"
        );
    }

    #[test]
    fn test_select_nested_agent() {
        let mut tree = AgentTreeState::new();

        // Create a nested structure
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Root".to_string(),
        );
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "child".to_string(),
            1,
            "Child".to_string(),
        );
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "grandchild".to_string(),
            2,
            "Grandchild".to_string(),
        );

        // Select the deeply nested agent
        tree.select("grandchild");

        // Verify selection path is correct
        let selected = tree.selected_session_id();
        assert_eq!(selected, Some("grandchild"));

        // The path should include all ancestors
        let path = tree.build_path_to_session("grandchild");
        assert_eq!(path, vec!["root", "child", "grandchild"]);
    }

    #[test]
    fn test_handle_subtask_created() {
        let mut tree = AgentTreeState::new();

        // Start orchestrator at depth 0
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-main".to_string(),
            0,
            "Main task".to_string(),
        );

        // SubtaskCreated event fires when decompose is called
        // This should not panic and should not change tree structure
        tree.handle_subtask_created(0, 1, "/logs/run/subtask-001");

        // Verify state is still valid
        assert_eq!(tree.agent_count(), 1);

        // Now sub-agents start at depth 1
        tree.handle_agent_started(
            AgentType::Planner,
            "sub-planner".to_string(),
            1,
            "Sub plan".to_string(),
        );

        // Sub-planner should be correctly parented
        let sub_planner = tree.get_agent("sub-planner").unwrap();
        assert_eq!(sub_planner.parent_session_id, Some("orch-main".to_string()));
    }

    #[test]
    fn test_full_decomposition_flow_with_subtask_created() {
        // This simulates the exact event sequence that occurs during decomposition
        let mut tree = AgentTreeState::new();

        // 1. Main planner starts
        tree.handle_agent_started(
            AgentType::Planner,
            "planner-main".to_string(),
            0,
            "Create plan".to_string(),
        );
        tree.handle_agent_complete(Some("planner-main"), true);

        // 2. Main orchestrator starts
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-main".to_string(),
            0,
            "Execute plan".to_string(),
        );

        // 3. Orchestrator calls decompose -> SubtaskCreated
        tree.handle_subtask_created(0, 1, "/logs/run/subtask-001");

        // 4. Sub-planner starts at depth 1
        tree.handle_agent_started(
            AgentType::Planner,
            "planner-sub".to_string(),
            1,
            "Create sub-plan".to_string(),
        );
        tree.handle_agent_complete(Some("planner-sub"), true);

        // 5. Sub-orchestrator starts at depth 1
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "orch-sub".to_string(),
            1,
            "Execute sub-plan".to_string(),
        );

        // 6. Sub-orchestrator spawns implementers at depth 2
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "impl-1".to_string(),
            2,
            "Implement task 1".to_string(),
        );
        tree.handle_agent_complete(Some("impl-1"), true);

        tree.handle_agent_started(
            AgentType::Implementer { index: 2 },
            "impl-2".to_string(),
            2,
            "Implement task 2".to_string(),
        );
        tree.handle_agent_complete(Some("impl-2"), true);

        // 7. Sub-orchestrator completes
        tree.handle_agent_complete(Some("orch-sub"), true);

        // 8. Main orchestrator continues with another task at depth 1
        tree.handle_agent_started(
            AgentType::Implementer { index: 3 },
            "impl-3".to_string(),
            1,
            "Implement simple task".to_string(),
        );
        tree.handle_agent_complete(Some("impl-3"), true);

        // 9. Main orchestrator completes
        tree.handle_agent_complete(Some("orch-main"), true);

        // Verify the final tree structure
        // 7 agents: planner-main, orch-main, planner-sub, orch-sub, impl-1, impl-2, impl-3
        assert_eq!(tree.agent_count(), 7);

        // Check main planner is a root
        let main_planner = tree.get_agent("planner-main").unwrap();
        assert!(main_planner.parent_session_id.is_none());
        assert_eq!(main_planner.status, AgentStatus::Completed);

        // Check main orchestrator is a root with children
        let main_orch = tree.get_agent("orch-main").unwrap();
        assert!(main_orch.parent_session_id.is_none());
        assert_eq!(main_orch.status, AgentStatus::Completed);
        assert_eq!(
            main_orch.children.len(),
            3,
            "Main orch should have: sub-planner, sub-orch, impl-3"
        );

        // Check sub-orchestrator has implementer children
        let sub_orch = tree.get_agent("orch-sub").unwrap();
        assert_eq!(sub_orch.parent_session_id, Some("orch-main".to_string()));
        assert_eq!(
            sub_orch.children.len(),
            2,
            "Sub-orch should have 2 implementers"
        );

        // Verify tree builds correctly
        let items = tree.build_tree_items(0);
        assert_eq!(
            items.len(),
            2,
            "Should have 2 roots: main planner and main orch"
        );

        // Find main orchestrator and verify its structure
        let main_orch_item = items
            .iter()
            .find(|i| i.identifier() == "orch-main")
            .unwrap();
        assert_eq!(main_orch_item.children().len(), 3);
    }

    #[test]
    fn test_count_agents_by_status() {
        let mut tree = AgentTreeState::new();

        // Initially empty
        let (succeeded, failed, in_progress) = tree.count_agents_by_status();
        assert_eq!(succeeded, 0);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 0);

        // Add a running agent
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "running-1".to_string(),
            0,
            "Task".to_string(),
        );
        let (succeeded, failed, in_progress) = tree.count_agents_by_status();
        assert_eq!(succeeded, 0);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 1);

        // Add another running agent
        tree.handle_agent_started(
            AgentType::Planner,
            "running-2".to_string(),
            0,
            "Task".to_string(),
        );
        let (succeeded, failed, in_progress) = tree.count_agents_by_status();
        assert_eq!(succeeded, 0);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 2);

        // Complete one successfully
        tree.handle_agent_complete(Some("running-1"), true);
        let (succeeded, failed, in_progress) = tree.count_agents_by_status();
        assert_eq!(succeeded, 1);
        assert_eq!(failed, 0);
        assert_eq!(in_progress, 1);

        // Fail one
        tree.handle_agent_complete(Some("running-2"), false);
        let (succeeded, failed, in_progress) = tree.count_agents_by_status();
        assert_eq!(succeeded, 1);
        assert_eq!(failed, 1);
        assert_eq!(in_progress, 0);

        // Add more agents with mixed statuses
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "impl-1".to_string(),
            0,
            "Task".to_string(),
        );
        tree.handle_agent_started(
            AgentType::Implementer { index: 2 },
            "impl-2".to_string(),
            0,
            "Task".to_string(),
        );
        tree.handle_agent_complete(Some("impl-1"), true);

        let (succeeded, failed, in_progress) = tree.count_agents_by_status();
        assert_eq!(succeeded, 2); // running-1, impl-1
        assert_eq!(failed, 1); // running-2
        assert_eq!(in_progress, 1); // impl-2
    }

    #[test]
    fn test_standalone_message_not_concatenated_with_agent_message() {
        // Regression test for: agent output lines incorrectly joined together
        // Example of the problem:
        // > Calling: Context engine: "Help popup..."Let me gather more information...
        // Should be:
        // > Calling: Context engine: "Help popup..."
        // Let me gather more information...
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Simulate a tool call followed by an agent message
        tree.handle_standalone_message(
            Some("session-1"),
            "> Calling: Context engine: \"Help popup implementation\"",
        );
        tree.handle_agent_message(
            Some("session-1"),
            "Let me gather more information about the model configuration.",
        );

        let messages = tree.get_messages("session-1").unwrap();

        // Should have 2 separate messages, not concatenated
        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(
            messages[0], "> Calling: Context engine: \"Help popup implementation\"",
            "First message should be the tool call"
        );
        assert_eq!(
            messages[1], "Let me gather more information about the model configuration.",
            "Second message should be the agent message"
        );
    }

    #[test]
    fn test_tool_result_not_concatenated_with_agent_message() {
        // Same issue but with tool result (✓ prefix)
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Tool result followed by agent message
        tree.handle_standalone_message(Some("session-1"), "✓ view completed");
        tree.handle_agent_message(Some("session-1"), "Now I'll make the changes.");

        let messages = tree.get_messages("session-1").unwrap();

        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(messages[0], "✓ view completed");
        assert_eq!(messages[1], "Now I'll make the changes.");
    }

    #[test]
    fn test_tool_error_not_concatenated_with_agent_message() {
        // Same issue but with tool error (✗ prefix)
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Tool error followed by agent message
        tree.handle_standalone_message(Some("session-1"), "✗ compile completed");
        tree.handle_agent_message(Some("session-1"), "Let me fix the error.");

        let messages = tree.get_messages("session-1").unwrap();

        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(messages[0], "✗ compile completed");
        assert_eq!(messages[1], "Let me fix the error.");
    }

    #[test]
    fn test_plus_prefix_not_concatenated_with_agent_message() {
        // Same issue but with + prefix
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Plus prefix followed by agent message
        tree.handle_standalone_message(Some("session-1"), "+ Added file.rs");
        tree.handle_agent_message(Some("session-1"), "The file has been created.");

        let messages = tree.get_messages("session-1").unwrap();

        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(messages[0], "+ Added file.rs");
        assert_eq!(messages[1], "The file has been created.");
    }

    #[test]
    fn test_normal_streaming_still_concatenates() {
        // Ensure normal streaming behavior is preserved
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Normal streaming content should still concatenate
        tree.handle_agent_message(Some("session-1"), "Hello ");
        tree.handle_agent_message(Some("session-1"), "world!");

        let messages = tree.get_messages("session-1").unwrap();

        assert_eq!(messages.len(), 1, "Expected 1 message, got {:?}", messages);
        assert_eq!(messages[0], "Hello world!");
    }

    #[test]
    fn test_multiple_tool_calls_followed_by_message() {
        // Test sequence: tool call, tool result, agent message
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        tree.handle_standalone_message(Some("session-1"), "> Calling: view");
        tree.handle_standalone_message(Some("session-1"), "✓ view completed");
        tree.handle_agent_message(Some("session-1"), "I see the file contains...");

        let messages = tree.get_messages("session-1").unwrap();

        assert_eq!(messages.len(), 3, "Expected 3 messages, got {:?}", messages);
        assert_eq!(messages[0], "> Calling: view");
        assert_eq!(messages[1], "✓ view completed");
        assert_eq!(messages[2], "I see the file contains...");
    }

    #[test]
    fn test_insert_missing_line_breaks_quote_followed_by_capital() {
        // The exact issue: tool call description followed by regular text in same chunk
        // e.g., 'Calling: Context engine: "Help popup"Let me gather...'
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Simulate content arriving as a single chunk without newline
        tree.handle_agent_message(
            Some("session-1"),
            "Calling: Context engine: \"Help popup implementation\"Let me gather more information.",
        );

        let messages = tree.get_messages("session-1").unwrap();

        // Should be split into 2 messages (at the LAST closing quote before capital letter)
        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(
            messages[0],
            "Calling: Context engine: \"Help popup implementation\""
        );
        assert_eq!(messages[1], "Let me gather more information.");
    }

    #[test]
    fn test_insert_missing_line_breaks_only_affects_calling_pattern() {
        // The heuristic only applies when "Calling:" is present
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Without "Calling:", multiple quote-followed-by-capital patterns are NOT split
        // (to avoid breaking normal quoted text in code/prose)
        tree.handle_agent_message(
            Some("session-1"),
            "First \"query\"Second \"query\"Third text.",
        );

        let messages = tree.get_messages("session-1").unwrap();

        // Should NOT be split - no "Calling:" pattern
        assert_eq!(messages.len(), 1, "Expected 1 message, got {:?}", messages);
        assert_eq!(messages[0], "First \"query\"Second \"query\"Third text.");
    }

    #[test]
    fn test_insert_missing_line_breaks_preserves_normal_text() {
        // Normal text with quotes should not be affected
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // Normal sentence with quotes - no "Calling:" so no splitting
        tree.handle_agent_message(
            Some("session-1"),
            "I said \"hello\" to him and he replied \"hi\" back.",
        );

        let messages = tree.get_messages("session-1").unwrap();

        // Should remain as single message
        assert_eq!(messages.len(), 1, "Expected 1 message, got {:?}", messages);
        assert_eq!(
            messages[0],
            "I said \"hello\" to him and he replied \"hi\" back."
        );
    }

    #[test]
    fn test_calling_prefix_is_standalone() {
        // Test that "Calling:" prefix is detected as standalone
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "session-1".to_string(),
            0,
            "Task".to_string(),
        );

        // First message is regular text
        tree.handle_agent_message(Some("session-1"), "Looking at the code.");

        // Then a "Calling:" message arrives
        tree.handle_agent_message(Some("session-1"), "Calling: codebase-retrieval");

        let messages = tree.get_messages("session-1").unwrap();

        // Should be 2 separate messages since "Calling:" is standalone
        assert_eq!(messages.len(), 2, "Expected 2 messages, got {:?}", messages);
        assert_eq!(messages[0], "Looking at the code.");
        assert_eq!(messages[1], "Calling: codebase-retrieval");
    }

    #[test]
    fn test_visible_items_empty_tree() {
        let tree = AgentTreeState::new();
        let visible = tree.visible_items();
        assert!(
            visible.is_empty(),
            "Empty tree should have no visible items"
        );
    }

    #[test]
    fn test_visible_items_single_root() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Task".to_string(),
        );

        let visible = tree.visible_items();
        assert_eq!(visible, vec!["root"]);
    }

    #[test]
    fn test_visible_items_multiple_roots() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root1".to_string(),
            0,
            "Task 1".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Planner,
            "root2".to_string(),
            0,
            "Task 2".to_string(),
        );

        let visible = tree.visible_items();
        assert_eq!(visible, vec!["root1", "root2"]);
    }

    #[test]
    fn test_visible_items_collapsed_parent() {
        // By default, nodes are collapsed, so children should NOT be visible
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Root".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "child".to_string(),
            1,
            "Child".to_string(),
        );

        // Parent is collapsed by default
        let visible = tree.visible_items();
        assert_eq!(
            visible,
            vec!["root"],
            "Collapsed parent should hide children"
        );
    }

    #[test]
    fn test_visible_items_expanded_parent() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Root".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "child".to_string(),
            1,
            "Child".to_string(),
        );

        // Expand the root node
        tree.tree_state.open(vec!["root".to_string()]);

        let visible = tree.visible_items();
        assert_eq!(
            visible,
            vec!["root", "child"],
            "Expanded parent should show children"
        );
    }

    #[test]
    fn test_visible_items_deeply_nested_partial_expansion() {
        let mut tree = AgentTreeState::new();

        // Create a 3-level hierarchy: root -> child -> grandchild
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Root".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "child".to_string(),
            1,
            "Child".to_string(),
        );

        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "grandchild".to_string(),
            2,
            "Grandchild".to_string(),
        );

        // Only expand root, not child
        tree.tree_state.open(vec!["root".to_string()]);

        let visible = tree.visible_items();
        assert_eq!(
            visible,
            vec!["root", "child"],
            "Only first level of children should be visible when child is collapsed"
        );

        // Now also expand child
        tree.tree_state
            .open(vec!["root".to_string(), "child".to_string()]);

        let visible = tree.visible_items();
        assert_eq!(
            visible,
            vec!["root", "child", "grandchild"],
            "Grandchild should be visible when both ancestors are expanded"
        );
    }

    #[test]
    fn test_visible_items_multiple_children_in_order() {
        let mut tree = AgentTreeState::new();

        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root".to_string(),
            0,
            "Root".to_string(),
        );

        // Add multiple children
        for i in 1..=3 {
            tree.handle_agent_started(
                AgentType::Implementer { index: i },
                format!("child-{i}"),
                1,
                format!("Child {i}"),
            );
        }

        // Expand root
        tree.tree_state.open(vec!["root".to_string()]);

        let visible = tree.visible_items();
        assert_eq!(
            visible,
            vec!["root", "child-1", "child-2", "child-3"],
            "Children should appear in insertion order"
        );
    }

    #[test]
    fn test_visible_items_complex_tree() {
        // Complex tree structure:
        // root1
        //   child1a
        //     grandchild1
        //   child1b
        // root2
        //   child2a
        let mut tree = AgentTreeState::new();

        // Build the tree
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root1".to_string(),
            0,
            "Root 1".to_string(),
        );
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "child1a".to_string(),
            1,
            "Child 1a".to_string(),
        );
        tree.handle_agent_started(
            AgentType::Implementer { index: 1 },
            "grandchild1".to_string(),
            2,
            "Grandchild 1".to_string(),
        );

        // Start a new branch at depth 1 (sibling of child1a)
        // We need to reset depth_to_session for proper parenting
        // This simulates child1b starting after grandchild1 completes
        tree.handle_agent_complete(Some("grandchild1"), true);
        tree.handle_agent_complete(Some("child1a"), true);

        // Now we need to manually set up child1b under root1
        // In real usage, the depth_to_session mapping would be set correctly
        // For this test, we'll add child1b manually
        let mut child1b = AgentNode::new(
            AgentType::Implementer { index: 2 },
            "child1b".to_string(),
            1,
            "Child 1b".to_string(),
        );
        child1b.parent_session_id = Some("root1".to_string());
        if let Some(root1) = tree.agents.get_mut("root1") {
            root1.children.push("child1b".to_string());
        }
        tree.agents.insert("child1b".to_string(), child1b);
        tree.messages.insert("child1b".to_string(), Vec::new());

        // Add root2 with a child
        tree.handle_agent_started(
            AgentType::Orchestrator,
            "root2".to_string(),
            0,
            "Root 2".to_string(),
        );
        tree.handle_agent_started(
            AgentType::Implementer { index: 3 },
            "child2a".to_string(),
            1,
            "Child 2a".to_string(),
        );

        // Expand everything
        tree.tree_state.open(vec!["root1".to_string()]);
        tree.tree_state
            .open(vec!["root1".to_string(), "child1a".to_string()]);
        tree.tree_state.open(vec!["root2".to_string()]);

        let visible = tree.visible_items();
        assert_eq!(
            visible,
            vec![
                "root1",
                "child1a",
                "grandchild1",
                "child1b",
                "root2",
                "child2a"
            ],
            "Complex tree should be traversed in depth-first order"
        );
    }
}
