# TUI Mouse Support & Selection Styling Plan

## Overview

This plan covers three related improvements to the TUI:
1. **Mouse click-to-select** for Agent Tree and Task List panels
2. **Unified selection styling** between Agent Tree and Task List
3. **Refined focus behavior** for the center panel

## Current State

### Mouse Support
- ✅ Mouse clicks switch panel **focus** (clicking a panel focuses it)
- ✅ Mouse scroll moves selection in Agent Tree and Task List
- ❌ Mouse clicks do NOT select individual items within panels

### Selection Styling
| Widget | Highlight Style | Symbol |
|--------|-----------------|--------|
| Agent Tree | Yellow foreground + Bold | `"> "` |
| Task List | Bold + **Reversed** (inverted colors) | `"> "` |

The styles are visually inconsistent.

### Center Panel Behavior
Currently in `TuiState::on_focus_changed()`:
- Focusing **Task List** → auto-selects first task, shows task detail in center
- Focusing **any other panel** → clears task selection, shows agent output

**Problem**: Clicking the Agent Output panel itself clears the task view, which is
counterintuitive. Users may want to scroll agent output while keeping a task selected.

---

## Implementation Plan

### Phase 1: Unified Selection Styling
**Estimated Time: 1-2 hours**

Unify the highlight styles so both panels look consistent.

#### Files to Modify
- `src/tui/widgets/agent_tree.rs`
- `src/tui/widgets/task_list.rs`

#### Design Decision
Use the **yellow foreground + bold** style (currently in Agent Tree) for both:
```rust
.highlight_style(Style::default().fg(Color::Yellow).bold())
.highlight_symbol("> ")
```

The reversed style in Task List can be jarring; the yellow highlight is more subtle
and consistent with the overall TUI aesthetic.

#### Changes
1. In `task_list.rs`, change highlight_style from:
   ```rust
   .highlight_style(
       Style::default()
           .add_modifier(Modifier::BOLD)
           .add_modifier(Modifier::REVERSED),
   )
   ```
   To:
   ```rust
   .highlight_style(Style::default().fg(Color::Yellow).bold())
   ```

2. Consider extracting shared highlight constants to a common module (optional).

---

### Phase 2: Refined Focus Behavior
**Estimated Time: 1-2 hours**

Change when the center panel switches from task detail back to agent output.

#### Current Behavior
```
Focus TaskList    → Show task detail (if task selected)
Focus anything    → Clear task selection, show agent output
```

#### New Behavior
```
Focus TaskList    → Show task detail (if task selected)
Focus AgentTree   → Clear task selection, show agent output
Focus AppLogs     → Clear task selection, show agent output  
Focus AgentOutput → KEEP task selection (no change to center panel)
```

#### Files to Modify
- `src/tui/state.rs` - `on_focus_changed()` method

#### Changes
```rust
pub fn on_focus_changed(&mut self, new_focus: FocusedPanel) {
    self.current_focus = new_focus;

    match new_focus {
        FocusedPanel::TaskList => {
            // Auto-select first task when focusing TaskList
            if !self.task_list_state.is_empty() 
                && self.task_list_state.selected_index.is_none() 
            {
                self.task_list_state.selected_index = Some(0);
            }
        }
        FocusedPanel::AgentTree | FocusedPanel::AppLogs => {
            // Clear task selection only when focusing these panels
            self.task_list_state.selected_index = None;
        }
        FocusedPanel::AgentOutput => {
            // Keep task selection - user may want to scroll output
            // while keeping task detail visible
        }
    }
}
```

---

### Phase 3: Mouse Click-to-Select for Task List
**Estimated Time: 2-3 hours**

Enable clicking on a task row to select it.

#### Approach
The Task List uses `ratatui::widgets::List`, which renders items sequentially.
Mapping click position to item index is straightforward:

```
clicked_index = (click_row - inner_area.y) + scroll_offset
```

#### Files to Modify
- `src/tui/events/mouse.rs`
- `src/tui/task_list_state.rs` (add `select_index` method)

#### Implementation Steps

1. **Add `select_index()` method to `TaskListState`**:
   ```rust
   pub fn select_index(&mut self, index: usize) {
       if index < self.task_order.len() {
           self.selected_index = Some(index);
       }
   }
   ```

2. **Add `handle_task_list_click()` in `mouse.rs`**:
   ```rust
   fn handle_task_list_click(
       state: &mut TuiState,
       row: u16,
       layout: &PanelLayout,
   ) {
       // Calculate inner area (excluding borders)
       let inner_y = layout.task_list.y + 1;
       let inner_height = layout.task_list.height.saturating_sub(2);
       
       if row >= inner_y && row < inner_y + inner_height {
           let visible_row = (row - inner_y) as usize;
           let clicked_index = visible_row + state.task_list_state.scroll_offset;
           state.task_list_state.select_index(clicked_index);
       }
   }
   ```

3. **Update `handle_mouse_click()` to call the new function** when clicking
   inside the task list panel.

---

### Phase 4: Mouse Click-to-Select for Agent Tree
**Estimated Time: 4-6 hours**

This is more complex because `tui-tree-widget` doesn't expose click-to-item mapping.

#### Challenge
The tree has:
- Expandable/collapsible nodes (some children hidden)
- Indentation based on depth
- Scroll offset
- Dynamic item count based on expand state

We need to know which items are **visually rendered** on which rows.

#### Approach: Track Flattened Visible Items

Add a method to `AgentTreeState` that returns visible `session_id`s in render order:

```rust
/// Returns session_ids in the order they appear visually (flattened tree).
/// Only includes items that are currently visible (parents expanded).
pub fn visible_items(&self) -> Vec<String> {
    let mut result = Vec::new();
    for root_id in &self.roots {
        self.collect_visible_recursive(root_id, &mut result);
    }
    result
}

fn collect_visible_recursive(&self, session_id: &str, result: &mut Vec<String>) {
    result.push(session_id.to_string());
    
    // Only include children if this node is expanded in tree_state
    if self.tree_state.opened().contains(&vec![session_id.to_string()]) {
        if let Some(agent) = self.agents.get(session_id) {
            for child_id in &agent.children {
                self.collect_visible_recursive(child_id, result);
            }
        }
    }
}
```

#### Files to Modify
- `src/tui/agent_tree_state.rs` - add `visible_items()` method
- `src/tui/events/mouse.rs` - add `handle_agent_tree_click()`

#### Implementation Steps

1. **Add `visible_items()` to `AgentTreeState`** (see above)

2. **Add `handle_agent_tree_click()` in `mouse.rs`**:
   ```rust
   fn handle_agent_tree_click(
       state: &mut TuiState,
       row: u16,
       layout: &PanelLayout,
   ) {
       let inner_y = layout.agent_tree.y + 1;
       let inner_height = layout.agent_tree.height.saturating_sub(2);
       
       if row >= inner_y && row < inner_y + inner_height {
           let visible_row = (row - inner_y) as usize;
           let visible_items = state.agent_tree_state.visible_items();
           
           // Account for scroll offset in tree_state
           let scroll_offset = state.agent_tree_state.tree_state.offset();
           let clicked_index = visible_row + scroll_offset;
           
           if let Some(session_id) = visible_items.get(clicked_index) {
               state.agent_tree_state.select(session_id);
               state.selected_agent_id = Some(session_id.clone());
               // Disable auto-follow on manual selection
               state.auto_follow_enabled = false;
           }
       }
   }
   ```

3. **Update `handle_mouse_click()`** to call the new function when clicking
   inside the agent tree panel.

#### Edge Cases to Handle
- Clicking on expand/collapse indicators (▶/▼) - should toggle, not just select
- Empty tree
- Scroll offset calculation

---

## Testing Plan

### Manual Testing
1. Click on various agents in the tree → should select and show their output
2. Click on various tasks → should select and show task detail
3. Verify highlight styles match between panels
4. Click Agent Output while task is selected → task should remain selected
5. Click Agent Tree while task is selected → should switch to agent output
6. Test with collapsed tree nodes
7. Test with scrolled lists

### Unit Tests
- `AgentTreeState::visible_items()` returns correct order for various tree shapes
- `TaskListState::select_index()` bounds checking
- Focus change behavior preserves/clears selection correctly

---

## Implementation Order

1. **Phase 1** (Styling) - Quick win, low risk
2. **Phase 2** (Focus behavior) - Small change, improves UX
3. **Phase 3** (Task List click) - Straightforward implementation
4. **Phase 4** (Agent Tree click) - Most complex, do last

**Total Estimated Time: 8-13 hours**

---

## Future Considerations

- **Double-click to expand/collapse** tree nodes
- **Right-click context menu** for agents/tasks
- Consider `ratkit` library if more complex mouse interactions are needed
  (provides built-in `TreeView` with native mouse support)

