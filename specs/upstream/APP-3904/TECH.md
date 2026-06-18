# TECH.md — APP-3904: Don't override pane/tab representation for expanded edit tool call

## Problem

When an agent edit tool call is expanded into a pane (via `ExpandEditToPane`), the vertical tabs sidebar changes its representation for that tab from the original pane (e.g. "Create Korean Poem File" with agent icon and metadata) to a generic "Requested Edit" code-diff pane. The entire sidebar row — title, icon, subtitle, badge — changes to reflect the replacement `CodeDiffPane` instead of the original agent conversation pane.

The sidebar should continue showing the original pane's full representation while a temporary replacement is active.

## Relevant code

- `app/src/pane_group/pane/code_diff_pane.rs:22-45` — `CodeDiffPane::from_view` hardcodes `PaneConfiguration` title to "Requested Edit"
- `app/src/workspace/view.rs:6428-6460` — `open_code_diff` creates the `CodeDiffPane` and calls `replace_pane` with `is_temporary: true`
- `app/src/pane_group/tree.rs:55-112` — `HiddenPane` / `HiddenPaneReason::TemporaryReplacement` tracks the original→replacement mapping
- `app/src/pane_group/tree.rs:368-372` — `PaneData::is_temporary_replacement` checks if a pane is a replacement
- `app/src/pane_group/mod.rs:4456-4502` — `PaneGroup::replace_pane` orchestrates temporary replacement; original stays in `pane_contents`
- `app/src/workspace/view/vertical_tabs.rs:2065-2136` — `PaneProps::new` resolves display properties (typed, title, subtitle, icon, badge) from the pane's configuration and type
- `app/src/workspace/view/vertical_tabs.rs:2348-2391` — `PaneGroup::resolve_pane_type` maps `PaneId` → `TypedPane` which drives icon/badge/kind

## Current state

The `ExpandEditToPane` feature flag controls how code diff views are opened:
- **Enabled**: The focused pane is temporarily replaced with a `CodeDiffPane`. The original pane is hidden via `HiddenPaneReason::TemporaryReplacement(replacement_id)` and kept in `pane_contents` for later restoration.
- **Disabled**: The diff opens in a new tab.

When the sidebar renders tab rows, `PaneProps::new` resolves all display properties from the visible pane. For a temporary replacement, the visible pane is the `CodeDiffPane`, so the sidebar shows:
- Icon: `WarpIcon::Diff` (instead of the original terminal/agent icon)
- Title: "Requested Edit" (hardcoded in `CodeDiffPane::from_view`)
- Type: `TypedPane::CodeDiff` (loses all terminal-specific metadata like conversation title, working directory, git branch)
- Badge/subtitle: empty

The pane header ("Requested Edit" with Refine/Done/Accept buttons) shown *inside* the pane content area is correct and is rendered by `CodeDiffView::render_header_content` — that is not affected by this change.

## Proposed changes

### 1. Add `original_pane_for_replacement` lookup to `PaneData`

In `app/src/pane_group/tree.rs`, add a method that returns the original hidden pane's ID given a replacement pane ID:

```rust
pub fn original_pane_for_replacement(&self, replacement_pane_id: PaneId) -> Option<PaneId>
```

This scans `hidden_panes` for a `TemporaryReplacement` entry whose associated replacement ID matches. It follows the same pattern as the existing `is_temporary_replacement` method.

### 2. Expose through `PaneGroup`

In `app/src/pane_group/mod.rs`, add a thin delegation method:

```rust
pub fn original_pane_for_replacement(&self, replacement_pane_id: PaneId) -> Option<PaneId>
```

### 3. Update `PaneProps::new` to use original pane for display

In `app/src/workspace/view/vertical_tabs.rs`, modify `PaneProps::new` so that when the requested `pane_id` is a temporary replacement, it resolves the `PaneConfiguration` and `TypedPane` from the original hidden pane:

```
let display_pane_id = pane_group
    .original_pane_for_replacement(pane_id)
    .unwrap_or(pane_id);
let display_pane = pane_group.pane_by_id(display_pane_id)?;
let pane_configuration = display_pane.pane_configuration();
let typed = pane_group.resolve_pane_type(display_pane_id, app);
```

This makes the sidebar row render the original pane's icon, title, subtitle, badge, and all terminal-specific metadata (conversation title, working directory, git branch, status indicators).

Fields that should still use the replacement `pane_id`:
- `pane_id` — click/focus targets the visible replacement pane
- `is_focused` — the replacement pane is what actually holds focus
- `is_being_dragged` — drag state belongs to the visible pane

## End-to-end flow

1. User is in an agent conversation (terminal pane, sidebar shows "Create Korean Poem File" with agent icon)
2. Agent produces an edit tool call; user expands it
3. `open_code_diff` creates a `CodeDiffPane` and calls `replace_pane(focused_pane_id, new_pane, true)`, which:
   - Adds the original terminal pane to `hidden_panes` as `TemporaryReplacement(replacement_id)`
   - Swaps the tree node to the `CodeDiffPane`
4. Sidebar re-renders. `PaneProps::new` receives the replacement `pane_id`:
   - Looks up `original_pane_for_replacement(pane_id)` → finds the hidden terminal pane
   - Resolves `typed`, `pane_configuration`, title, icon, etc. from the original terminal pane
   - Sidebar row shows "Create Korean Poem File" with agent icon and metadata (unchanged)
5. User accepts/rejects the edit → `close_temporary_replacement_pane` reverts to the original pane
6. Sidebar naturally shows the original pane again (no special handling needed)

## Risks and mitigations

- **Original pane removed prematurely**: For temporary replacements, `replace_pane` explicitly skips removing the original from `pane_contents`. The original pane is guaranteed to exist for lookup. No new risk.
- **Multiple temporary replacements**: If multiple diffs are expanded in sequence on the same pane, the previous replacement is reverted first (via `close_temporary_replacement_pane`) before a new one is created. The lookup remains 1:1.
- **Non-`ExpandEditToPane` path**: When the flag is disabled, diffs open in a new tab (not a replacement). `original_pane_for_replacement` returns `None`, and `PaneProps::new` falls through to the existing behavior. No regression.
- **Detail sidecar**: The detail sidecar (hover popup) also resolves from `PaneProps`. Using the original pane's type means the sidecar will show terminal-specific detail (working directory, git branch, etc.) instead of code-diff detail. This is the correct behavior since the tab still conceptually represents the agent conversation.

## Testing and validation

- Manual: expand an agent edit tool call with `ExpandEditToPane` enabled. Verify the sidebar row keeps the original pane's icon, title, subtitle, and any badges. Verify clicking the sidebar row still focuses the diff pane. Verify accept/reject restores the original pane normally.
- Manual: repeat with `ExpandEditToPane` disabled. Verify no regression — diff opens in a new tab with "Requested Edit" title as before.
- Unit test: Add a test in `tree_tests.rs` for `original_pane_for_replacement` — verify it returns `Some(original_id)` after a temporary replacement and `None` otherwise.

## Follow-ups

- Consider whether `CodeDiffPane` still needs a hardcoded "Requested Edit" title at all, since the pane header text comes from `CodeDiffView::render_header_content` independently. The `PaneConfiguration` title is only relevant when the pane is shown in a new tab (non-replacement path). Leaving it as-is is safe.
