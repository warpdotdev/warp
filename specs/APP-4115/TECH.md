# TECH — Don't auto-add indexed repos to Directory tab colors
See also: `PRODUCT.md` in this directory.
Linear: [APP-4115](https://linear.app/warpdotdev/issue/APP-4115/dont-auto-add-indexed-repos-including-worktrees-to-directory-tab).
## Problem
Two coupled code changes are needed to deliver the product behavior:
1. Stop the `CodebaseIndexManager`-driven subscription that auto-populates `TabSettings.directory_tab_colors` with newly indexed paths.
2. Make the `Add directory color` control conditional: render the existing searchable `FilterableDropdown` when candidate repos exist, and render a plain `ActionButton` that opens the folder picker when the candidate set is empty.
## Relevant code
- `app/src/workspace/view.rs:2775-2790` — the subscription that calls `DirectoryTabColors::merge_new_paths` on every `CodebaseIndexManagerEvent::SyncStateUpdated`.
- `app/src/workspace/tab_settings.rs:168-243` — `DirectoryTabColor` / `DirectoryTabColors`, including `with_color` and `color_for_directory`.
- `app/src/settings_view/appearance_page.rs:1371-1378` — construction of `DirectoryColorAddPicker`.
- `app/src/settings_view/appearance_page.rs:2416-2425` — `handle_directory_color_add_picker_event`.
- `app/src/settings_view/appearance_page.rs:4632-4671` — helper functions for adding a directory tab color directly and opening the folder picker.
- `app/src/settings_view/appearance_page.rs:4674-4815` — `DirectoryTabColorsWidget`, delete buttons, and rendering of the colors list.
- `app/src/settings_view/directory_color_add_picker.rs` — candidate computation, conditional rendering, and footer/button event emission.
- `app/src/view_components/filterable_dropdown.rs` — existing searchable dropdown behavior and pinned footer support.
- `app/src/ai/persisted_workspace.rs:714-721` — `PersistedWorkspace::workspaces()` iterator.
- `crates/ai/src/index/full_source_code_embedding/manager.rs:556-558` — `get_codebase_paths`.
- `crates/warp_util/src/path.rs:85-114` — `user_friendly_path` helper used in the visible list.
## Current state
- On startup and on every indexing sync-state update, the workspace view reads all `CodebaseIndexManager` paths and calls `DirectoryTabColors::merge_new_paths`, which inserts each canonical path into `directory_tab_colors` as `Unassigned` if not already present.
- `DirectoryColorAddPicker` already wraps `FilterableDropdown` and computes the correct candidate set for the searchable known-repo flow.
- The X button on each row dispatches `RemoveDefaultDirectoryTabColor`, which writes `DirectoryTabColor::Suppressed` rather than deleting the key.
## Proposed changes
### 1. Remove the auto-add subscription
Delete the entire `if FeatureFlag::DirectoryTabColors.is_enabled() { ctx.subscribe_to_model(&CodebaseIndexManager::handle(ctx), …) }` block at `app/src/workspace/view.rs:2775-2790`. No replacement subscription is needed.
Delete `DirectoryTabColors::merge_new_paths` and any tests or helpers whose only purpose was validating the old auto-add behavior.
### 2. Keep `FilterableDropdown` and add a fallback button
Keep `app/src/settings_view/directory_color_add_picker.rs` centered on the existing `FilterableDropdown` implementation. Add a second child view for the plain button and track whether the dropdown currently has any candidate rows.
```rust path=null start=null
pub struct DirectoryColorAddPicker {
    button: ViewHandle<ActionButton>,
    dropdown: ViewHandle<FilterableDropdown<DirectoryColorAddPickerAction>>,
    footer_mouse_state: MouseStateHandle,
    has_dropdown_items: bool,
}
```
`DirectoryColorAddPicker::new`:
- Creates the fallback `ActionButton::new("Add directory color", SecondaryTheme).with_icon(Icon::Plus)` and wires it to dispatch `DirectoryColorAddPickerAction::AddNewDirectory`.
- Creates the existing `FilterableDropdown`, keeps its current searchable behavior, and preserves the pinned `+ Add directory…` footer.
- Subscribes to `CodebaseIndexManager`, `PersistedWorkspace`, and `TabSettings` so the candidate list refreshes when any source changes.
`refresh_items`:
- Computes the candidate set using the existing pure helper.
- Converts candidates into `DropdownItem<DirectoryColorAddPickerAction>` values and passes them to `dropdown.set_items(items, ctx)`.
- Sets `has_dropdown_items = !items.is_empty()`.
- If the list becomes empty, closes the dropdown so the view can switch cleanly to the fallback button.
`render`:
- If `has_dropdown_items` is true, render `ChildView::new(&self.dropdown)`.
- Otherwise, render `ChildView::new(&self.button)`.
`handle_action`:
- `Select(path)` emits `DirectoryColorAddPickerEvent::Selected(path)` and lets `FilterableDropdown` close itself after dispatch.
- `AddNewDirectory` closes the dropdown if needed and emits `DirectoryColorAddPickerEvent::RequestAddFromFilePicker`.
### 3. Candidate-set helper
Keep `compute_candidate_paths` as the pure helper that:
- unions indexed and persisted paths,
- canonicalizes keys the same way `DirectoryTabColors::with_color` does,
- dedupes by canonical key,
- filters out entries already present with a non-`Suppressed` color,
- keeps `Suppressed` entries re-addable,
- filters out missing paths,
- sorts by canonical key.
### 4. Appearance page wiring
No appearance-page architectural change is needed beyond the existing `DirectoryColorAddPicker` integration:
- `build_page` continues to construct the picker and subscribe to its events.
- `handle_directory_color_add_picker_event` continues to map `Selected(path)` to `add_directory_tab_color_path(path, ctx)` and `RequestAddFromFilePicker` to `open_directory_tab_color_folder_picker(ctx)`.
- The existing rebuild of `directory_tab_color_delete_buttons` and `color_picker_dot_states` in `handle_tab_settings_event` remains the source of truth for the visible list.
### 5. `Suppressed` behavior
No special-case behavior is needed beyond the existing helper and appearance-page add path:
- If a candidate row corresponds to a `Suppressed` key, selecting it transitions that entry back to `Unassigned` through `with_color(path, DirectoryTabColor::Unassigned)`.
- If the user removes a row and it still exists in the indexed/persisted candidate set, it will reappear in the dropdown.
## End-to-end flow
### Candidate repos exist
1. `DirectoryColorAddPicker` renders the searchable dropdown.
2. The user opens it, filters if needed, and either selects a row or clicks the pinned footer.
3. Row selection emits `Selected(path)`.
4. Footer click emits `RequestAddFromFilePicker`.
### No candidate repos exist
1. `DirectoryColorAddPicker` renders the plain `Add directory color` button.
2. Clicking the button emits `RequestAddFromFilePicker` immediately.
3. The native folder picker opens, and on success the selected path is added through the existing appearance-page helper.
### Indexing sync state updates
1. `CodebaseIndexManagerEvent::SyncStateUpdated` fires.
2. `directory_tab_colors` is not modified.
3. `DirectoryColorAddPicker` refreshes its candidate list and may switch between dropdown and button depending on whether any rows remain.
## Risks and mitigations
- **The control changes shape when the candidate count crosses zero.** This is intentional. The dropdown only adds value when there are known repos to show.
- **`Suppressed` semantics become less obvious.** Keep the existing `Suppressed` write path unchanged so prefix-shadowing behavior remains stable.
- **Stale dropdown state when candidates disappear.** Close the dropdown in `refresh_items` whenever the candidate list becomes empty.
## Testing and validation
- Unit tests on `compute_candidate_paths` covering dedupe, `Suppressed`, missing-path filtering, worktree inclusion, and sort order.
- Regression test confirming that `CodebaseIndexManagerEvent::SyncStateUpdated` no longer mutates `directory_tab_colors`.
- Manual validation of both control states:
  1. Dropdown shown when candidates exist.
  2. Fallback button shown when no candidates exist.
  3. Dropdown row selection adds an `Unassigned` entry.
  4. Dropdown footer opens the folder picker.
  5. Fallback button opens the folder picker.
- `cargo check` and `cargo fmt --check`.
- `verify-ui-change-in-cloud` for the final UI check.
## Follow-ups
None.
