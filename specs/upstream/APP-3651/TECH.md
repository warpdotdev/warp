# APP-3651: Tech Spec — Vertical Tabs Pane Row Layout Iteration

## Problem

The vertical tabs panel pane rows need restructuring to improve information hierarchy (see `specs/APP-3651/PRODUCT.md`). The primary rendering functions for terminal and non-terminal pane rows, the tab group header collapse button, and text clipping direction all need to change within the same file.

## Relevant code

- `app/src/workspace/view/vertical_tabs.rs` — all pane row rendering; `VerticalTabsPanelState`, `PaneProps`, `TypedPane`, `render_pane_row`, `render_terminal_row_content`, `render_terminal_primary_line`, `render_terminal_secondary_line`, `render_group_header`, `render_kind_badge`
- `app/src/terminal/view/tab_metadata.rs` — `terminal_title_from_shell()`, `display_working_directory()`, `current_git_branch()`
- `app/src/terminal/view/pane_impl.rs:962-973` — `selected_conversation_status()`, `selected_conversation_display_title()`, `is_ambient_agent_session()`
- `app/src/terminal/model/blocks.rs:1708` — `BlockList::blocks()` returns `&Vec<Block>`
- `app/src/terminal/model/block.rs:2143` — `Block::command_to_string()`, `Block::finished()`, `BlockState`
- `app/src/code/view.rs:222-231` — `CodeView::tab_group` (private), `CodeView::set_title` (sets PaneConfiguration title/secondary)
- `app/src/code/icon.rs:11` — `icon_from_file_path(path, appearance) -> Option<Box<dyn Element>>`
- `app/src/pane_group/pane/code_pane.rs:53` — `CodePane::file_view()` returns `ViewHandle<CodeView>`
- `app/src/workspace/action.rs:99,227-230` — `WorkspaceAction::CloseTab`, `ToggleVerticalTabsGroupCollapsed`, `ToggleAllVerticalTabsGroupsCollapsed`
- `ui/src/text_layout.rs:455-460` — `ClipConfig::start()` already exists

## Current state

### Terminal pane rows
`render_terminal_row_content` builds three lines:
1. **Primary** (`render_terminal_primary_line`): working directory + git branch, main text color
2. **Secondary** (`render_terminal_secondary_line`): conversation title/status, or terminal title if it differs from working directory; sub text color. Returns `None` if terminal title matches working directory.
3. **Tertiary** (`render_terminal_tertiary_line`): kind badge (Terminal/Oz icon + label) + right badges (diff stats, PR)

### Non-terminal pane rows
`render_pane_row` builds the content in the `else` branch (line 739-774):
1. Title row (main text, `ClipConfig::end()`)
2. Optional subtitle row (sub text)
3. Meta row: `render_kind_badge(icon, kind_label)` on left, optional `render_row_badge(badge)` on right

### Tab group headers
`render_group_header` renders a collapse chevron (`ChevronDown`/`ChevronRight`) that dispatches `WorkspaceAction::ToggleVerticalTabsGroupCollapsed`. The `is_collapsed` state in `VerticalTabsPanelState::collapsed_tab_groups` controls whether pane rows are rendered.

### ClipConfig
All text currently uses `ClipConfig::end()`. `ClipConfig::start()` exists and fades from the leading edge.

### Last completed command
`TerminalModel` → `BlockList::blocks()` → `Vec<Block>`. Each `Block` has `command_to_string() -> String` and `state` (public via `finished()`). There is no existing accessor on `TerminalView` for the last completed command. The model is behind `self.model.lock()` on `TerminalView`.

### Code pane tab count
`CodeView::tab_group` is a private `Vec<TabData>`. No public accessor for `tab_group.len()` exists. `CodeView::set_title` already sets secondary title to `(+N)` format when `tab_group.len() > 1`.

## Proposed changes

### 1. Add `last_completed_command_text` to `TerminalView`

`app/src/terminal/view/tab_metadata.rs` — add a new method:

```rust
pub fn last_completed_command_text(&self) -> Option<String> {
    let model = self.model.lock();
    model.block_list().blocks().iter().rev().find_map(|block| {
        if block.finished()
            && !block.is_background()
            && !block.is_static()
        {
            let cmd = block.command_to_string();
            if cmd.trim().is_empty() { None } else { Some(cmd) }
        } else {
            None
        }
    })
}
```

This iterates backwards through blocks, finding the last finished non-background, non-static block with a non-empty command. The lock scope is contained to this method.

### 2. Add `tab_count` to `CodeView`

`app/src/code/view.rs` — add a one-line public method on `CodeView`:

```rust
pub fn tab_count(&self) -> usize {
    self.tab_group.len()
}
```

### 3. Restructure `render_terminal_row_content`

`app/src/workspace/view/vertical_tabs.rs` — rewrite `render_terminal_row_content` (line 969):

The new structure:
1. **Primary line**: Determine content using the precedence rules from the product spec:
   - If `conversation_display_title` is `Some`: render conversation status + title (reuse existing status element rendering)
   - Else if `terminal_title.trim() != working_directory.trim()`: render terminal title in monospace font
   - Else if `last_completed_command_text()` returns `Some`: render command in monospace font
   - Else: render "New session" in UI font

   All variants use main text color.

2. **Secondary line**: Always render the old primary line content (working directory + git branch) but in sub text color and using `ClipConfig::start()` for the working directory.

3. **Tertiary line**: Unchanged (`render_terminal_tertiary_line`).

The existing `render_terminal_primary_line` and `render_terminal_secondary_line` functions are replaced/rewritten to match the new line assignments. `render_terminal_tertiary_line` stays as-is.

### 4. Restructure non-terminal pane rows

`app/src/workspace/view/vertical_tabs.rs` — rewrite the `else` branch in `render_pane_row` (line 739):

New structure:
1. **Primary row**: `Flex::row` with kind icon (12px) + title text. For code panes, resolve the icon via `crate::code::icon_from_file_path` on the title string (which is the file path), falling back to `WarpIcon::Code2.to_warpui_icon(sub_text_color)`. For other types, use `TypedPane::icon().to_warpui_icon(sub_text_color)`.

2. **Secondary row** (if non-empty subtitle): subtitle in sub text color.

Remove the meta row (`Flex::row` with `render_kind_badge` + `render_row_badge`). The `render_kind_badge` and `render_row_badge` functions are no longer called from non-terminal pane rows (they are still used by terminal pane tertiary lines).

### 5. Code pane multi-tab subtitle

`app/src/workspace/view/vertical_tabs.rs` — in `TypedPane::Code` handling within the non-terminal branch:

Add a method or inline logic on `TypedPane` to expose the code pane tab count:

```rust
fn code_tab_count(&self, app: &AppContext) -> Option<usize> {
    match self {
        TypedPane::Code(code_pane) => {
            let count = code_pane.file_view(app).as_ref(app).tab_count();
            (count > 1).then_some(count)
        }
        _ => None,
    }
}
```

When rendering the subtitle for code panes, if `code_tab_count` returns `Some(count)`, override the subtitle with `format!("and {} more", count - 1)` regardless of what `PaneConfiguration.title_secondary()` contains.

### 6. Path clipping changes

In the new secondary line for terminal panes (`render_terminal_secondary_line`), change the working directory `Text` from `ClipConfig::end()` to `ClipConfig::start()`. Git branch text remains `ClipConfig::end()`.

In the non-terminal primary row, change the title `Text` from `ClipConfig::end()` to `ClipConfig::start()` when the pane type is `Code` (file paths). For non-path titles (Notebook, Settings, etc.), keep `ClipConfig::end()`.

### 7. Replace collapse chevron with close button

`app/src/workspace/view/vertical_tabs.rs` — in `render_group_header` (line 564):

Replace the collapse button construction:
- Change icon from `chevron_icon` (`ChevronDown`/`ChevronRight`) to a constant `WarpIcon::X` (or `UiIcon::X`).
- Change the click handler from dispatching `WorkspaceAction::ToggleVerticalTabsGroupCollapsed` to `WorkspaceAction::CloseTab(tab_index)`.
- Remove the `is_collapsed` parameter from `GroupHeaderProps`.

### 8. Remove collapse state

`app/src/workspace/view/vertical_tabs.rs`:
- Remove `collapsed_tab_groups: HashSet<EntityId>` from `VerticalTabsPanelState`.
- Remove `toggle_group_collapsed`, `toggle_all_groups_collapsed`, `is_group_collapsed` methods.
- Remove the `collapse: MouseStateHandle` from `PaneGroupStateHandles` — rename the field to `close` for clarity.
- Remove the `is_collapsed` check in `render_tab_group` that conditionally skips rendering pane rows.

`app/src/workspace/action.rs`:
- Remove `ToggleVerticalTabsGroupCollapsed` and `ToggleAllVerticalTabsGroupsCollapsed` variants from `WorkspaceAction`.

`app/src/workspace/view.rs`:
- Remove the match arms for `ToggleVerticalTabsGroupCollapsed` and `ToggleAllVerticalTabsGroupsCollapsed` (lines 16759-16764).
- Remove `toggle_vertical_tabs_group_collapsed` and `toggle_all_vertical_tabs_groups_collapsed` methods (lines 6125-6139).

## End-to-end flow

### Terminal pane row rendering
1. `render_pane_row` is called for each visible pane in a tab group.
2. For terminal panes, `render_terminal_row_content` is called with the `TerminalView` reference.
3. It calls `terminal_title_from_shell()`, `display_working_directory()`, `selected_conversation_display_title()`, and the new `last_completed_command_text()`.
4. The primary line function applies the precedence rules and returns the appropriate element.
5. The secondary line always renders working directory + git branch in sub text color with `ClipConfig::start()`.
6. The tertiary line renders unchanged.

### Close button
1. User clicks X on a tab group header.
2. The click handler dispatches `WorkspaceAction::CloseTab(tab_index)`.
3. The existing workspace close-tab logic handles teardown, undo grace period, etc.

## Risks and mitigations

**Lock contention for `last_completed_command_text`**: The method acquires `self.model.lock()` and iterates blocks. This runs on the render path. Mitigation: the iteration is backwards and short-circuits on the first match, so it's fast in practice. The lock is already acquired in `terminal_title_from_shell()` on the same render path, so this is consistent with existing patterns.

**Removing collapse state**: Any external callers of `ToggleVerticalTabsGroupCollapsed` or `ToggleAllVerticalTabsGroupsCollapsed` will fail to compile. Mitigation: grep confirms these are only dispatched from `vertical_tabs.rs` and handled in `view.rs` — no external callers.

**Code pane tab count**: `CodeView::tab_count()` requires reading through `CodePane::file_view(app).as_ref(app)`, which is the same pattern already used by `TypedPane::badge()`. No additional risk.

## Testing and validation

All changes are in rendering code with no persistence or protocol changes. Validation is primarily manual:

- Build and run with `cargo run`, enable vertical tabs, and verify each success criterion from the product spec.
- Verify terminal panes with: default shell (should show "New session"), after running a command (should show command text), running `vim` (should show "vim"), agent conversation (should show conversation title + status).
- Verify code panes with: single `.rs` file (Rust icon), single `.txt` file (Code2 icon), multiple tabs (language icon + "and X more").
- Verify path clipping by narrowing the panel.
- Verify close button closes the tab.
- Verify no compilation errors or clippy warnings from removed collapse state.

## Follow-ups

- The "compact" single-row mode shown in the Figma mock is deferred.
- Consider caching `last_completed_command_text` if profiling shows the block iteration is a hot path (unlikely given reverse iteration with early exit).
- The `render_kind_badge` and `render_row_badge` functions can be cleaned up if they become unused after future terminal row changes.
