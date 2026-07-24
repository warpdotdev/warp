# Tech Spec: Tab transfer between standard and dedicated hotkey windows

**Issue:** [warpdotdev/warp#12306](https://github.com/warpdotdev/warp/issues/12306)
**Product spec:** `specs/GH12306/product.md`

## Context

The dedicated hotkey window (quake mode) is created as a `WindowStyle::Pin` window tracked through a global `QUAKE_STATE` singleton in `root_view.rs`. It has its own `Workspace` instance with its own tab bar. Cross-window tab transfers already exist for standard windows via:
- `Workspace::get_tab_transfer_info_for_attach` (line 26703) — snapshots a tab for transfer
- `Workspace::prepare_for_transferred_tab_attach` (line 26716) — detaches the pane group
- `Workspace::insert_transferred_tab_at_index` (line 26734) — inserts into target workspace
- `root_view::create_transferred_window` (line 581) — creates a new window for tab(s)
- `ctx.transfer_view_tree_to_window` — moves the view tree across windows

The binding system supports context-predicate gating via `EditableBinding::with_context_predicate`, where context flags are set per-view in `View::context()`.

The Warp team is shipping cross-window tab drag (hotkey→standard direction) in the same release cycle ([maintainer comment](https://github.com/warpdotdev/warp/issues/12306#issuecomment-4749224419)). The command-palette actions in this spec are complementary: they provide a keyboard-driven alternative that works in both directions, and they share the same transfer primitives the drag path uses.

Gaps in the current system:
- No `WorkspaceAction` variants for moving tabs to/from the hotkey window specifically.
- No `Workspace_InQuakeWindow` context flag — views cannot self-identify as belonging to the hotkey window.
- `toggle_quake_mode_window` is private to `root_view` (line 1331) — workspace actions cannot programmatically open the hotkey window.
- `selected_tab_indices` on `Workspace` is private to `tab_grouping.rs` (line 108) — multi-selection state is inaccessible outside that module.

## Proposed changes

### 1. `app/src/workspace/action.rs` (lines 136, 867)

Add three variants to `WorkspaceAction`:

```
MoveActiveTabToDedicatedHotkeyWindow,
MoveActiveTabToStandardWindow,
ToggleActiveTabWindowType,
```

Add corresponding match arms in the `WorkspaceAction` impl block (around line 867).

### 2. `app/src/workspace/mod.rs` (line 482) and `app/src/root_view.rs`

Add a global `static QUAKE_MODE_SETTING_ENABLED: AtomicBool` in `root_view.rs`, updated whenever the dedicated hotkey window setting changes (in the settings update handler).

Register three `EditableBinding`s with both a context predicate and an enabled predicate:

| Command | Context predicate | Enabled predicate |
|---|---|---|
| `workspace:move_active_tab_to_dedicated_hotkey_window` | `Workspace & Quake_Mode_Editor & !Workspace_InQuakeWindow` | `QUAKE_MODE_SETTING_ENABLED.load(Ordering::Relaxed)` |
| `workspace:move_active_tab_to_standard_window` | `Workspace & Quake_Mode_Editor & Workspace_InQuakeWindow` | `QUAKE_MODE_SETTING_ENABLED.load(Ordering::Relaxed)` |
| `workspace:toggle_active_tab_window_type` | `Workspace & Quake_Mode_Editor` | `QUAKE_MODE_SETTING_ENABLED.load(Ordering::Relaxed)` |

(Implemented as closures `|| QUAKE_MODE_SETTING_ENABLED.load(Ordering::Relaxed)` coercing to `fn() -> bool`.)

The context predicate hides from the command palette and dispatch; the enabled predicate (a `fn() -> bool` from a global atomic) hides from the keybindings settings UI — matching the established pattern of `FeatureFlag::is_enabled()` and test-statics in the codebase. `Workspace_InQuakeWindow` (new) differentiates standard from hotkey windows.

### 3. `app/src/workspace/view.rs` and `app/src/settings_view/mod.rs`

Add `pub const WORKSPACE_IN_QUAKE_WINDOW_FLAG: &str = "Workspace_InQuakeWindow";` to the settings flags module (alongside `QUAKE_MODE_ENABLED_CONTEXT_FLAG`).

**Context flag** (~line 22194): Insert `Workspace_InQuakeWindow` when `quake_mode_window_id() == Some(self.window_id)`.

**Action dispatch** (line 22973): Route the three new variants to handler methods.

**New handler methods** (~line 26703, alongside existing transfer functions):
- `move_active_tab_to_dedicated_hotkey_window` — Verifies the hotkey window is open and visible via `quake_mode_window_is_open()` (opens it via the newly-`pub(crate)` `toggle_quake_mode_window` if closed or hidden), then calls `move_active_tab_to_window`.
- `move_active_tab_to_standard_window` — Finds an existing standard window via `ctx.ordered_window_ids()` (z-order front-to-back, skipping self and quake). If one exists, calls `move_active_tab_to_window` with that window as target. If none exists, collects all transfer infos in ascending order, calls `prepare_for_transferred_tab_attach` on the source for each, creates a new window for the first tab via `create_transferred_window`, then transfers remaining tabs to that new window via `transfer_view_tree_to_window` + `insert_transferred_tab_at_index` for each. Removes all source tabs in descending order, clears `suppress_detach_panes_on_window_close` if source stays alive.
- `toggle_active_tab_window_type` — Dispatches to the appropriate directional method based on current window.
- `move_active_tab_to_window(target_window_id, ctx)` — Core logic: gathers selected indices (multi-selection from `selected_tab_indices()` or fallback to `active_tab_index`), collects transfer infos in ascending order. For each tab: calls `prepare_for_transferred_tab_attach` on the source workspace, then `ctx.transfer_view_tree_to_window`. After all transfers, removes tabs from source in descending order (to avoid index shift). If the source still has remaining tabs, clears `suppress_detach_panes_on_window_close` via `set_suppress_detach_panes_on_window_close(false)`. Inserts collected tabs at end of target in original ascending order. Focuses target window.

### 4. `app/src/workspace/view/tab_grouping.rs:108`

Change `selected_tab_indices` from `fn` to `pub(crate) fn`.

### 5. `app/src/root_view.rs`

Change `toggle_quake_mode_window` from `fn` to `pub(crate) fn` (line 1331).

## End-to-end flow

### Move to dedicated hotkey window

1. User invokes the command-palette action (or keybinding) from a standard window.
2. The `EditableBinding` matches via context predicate (`Workspace & Quake_Mode_Editor & !Workspace_InQuakeWindow`).
3. `WorkspaceAction::MoveActiveTabToDedicatedHotkeyWindow` is dispatched to `TypedActionView`.
4. `move_active_tab_to_dedicated_hotkey_window` is called.
5. `quake_mode_window_id()` retrieves the hotkey window ID, verified with `quake_mode_window_is_open()`.
6. If the window is not open or is hidden, `toggle_quake_mode_window` opens or reveals it; the window ID is re-checked.
7. `move_active_tab_to_window(target_window_id, ctx)` is called.
8. `selected_tab_indices()` gathers multi-selected indices, or falls back to `active_tab_index`.
9. For each selected tab: `get_tab_transfer_info_for_attach` → `prepare_for_transferred_tab_attach` → `ctx.transfer_view_tree_to_window`.
10. Tabs are removed from the source (descending index via `remove_tab_without_undo`, or `close_window_for_content_transfer` if it was the last tab). If the source still has remaining tabs after removal, `suppress_detach_panes_on_window_close` is cleared via `set_suppress_detach_panes_on_window_close(false)`.
11. Target workspace inserts each tab at the end via `insert_transferred_tab_at_index`.
12. Focus moves to the target window.

### Move to standard window

If an existing standard window is found: same flow as above (collect, prep, transfer, insert, remove, cleanup), targeting that window via `move_active_tab_to_window`.

If no standard window exists: collect all transfer infos in ascending order, call `prepare_for_transferred_tab_attach` on the source for each, create a new window via `create_transferred_window` with the first tab's info, then transfer remaining tabs to the new window via `transfer_view_tree_to_window` + `insert_transferred_tab_at_index` for each. Remove all source tabs in descending order. Clear `suppress_detach_panes_on_window_close` if the source window remains alive. Focus moves to the new standard window.

### Toggle

`toggle_active_tab_window_type` checks whether the current window is the quake window and dispatches to the appropriate directional method.

## Testing and validation

### Behavior-to-verification mapping

- **1 — Move to hotkey (opens first if needed):** Enable quake mode, close overlay, invoke command from standard window.
- **2 — Move to standard (create if none exist):** Invoke from quake overlay with and without other windows open.
- **3 — Toggle moves to opposite type:** Invoke toggle from both window types.
- **4 — Multi-selection:** Enable `FeatureFlag::GroupedTabs`, select 2+ tabs, invoke each action.
- **5 — Last tab closes window:** Move the only tab out of a window.
- **6 — Hidden from command palette when setting disabled:** Disable quake mode, search command palette.
- **6b — Hidden from keybindings settings UI when setting disabled:** Open Settings > Keybindings, verify the three commands are absent from the list.
- **7 — Multi-tab quake stays open:** Move one tab out of a multi-tab quake overlay.
- **8 — Dynamic window ID:** Close quake window between action invocation and execution.

### Unit tests

- `test_move_active_tab_to_window` — Creates two mock workspaces, adds a tab to the source, calls `move_active_tab_to_window` targeting the second workspace, asserts tab counts shifted correctly.

### Presubmit

- `./script/presubmit` must pass.
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` must pass.
- `cargo nextest run` must pass.

## Risks and mitigations

- **`toggle_quake_mode_window` misidentifies state** — Window state verified via `quake_mode_window_is_open()` before and after the toggle call, which distinguishes hidden from truly closed.
- **Stale multi-selection indices** — Removal iterates descending to avoid index shift.
- **Leaked `suppress_detach_panes_on_window_close`** — Source window always clears the flag (via `close_window_for_content_transfer` on last tab, or `set_suppress_detach_panes_on_window_close(false)` if tabs remain).
- **Race from parallel actions** — Single-threaded event loop serializes all mutations.
- **Exposing `toggle_quake_mode_window`** — Only `pub(crate)`, visible within the crate only.

## Follow-ups

- **Drag-and-drop standard→hotkey** — The team is shipping hotkey→standard drag this cycle. The reverse direction would extend the cross-window drag system (`Workspace::drag_tab_over`, `tab_insertion_index_for_cursor`) to accept `WindowStyle::Pin` windows as valid drop targets.
- **Non-dedicated hotkey mode** — Deferred per maintainer signal. Would require a new `GlobalHotkeyMode` variant, different window creation path, changes to quake state lifecycle.
