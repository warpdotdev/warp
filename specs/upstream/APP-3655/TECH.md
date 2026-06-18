# APP-3655: Vertical Tabs — Search Functionality + Control Bar UI Polish (Tech Spec)

## 1. Problem

The vertical tabs panel has a search input that is fully inert — it cannot be focused or typed into. This spec covers:

1. Making the search input functional: read query text, filter the rendered pane list and tab group headers in real time.
2. Updating tab cycling (`activate_next_tab` / `activate_prev_tab`) to skip tab groups that have no matching panes when a filter is active.
3. Visual polish of the control bar: remove the search input container background/border, and align the control bar's horizontal padding with the rest of the panel content.

## 2. Relevant Code

- `app/src/workspace/view/vertical_tabs.rs` — all vertical tabs rendering; the files that change most
  - `render_control_bar` (line 185) — search bar UI
  - `render_groups` (line 380) — tab group iteration, filter logic goes here
  - `render_tab_group` (line 408) — renders one group header + its pane rows
  - `PaneProps::new` (line ~882) — assembles the searchable text fields per pane
  - `TypedPane::kind_label`, `TypedPane::badge` (lines ~820–860)
- `app/src/workspace/view.rs`
  - `Workspace` struct (line ~760) — add `vertical_tabs_search_query: String`
  - `vertical_tabs_search_input` constructor (line 890) — add `Edited` subscription
  - `activate_next_tab` / `activate_prev_tab` (lines 7481–7497) — filter-aware cycling
- `app/src/editor/view/mod.rs`
  - `EditorView::buffer_text(&self, ctx: &AppContext) -> String` (line 3675) — the public API to read query text from an `EditorView`

## 3. Current State

**Search input**: The `EditorView` for the search bar is created and rendered but is fully passive. No subscription listens to text changes, and no query string is read or used anywhere.

**Control bar layout** (`render_control_bar`, line 232–242):
- The outer `Container` uses `Padding::uniform(CONTROL_BAR_VERTICAL_PADDING)` (4px all sides) — no left/right padding to match the content area.
- The search bar `Container` has `with_background(internal_colors::fg_overlay_1(theme))` and `with_corner_radius(...)`, giving it a visible box appearance.

**Tab cycling** (`activate_next_tab` / `activate_prev_tab`, lines 7481–7497):
- Cycles through `self.tabs` (all tab groups) unconditionally by index with wrap-around.

## 4. Proposed Changes

### 4.1 Store the search query on `VerticalTabsPanelState`

Add a `String` field to `VerticalTabsPanelState` (vertical_tabs.rs):

```rust
pub(super) struct VerticalTabsPanelState {
    // ... existing fields ...
    search_query: String,
}
```

Initialize it as `String::new()`. Keeping the query here (rather than on `Workspace`) co-locates it with all other panel-specific state, and `render_groups` already receives `state: &VerticalTabsPanelState` directly — no additional threading needed to read it during render. `activate_next_tab`/`activate_prev_tab` access it via `self.vertical_tabs_panel.search_query` (the field on `Workspace` is `vertical_tabs_panel: VerticalTabsPanelState`, view.rs line 862).

### 4.2 Subscribe to `EditorEvent::Edited` in `vertical_tabs_search_input`

In `Workspace::vertical_tabs_search_input` (view.rs line 890), add a second subscription alongside the existing `Escape` handler:

```rust
ctx.subscribe_to_view(&editor, |me, editor_view, event, ctx| {
    if matches!(event, EditorEvent::Edited(_)) {
        me.vertical_tabs_panel.search_query = editor_view.as_ref(ctx).buffer_text(ctx);
        ctx.notify();
    }
});
```

When `Escape` clears the editor, also clear the query. Merge this into the existing `Escape` handler rather than adding a separate subscription:

```rust
ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
    if matches!(event, EditorEvent::Escape) {
        me.vertical_tabs_panel.search_query.clear();
        me.focus_active_tab(ctx);
    }
});
```

### 4.3 Apply the filter in `render_groups`

`render_groups` (line 380) already receives `state: &VerticalTabsPanelState`. Read the query from `state.search_query` directly — no signature change needed.

If the query is non-empty:

1. For each `tab` in `workspace.tabs`, compute the list of `PaneId`s from `pane_group.visible_pane_ids()` that satisfy `pane_matches_query`.
2. If no panes match → skip the entire tab group (no call to `render_tab_group`).
3. If ≥1 panes match → call `render_tab_group`, passing the filtered `Vec<PaneId>` so only matching rows are rendered.
4. If no groups survive and the query is non-empty → render the empty-state message: `"No tabs match your search."` styled the same as the existing `"No tabs open"` message.

If the query is empty → preserve the existing behavior (pass all pane IDs, no change).

**Add a helper function:**

```rust
fn pane_matches_query(props: &PaneProps<'_>, query_lower: &str) -> bool {
    props.title.to_lowercase().contains(query_lower)
        || props.subtitle.to_lowercase().contains(query_lower)
        || props.kind_label.to_lowercase().contains(query_lower)
        || props.typed.badge().map_or(false, |b| b.to_lowercase().contains(query_lower))
}
```

The caller lowercases the query once before the loop: `let query_lower = state.search_query.to_lowercase();`. This avoids re-allocating the lowercased query string for every pane check. Each pane field is still lowercased per check; that is fine given that pane titles are short strings and tab counts are in the tens to low hundreds. No caching is needed at this scale.

`PaneProps` already aggregates `title`, `subtitle`, `kind_label`, and `badge` — no need to reach into lower-level types.

### 4.4 Update `render_tab_group` to accept a filtered pane list

Change the signature to accept an optional filtered pane ID list:

```rust
fn render_tab_group(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    tab_index: usize,
    tab: &TabData,
    filtered_pane_ids: Option<&[PaneId]>,  // None = render all
    app: &AppContext,
) -> Box<dyn Element>
```

Inside the function, where `visible_pane_ids` is currently read from `pane_group.visible_pane_ids()`, replace with the filtered list when `filtered_pane_ids` is `Some`. The group header always renders when this function is called (skipping was already handled in `render_groups`).

### 4.5 Filter-aware tab cycling

In `activate_next_tab` and `activate_prev_tab` (view.rs lines 7481–7497), when `self.vertical_tabs_search_query` is non-empty, compute the set of tab indices that have at least one matching pane, then find the next/previous in that set (with wrap-around) relative to `self.active_tab_index`.

```rust
pub fn activate_next_tab(&mut self, ctx: &mut ViewContext<Self>) {
    if self.vertical_tabs_panel.search_query.is_empty() {
        // existing logic
        let index = if self.active_tab_index + 1 < self.tabs.len() {
            self.active_tab_index + 1
        } else {
            0
        };
        self.activate_tab(index, ctx);
    } else {
        let matching: Vec<usize> = self.matching_tab_indices(ctx);
        if let Some(next) = next_in_cycle(&matching, self.active_tab_index) {
            self.activate_tab(next, ctx);
        }
    }
}
```

Add a private helper:

```rust
fn matching_tab_indices(&self, ctx: &AppContext) -> Vec<usize> {
    // returns tab indices (in original order) where ≥1 pane matches the query
}
```

The `next_in_cycle` / `prev_in_cycle` helpers find the next/previous element in a sorted index list relative to a current value, wrapping around.

### 4.6 Control bar visual changes

In `render_control_bar` (line 209–227), change the `search_bar` container:

- Remove `.with_background(internal_colors::fg_overlay_1(theme))`.
- Remove `.with_corner_radius(...)`.
- Remove (or simplify) the fixed `with_padding(Padding::uniform(4.).with_left(8.).with_right(8.))` inner padding — replace with minimal padding that aligns the icon visually without a box.

In the outer `Container` (line 232–242), change its padding to add 12px left and right:

```rust
.with_padding(
    Padding::uniform(CONTROL_BAR_VERTICAL_PADDING)
        .with_left(GROUP_HORIZONTAL_PADDING)
        .with_right(GROUP_HORIZONTAL_PADDING),
)
```

Remove the `SEARCH_BAR_HEIGHT` constraint on the search bar's `ConstrainedBox` if it conflicts with the new unboxed layout, or keep it for consistent height — check against the Figma mock.

## 5. End-to-End Flow

1. User types "rust" into the search bar.
2. `EditorView` emits `EditorEvent::Edited(_)`.
3. The subscription in `vertical_tabs_search_input` fires: `workspace.vertical_tabs_search_query = "rust"`, then `ctx.notify()`.
4. The workspace re-renders. `render_groups` reads `"rust"` from `workspace.vertical_tabs_search_query`.
5. For each tab group, `visible_pane_ids()` is retrieved, each pane is tested via `pane_matches_query`, and the filtered ID list is passed to `render_tab_group`. Groups with no matches are skipped.
6. User presses next-tab shortcut. `activate_next_tab` reads the non-empty query, computes `matching_tab_indices`, and steps to the next matching tab index, skipping groups with no matches.
7. User presses Escape. The `Escape` subscription clears `vertical_tabs_search_query` and calls `focus_active_tab`. The workspace re-renders with the full unfiltered list.

## 6. Risks and Mitigations

- **Performance**: `pane_matches_query` runs on every render for each visible pane. For realistic Warp usage (tens to low hundreds of tabs), this is O(n) with cheap string operations and is not a concern. The query is lowercased once before the loop; pane fields are lowercased per check on short strings. No caching is needed unless profiling shows otherwise.
- **`PaneProps::new` returns `Option`**: The filter must handle the `None` case (pane no longer exists) gracefully — consistent with how the existing render loop handles it via `continue`.
- **Collapse state**: Collapsed tab groups still participate in filtering. A group whose header is collapsed but whose panes match should be shown with its header visible (collapsed state preserved). The group header is always rendered when `render_tab_group` is called; only the pane rows are hidden when collapsed. This is unchanged behavior.
- **Query not cleared on new tab**: Creating a new tab appends to `self.tabs`; the query remains. The new tab group will appear only if one of its panes matches the query (it likely won't until it has content), which is acceptable. No special handling needed.
- **Escape event double-clear**: Subscribing to `Escape` in two places (old handler + new query-clear) needs to be merged into one subscription to avoid double-firing. Merge both effects into the single `Escape` handler.

## 7. Testing and Validation

- **Manual**: Type a partial query that matches only a subset of panes — confirm only matching rows and their group headers are visible.
- **Manual**: Type a query that matches no panes — confirm the empty state message appears.
- **Manual**: Clear the query — confirm the full list is restored exactly.
- **Manual**: Press Escape — confirm the query clears, the full list restores, and focus returns to the active pane.
- **Manual**: With a filter active, use next/prev tab shortcuts — confirm navigation lands only on tab groups with matching panes.
- **Manual**: Collapse a tab group that has matching panes, then search — confirm the group header is visible and collapsed (not omitted).
- **Visual**: Screenshot the control bar; confirm no background or border on the search input, and left/right padding aligns with pane row text.
- **Regression**: Confirm next/prev tab cycling without a query is unchanged (all tabs cycle in order).

## 8. Follow-ups

- Highlighted matched text within pane rows (out of scope for this iteration).
- Keyboard navigation through filtered results via arrow keys (out of scope).
- Fuzzy or ranked matching if substring search proves insufficient.
- Persisting the query across app restarts (currently out of scope per PRODUCT.md).
