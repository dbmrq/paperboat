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
use std::time::Instant;

use tui_tree_widget::{TreeItem, TreeState};

use crate::logging::AgentType;

// ============================================================================
// Agent Status
// ============================================================================

/// Status of an agent in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentStatus {
    /// Agent is currently running
    #[default]
    Running,
    /// Agent completed successfully
    Completed,
    /// Agent failed
    Failed,
}

// ============================================================================
// Agent Node (Tree Data)
// ============================================================================

/// Metadata about an agent in the tree.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Some fields are stored for future UI features
pub struct AgentNode {
    /// The session ID (unique identifier)
    pub session_id: String,
    /// Type of agent (Orchestrator, Planner, Implementer)
    pub agent_type: AgentType,
    /// Depth in the hierarchy (0 = root)
    pub depth: u32,
    /// The task this agent is working on
    pub task: String,
    /// When the agent started
    pub start_time: Instant,
    /// Current status
    pub status: AgentStatus,
    /// Children session IDs (for building tree)
    pub children: Vec<String>,
    /// Parent session ID (None for root)
    pub parent_session_id: Option<String>,
}

impl AgentNode {
    /// Creates a new agent node from an `AgentStarted` event.
    #[must_use]
    pub fn new(agent_type: AgentType, session_id: String, depth: u32, task: String) -> Self {
        Self {
            session_id,
            agent_type,
            depth,
            task,
            start_time: Instant::now(),
            status: AgentStatus::Running,
            children: Vec::new(),
            parent_session_id: None,
        }
    }

    /// Returns a display name for this agent.
    #[must_use]
    pub fn display_name(&self) -> String {
        let status_icon = match self.status {
            AgentStatus::Running => "~",
            AgentStatus::Completed => "✓",
            AgentStatus::Failed => "✗",
        };
        format!("{} {}", status_icon, self.agent_type.name())
    }
}

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
    pub fn handle_agent_message(&mut self, session_id: Option<&str>, content: &str) {
        if let Some(id) = session_id {
            if let Some(messages) = self.messages.get_mut(id) {
                // Concatenate streaming chunks into the last message
                // Only split on actual newlines in the content
                if messages.is_empty() {
                    messages.push(String::new());
                }

                // Append content to the last message, handling newlines
                for (i, part) in content.split('\n').enumerate() {
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
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Builds tree items for rendering with `tui-tree-widget`.
    ///
    /// Returns owned [`TreeItem`]s that can be used for rendering.
    #[must_use]
    pub fn build_tree_items(&self) -> Vec<TreeItem<'static, String>> {
        self.roots
            .iter()
            .filter_map(|root_id| self.build_tree_item_recursive(root_id))
            .collect()
    }

    /// Recursively builds a tree item and its children.
    fn build_tree_item_recursive(&self, session_id: &str) -> Option<TreeItem<'static, String>> {
        let agent = self.agents.get(session_id)?;

        // Build children first
        let children: Vec<TreeItem<'static, String>> = agent
            .children
            .iter()
            .filter_map(|child_id| self.build_tree_item_recursive(child_id))
            .collect();

        let display_name = agent.display_name();

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

    /// Returns agent counts by status: (succeeded, failed, in_progress).
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

        let items = tree.build_tree_items();
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
        let items = tree.build_tree_items();
        assert_eq!(items.len(), 2, "Should have 2 roots (planner + orchestrator at depth 0)");

        // Find the orchestrator root
        let orch_item = items.iter().find(|i| i.identifier() == &"orchestrator-001".to_string());
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
        assert_eq!(orch.children.len(), 5, "Orchestrator should have 5 children");

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
        let items = tree.build_tree_items();
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
        let items = tree.build_tree_items();
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
        assert!(
            running.display_name().contains('~'),
            "Running agent should have ~ indicator"
        );

        let completed = tree.get_agent("completed").unwrap();
        assert!(
            completed.display_name().contains('✓'),
            "Completed agent should have checkmark"
        );

        let failed = tree.get_agent("failed").unwrap();
        assert!(
            failed.display_name().contains('✗'),
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
        assert_eq!(
            sub_planner.parent_session_id,
            Some("orch-main".to_string())
        );
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
        assert_eq!(sub_orch.children.len(), 2, "Sub-orch should have 2 implementers");

        // Verify tree builds correctly
        let items = tree.build_tree_items();
        assert_eq!(items.len(), 2, "Should have 2 roots: main planner and main orch");

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
}
