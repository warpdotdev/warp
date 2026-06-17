# Save Current Tab as New Config — Tech Spec

Product spec: `specs/APP-3704/PRODUCT.md`

## Problem

There is no code path to convert a live tab's pane tree into a `TabConfig` TOML. The existing `PaneTemplateType::try_from(PaneNodeSnapshot)` in `launch_configs/launch_config.rs` converts snapshots to the launch config format but drops non-terminal panes entirely and produces a `PaneTemplateType`, not a `TabConfig`.

## Relevant Code

- `app/src/tab_configs/session_config.rs:174` — `tab_config_from_pane_snapshot` (new)
- `app/src/tab_configs/session_config.rs:194` — `snapshot_to_flat_panes` helper (new)
- `app/src/tab_configs/session_config.rs:144` — `write_tab_config` (updated signature)
- `app/src/tab_configs/tab_config.rs` — `TabConfig`, `TabConfigPaneNode`, `TabConfigPaneType`
- `app/src/pane_group/mod.rs:2029` — `PaneGroup::snapshot()` returns `PaneNodeSnapshot`
- `app/src/app_state.rs` — `PaneNodeSnapshot`, `LeafContents`, `TerminalPaneSnapshot`, `BranchSnapshot`
- `app/src/tab.rs:340` — `save_config_menu_items` (new)
- `app/src/workspace/action.rs:561` — `WorkspaceAction::SaveCurrentTabAsNewConfig`
- `app/src/workspace/view.rs:4969` — `save_current_tab_as_new_config` handler (new)
- `app/src/workspace/view.rs:16831` — action dispatch
- `app/src/user_config/mod.rs` — `find_unused_toml_path`, `tab_configs_dir`

## Current State

**Snapshot infrastructure:** `PaneGroup::snapshot()` recursively walks the pane tree and produces a `PaneNodeSnapshot` with `Branch` and `Leaf` variants. Leaf snapshots carry `LeafContents` which is `Terminal(TerminalPaneSnapshot)`, `Notebook(...)`, `Code(...)`, etc. `TerminalPaneSnapshot` includes `cwd: Option<String>` and `is_active: bool`.

**Tab config serialization:** `TabConfig` derives both `Serialize` and `Deserialize` (added in APP-3575). `write_tab_config` in `session_config.rs` serializes via `toml::to_string_pretty` and writes to disk.

**Tab right-click menu:** Built in `TabData::menu_items()` with four sections: session sharing, modify tab, close tab, and color. Each section is a separate method returning `Vec<MenuItem<WorkspaceAction>>`.

**Open-in-editor pattern:** `create_and_open_new_tab_config` writes a file, resolves the editor target via `EditorSettings`, and calls `open_file_with_target`.

## Changes

### 1. `tab_config_from_pane_snapshot` and `snapshot_to_flat_panes`

Added to `app/src/tab_configs/session_config.rs`. Public function `tab_config_from_pane_snapshot` accepts `&PaneNodeSnapshot`, optional title, and optional color. It delegates to a private recursive helper `snapshot_to_flat_panes` that walks the tree with a `&mut usize` counter for ID generation (`"p1"`, `"p2"`, …).

Branch nodes: reserve their ID, recurse into children to collect child IDs, then insert the split node before the children in the flat list (root-first ordering via `panes.insert(insert_pos, ...)`).

Terminal leaves: extract `cwd` from `TerminalPaneSnapshot.cwd`, set `pane_type = Terminal`, set `is_focused = Some(true)` only when focused (otherwise `None` — omitted from TOML).

Non-terminal leaves: produce a terminal pane with no `cwd`, preserving the split layout.

Returns `TabConfig { name: "My Tab Config", title: custom_title, color, panes, params: HashMap::new() }`.

### 2. `write_tab_config` generalized

Signature changed to accept `base_name: &str` instead of hardcoding `"startup_config"`. Call site in `handle_session_config_completed` updated to pass `"startup_config"`. The new save handler passes `"my_tab_config"`.

### 3. `WorkspaceAction::SaveCurrentTabAsNewConfig(usize)`

Added at `workspace/action.rs:561`. Returns `false` in `should_save_app_state_on_action` — writing a config file doesn't change workspace state.

### 4. `save_config_menu_items` in `TabData`

Added at `tab.rs:340`. Static method gated behind `FeatureFlag::TabConfigs`. Returns a single "Save as new config" item dispatching `SaveCurrentTabAsNewConfig(index)`. Inserted in `menu_items()` between the close-tab section and the color section.

### 5. `save_current_tab_as_new_config` workspace handler

Added at `workspace/view.rs:4969`. Gated behind `#[cfg(feature = "local_fs")]` with a no-op stub for non-local-fs builds. Inline-imports `tab_config_from_pane_snapshot` and `write_tab_config` to match the existing inline import pattern used by `handle_session_config_completed`. Snapshots the tab's pane group, extracts custom title and color, builds the `TabConfig`, writes TOML to `~/.warp/tab_configs/my_tab_config.toml` (collision-safe), and opens the file in the user's editor via `resolve_file_target` + `open_file_with_target`.

Action dispatch wired at `workspace/view.rs:16831`.

## End-to-End Flow

1. User right-clicks a tab → `TabData::menu_items()` runs, includes "Save as new config" (flag-gated).
2. User clicks → `WorkspaceAction::SaveCurrentTabAsNewConfig(tab_index)` dispatched.
3. Workspace handler snapshots the tab's `PaneGroup` via `snapshot()`.
4. `tab_config_from_pane_snapshot` converts the `PaneNodeSnapshot` tree into a flat `TabConfig`.
5. `write_tab_config` serializes to TOML and writes to `~/.warp/tab_configs/my_tab_config.toml`.
6. The file is opened in the user's configured editor.
7. The filesystem watcher detects the new file and adds it to the `+` menu.

## Risks and Mitigations

**Non-terminal pane replacement:** Replacing non-terminal panes with empty terminals is lossy — the user might not expect an empty pane when reopening the config. Mitigated by opening the file in the editor immediately, so the user can see and adjust the result.

**Snapshot timing:** `PaneGroup::snapshot()` is synchronous and captures the current state at call time. If a shell is still initializing, `cwd` might be empty. This is the same risk as the existing launch config save flow, which is acceptable.

## Testing and Validation

### `tab_config_from_pane_snapshot` (unit tests in `session_config_tests.rs`)

- `snapshot_single_terminal_pane` — single leaf produces correct cwd, `is_focused = true`, ID `"p1"`.
- `snapshot_two_pane_horizontal_split` — split root `"p1"` + two leaf children `"p2"` and `"p3"` with correct cwds.
- `snapshot_2x2_grid` — 3 split nodes + 4 leaf nodes = 7 total panes, root is `"p1"`.
- `snapshot_non_terminal_leaf_replaced_with_terminal` — notebook pane replaced with `pane_type = Terminal`, `cwd = None`.
- `snapshot_preserves_custom_title_and_color` — title and color propagated to `TabConfig`.
- `snapshot_round_trip_toml` — `toml::to_string_pretty` → `toml::from_str` produces matching `TabConfig`.

### `write_tab_config` with custom base name

- `write_tab_config_custom_base_name` — base name `"my_tab_config"` writes to `my_tab_config.toml`.
- Existing `write_tab_config_creates_file_with_correct_naming` tests updated for new `base_name` parameter.

### Manual verification

- Build and run Warp, create a split layout, right-click → "Save as new config", verify the TOML, open from `+` menu.

## Follow-ups

- **"Save and update config":** Track which tab config a tab was opened from (store path on `TabData` or `PaneGroup`), and overwrite on save.
- **Agent/Cloud pane handling:** Currently all panes save as `type = "terminal"`. Once agent/cloud panes are more common, detect them from the snapshot and save `type = "agent"` or `type = "cloud"`.
