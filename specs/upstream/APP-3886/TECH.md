# Sidecars for Tab Config Menu — Tech Spec

Linear: [APP-3886](https://linear.app/warpdotdev/issue/APP-3886/sidecars-for-tab-config-menu)
Product spec: `specs/APP-3886/PRODUCT.md`

## Problem
The tab configs menu has no per-item management actions. Users cannot set a default, edit, or remove tab configs from within the menu. The `DefaultSessionMode` setting only supports `Terminal | Agent`, with no way to select a tab config. The sidecar infrastructure exists for worktree/shell submenus but not for action panels.

## Relevant code

**Tab configs menu & sidecar:**
- `app/src/workspace/view.rs:5159` — `unified_new_session_menu_items()` builds the menu (shared by both horizontal and vertical tabs)
- `app/src/workspace/view.rs:5280` — `open_tab_configs_menu()` opens the menu, takes `is_vertical_tabs` param for width
- `app/src/workspace/view.rs:7838` — `update_new_session_sidecar()` dispatches sidecar content based on hovered item
- `app/src/workspace/view.rs:7675` — `configure_terminal_new_session_sidecar()` (Windows-only, to be removed)
- `app/src/workspace/view.rs:7731` — `configure_worktree_new_session_sidecar()`
- `app/src/workspace/view.rs:20291` — dropdown menu overlay rendering (different anchoring for vertical vs horizontal)
- `app/src/workspace/view.rs:20331` — sidecar overlay rendering (shared by both modes, anchored to hovered item label)
- `app/src/workspace/view.rs:966` — `new_session_sidecar_menu` field (existing `Menu`-based sidecar)
- `app/src/workspace/view.rs:1650` — sidecar menu construction in `build_menus()`
- `app/src/workspace/view/vertical_tabs.rs:1031` — `render_new_tab_button()` for vertical tabs (dispatches same `ToggleNewSessionMenu` action)
- `app/src/workspace/view/vertical_tabs.rs:766` — `VERTICAL_TABS_ADD_TAB_POSITION_ID`

**Tab config data model:**
- `app/src/tab_configs/tab_config.rs:128` — `TabConfig` struct (no `source_path` field)
- `app/src/user_config/util.rs:167` — `parse_tab_config_dir_entry()` (path known here, only stored for errors)
- `app/src/user_config/mod.rs:105` — `WarpConfig::tab_configs()` accessor
- `app/src/user_config/native.rs:248` — `load_tab_configs()` and filesystem watcher

**DefaultSessionMode setting:**
- `app/src/settings/ai.rs:253` — `DefaultSessionMode` enum (`Terminal | Agent`, derives `Copy`, `EnumIter`)
- `app/src/settings/ai.rs:1087` — stored as `default_session_mode_internal` in `AISettings`
- `app/src/settings/ai.rs:1227` — `AISettings::default_session_mode()` accessor (gates on AI enabled)
- `app/src/settings_view/features_page.rs:3273` — `update_default_session_mode_dropdown()` builds dropdown from `iter()`

**Cmd+T flow:**
- `app/src/workspace/view.rs:18317` — `AddDefaultTab` handler (the Cmd+T entry point; routes based on `DefaultSessionMode`)
- `app/src/app_menus.rs:1084` — `open_new_default_tab_or_window()` (macOS native menu callback; always dispatches `CustomAction::NewTab` → `AddDefaultTab`)
- `app/src/workspace/view.rs:9637` — `add_new_session_tab_with_default_mode()` (checks `DefaultSessionMode`)
- `app/src/workspace/view.rs:5430` — `open_tab_config()` (opens a tab config, shows params modal if needed)

**Editor setting for opening files:**
- `app/src/util/openable_file_type.rs:97` — `resolve_file_target()` (uses `open_file_editor`)
- `app/src/util/openable_file_type.rs:112` — `resolve_file_target_with_editor_choice()` (accepts explicit editor choice)
- `app/src/util/file/external_editor/settings.rs:66` — `open_code_panels_file_editor` setting
- `app/src/workspace/view.rs:5469` — `create_and_open_new_tab_config()` (currently uses wrong setting)

**Confirmation dialog prior art:**
- `app/src/workspace/close_session_confirmation_dialog.rs` — `CloseSessionConfirmationDialog` pattern

**Windows shell listing:**
- `app/src/workspace/view.rs:5191` — `#[cfg(target_os = "windows")]` Terminal submenu parent
- `app/src/workspace/view.rs:7675` — `configure_terminal_new_session_sidecar()` (lists shells)

## Current state

### Menu structure
`unified_new_session_menu_items()` builds the menu. This function is shared across both horizontal and vertical tab bar modes — the same items appear regardless of layout. The menu is opened via `open_tab_configs_menu()` which accepts `is_vertical_tabs` only to adjust the menu width (268px for vertical, default for horizontal).

Current order:
1. Agent (with Cmd+T shortcut label if default is Agent)
2. Terminal (submenu on Windows, regular item elsewhere; Cmd+T shortcut if default is Terminal)
3. Cloud Oz
4. User tab configs (from `WarpConfig::tab_configs()`)
5. Separator + "New worktree config" (submenu) + "New Tab Config"

### Sidecar positioning (horizontal vs vertical)
The dropdown menu is positioned differently in each mode:
- **Horizontal tabs**: anchored to `NEW_TAB_BUTTON_POSITION_ID` (lower-left of the + button)
- **Vertical tabs**: anchored to `VERTICAL_TABS_ADD_TAB_POSITION_ID` (below the + button)

The sidecar overlay anchors to the hovered menu item's label text, so it's positioned the same way in both modes. The new action sidecar will use the same anchoring mechanism and work in both modes without special handling.

Note: The vertical tabs panel has its own separate "detail sidecar" (`render_detail_sidecar` in `vertical_tabs.rs`) that shows pane details when hovering rows in the panel. This is a completely different system from the new-session menu sidecar and is unrelated to this feature.

### DefaultSessionMode
A `Copy + EnumIter` enum stored in `AISettings`. The settings dropdown iterates it. `default_session_mode()` returns `Terminal` when AI is disabled, otherwise returns the stored value.

### TabConfig
Has no `source_path` field. The file path is available during parsing but discarded for successfully parsed configs.

## Proposed changes

### 1. Add `source_path` to `TabConfig`
**File:** `app/src/tab_configs/tab_config.rs`

Add a skipped field:
```rust
#[serde(skip)]
pub source_path: Option<PathBuf>,
```

**File:** `app/src/user_config/util.rs`

In `parse_tab_config_dir_entry()`, populate `source_path` on successfully parsed configs:
```rust
Some(parsed.map(|mut config| {
    config.source_path = Some(item.path().into());
    config
}).map_err(...))
```

### 2. Extend `DefaultSessionMode` with `TabConfig` and `CloudAgent` variants + companion path setting
**File:** `app/src/settings/ai.rs`

Add `CloudAgent` and `TabConfig` variants to `DefaultSessionMode`:
```rust
pub enum DefaultSessionMode {
    #[default]
    Terminal,
    Agent,
    CloudAgent,
    TabConfig,
}
```

This preserves `Copy` and `EnumIter`. No `Shell` variant is needed — shell-specific defaults are handled by the existing `NewSessionShell` setting (see "Setting interaction" below).

Add a companion setting in `AISettings` to store the tab config file path:
```rust
default_tab_config_path: DefaultTabConfigPath {
    type: String,
    default: String::new(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Never,
    private: false,
    hierarchy: "general",
}
```

`SyncToCloud::Never` because tab config file paths are machine-local.

The companion is only read when mode is `TabConfig`. For all other modes it's ignored.

Add a helper on `AISettings`:
```rust
fn resolved_default_tab_config(&self, app: &AppContext) -> Option<TabConfig>
```
Reads `default_tab_config_path`, finds the matching `TabConfig` in `WarpConfig::tab_configs()` by `source_path`, and returns it. Returns `None` if the path is empty, the file doesn't exist, or the config isn't loaded (triggers fallback to `Terminal`).

### Setting interaction: `DefaultSessionMode` × `NewSessionShell`
Two settings, two concerns:
- **`DefaultSessionMode`** (`Terminal | Agent | CloudAgent | TabConfig`) — controls *what kind of thing* Cmd+T opens.
- **`NewSessionShell`** (existing, in `SessionSettings`) — controls *which shell binary* a terminal session uses. Only relevant when the mode resolves to opening a terminal.

How they interact on Cmd+T:
- `Terminal` → opens a terminal tab using whatever `NewSessionShell` is set to.
- `Agent` → opens agent view. `NewSessionShell` irrelevant.
- `CloudAgent` → opens ambient agent tab. `NewSessionShell` irrelevant.
- `TabConfig` → opens the stored tab config. `NewSessionShell` irrelevant (config defines its own panes/commands).

"Make default" from the sidecar:
- **Terminal item** → sets `DefaultSessionMode::Terminal`. Doesn't touch `NewSessionShell`.
- **A specific shell** (Windows, e.g., PowerShell) → sets `DefaultSessionMode::Terminal` *and* updates `NewSessionShell` to that shell. Now Cmd+T → terminal → PowerShell.
- **Agent / Cloud Oz** → sets `DefaultSessionMode` to `Agent` / `CloudAgent`. Doesn't touch `NewSessionShell`.
- **A user tab config** → sets `DefaultSessionMode::TabConfig` + stores config path in `default_tab_config_path`.

Key invariant: `NewSessionShell` is *always* the authority for which shell a terminal uses. `DefaultSessionMode` never duplicates that.

### 3. Create action sidecar render function
**New file:** `app/src/tab_configs/action_sidecar.rs`

A free function `render_action_sidecar()` (not a `View`) that returns `Box<dyn Element>`. Called directly from the Workspace render method. This is simpler than a View since the sidecar has no internal state — it just reads the current item and settings to produce an element tree.

The function takes a struct describing what to show:
```rust
enum SidecarItemKind {
    BuiltIn { name: String, default_mode: DefaultSessionMode, shell: Option<AvailableShell> },
    UserTabConfig { config: TabConfig },
}
```

Button clicks dispatch `WorkspaceAction` variants directly (`TabConfigSidecarMakeDefault`, `TabConfigSidecarEditConfig`, `TabConfigSidecarRemoveConfig`), which the Workspace handles inline in `handle_action`.

**File:** `app/src/tab_configs/mod.rs` — add `pub(crate) mod action_sidecar;`

### 4. Add `RemoveTabConfigConfirmationDialog`
**New file:** `app/src/tab_configs/remove_confirmation_dialog.rs`

Follows the `CloseSessionConfirmationDialog` pattern (`app/src/workspace/close_session_confirmation_dialog.rs`):
- Title: "Remove tab config?"
- Body: "This will permanently delete {config_name} ({file_path})."
- Buttons: Cancel / Remove (destructive)

Events: `RemoveTabConfigConfirmationEvent::Confirm { path } | Cancel`

On confirm, delete the file from disk. The filesystem watcher handles menu refresh. If the removed config was the default, clear `default_tab_config_path` and set `DefaultSessionMode` back to `Terminal`.

### 5. Wire up action sidecar in Workspace
**File:** `app/src/workspace/view.rs`

Add new fields to `Workspace`:
```rust
tab_config_action_sidecar_item: Option<SidecarItemKind>,
tab_config_action_sidecar_mouse_states: SidecarMouseStates,
remove_tab_config_confirmation_dialog: ViewHandle<RemoveTabConfigConfirmationDialog>,
```

The sidecar is shown when `tab_config_action_sidecar_item` is `Some`. No separate `bool` is needed.

**In `update_new_session_sidecar()`**: Extend the match to handle all actionable items (Terminal, shell variants, Agent, Cloud Oz, user tab configs). For these, set `tab_config_action_sidecar_item` to `Some(item_kind)`. For "New worktree config", keep existing behavior. For "New Tab Config" and separators, set it to `None`.

**In `render()`**: Add a second positioned overlay that calls `render_action_sidecar()` when `tab_config_action_sidecar_item` is `Some` (same positioning logic as the existing sidecar). This overlay uses the same `OffsetPositioning::offset_from_save_position_element` anchored to the hovered menu item label, so it works identically in both horizontal and vertical tabs modes.

**Event handlers**: `WorkspaceAction::TabConfigSidecar*` variants are handled directly in `handle_action`. Subscribe to `RemoveTabConfigConfirmationEvent`.

### 6. Rename `AddTab` → `AddDefaultTab` and route Cmd+T through it
**Files:** `app/src/workspace/action.rs`, `app/src/workspace/view.rs`, `app/src/workspace/mod.rs`, `app/src/app_menus.rs`

Rename `WorkspaceAction::AddTab` to `WorkspaceAction::AddDefaultTab` to clearly distinguish it from the explicit `AddTerminalTab` and `AddAgentTab` actions:
- `AddDefaultTab` = "open whatever the user's default is" (the Cmd+T action). Checks `DefaultSessionMode` and routes accordingly.
- `AddTerminalTab` = "always open a terminal, ignoring default" (explicit override, has its own keybinding).
- `AddAgentTab` = "always open an agent tab" (explicit override).

The `AddDefaultTab` handler checks the effective `DefaultSessionMode`:
1. `TabConfig` → call `resolved_default_tab_config()`. If found, call `open_tab_config()`. If missing, clear to `Terminal` and fall through.
2. `CloudAgent` → call `add_ambient_agent_tab()`.
3. `Agent` / `Terminal` → existing behavior (`add_terminal_tab` internally respects Agent mode).

**macOS native menu (Cmd+T routing):**
On macOS, Cmd+T is handled by the native menu system, not the WarpUI keybinding system. The native menu's "New Terminal Tab" item holds Cmd+T for non-Agent modes; "New Agent Tab" holds it for Agent mode. Both callbacks ultimately dispatch through `CustomAction::NewTab` → `AddDefaultTab`.

The callback `open_new_default_tab_or_window` (`app/src/app_menus.rs`) always dispatches `CustomAction::NewTab`, which the binding system maps to `WorkspaceAction::AddDefaultTab`. This means Cmd+T always goes through the `AddDefaultTab` handler regardless of the current mode — the handler is the single place that routes based on `DefaultSessionMode`.

### 7. Flatten Windows Terminal shell items
**File:** `app/src/workspace/view.rs`

In `unified_new_session_menu_items()`, replace the `#[cfg(target_os = "windows")]` block (`view.rs:5191`) that creates a submenu parent with code that lists each `AvailableShell` as an individual top-level `MenuItem` with `AddTabWithShell` action.

Remove `NewSessionSidecarKind::Terminal`, `configure_terminal_new_session_sidecar()`, and related dead code.

### 8. Fix editor setting for tab config file opens
**File:** `app/src/workspace/view.rs`

In `create_and_open_new_tab_config()` (`view.rs:5469`), change:
```rust
let target = resolve_file_target(&path, settings, None);
```
to:
```rust
let target = resolve_file_target_with_editor_choice(
    &path,
    *settings.open_code_panels_file_editor,
    *settings.prefer_markdown_viewer,
    *settings.open_file_layout,
    None,
);
```

Apply the same fix to `save_current_tab_as_new_config()` (`view.rs:5500`) and the `OpenTabConfigErrorFile` handler (`view.rs:18250`).

The "Edit config" button in the action sidecar will also use `resolve_file_target_with_editor_choice` with `open_code_panels_file_editor`.

### 9. Update settings dropdown
**File:** `app/src/settings_view/features_page.rs`

Replace the `default_session_mode_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>` (`features_page.rs:1222`) with a `FilterableDropdown<FeaturesPageAction>`. The `FilterableDropdown` component (`app/src/view_components/filterable_dropdown.rs`) already supports search/filter, arrow key navigation, and the same `DropdownItem` API — it wraps a `Menu` with a search editor, so users can type to narrow down the list when many tab configs are present.

In `update_default_session_mode_dropdown()` (`features_page.rs:3273`), after the `DefaultSessionMode::iter()` items (Terminal, Agent), append an item for each loaded tab config from `WarpConfig::tab_configs()`, using the config name as the display label and dispatching a new `FeaturesPageAction` variant that sets both `DefaultSessionMode::TabConfig` and `default_tab_config_path`.

Subscribe to `WarpConfigUpdateEvent::TabConfigs` to rebuild the dropdown when configs change.

### 10. Cmd+T keybinding indicator in menu
**File:** `app/src/workspace/view.rs`

In `unified_new_session_menu_items()`, the Cmd+T shortcut label is currently assigned to either the Agent or Terminal item based on `default_is_agent`. Replace this with a comprehensive check of the effective default:

1. `DefaultSessionMode::TabConfig` → attach the shortcut label to the matching tab config's menu item (matched by `source_path`).
2. `DefaultSessionMode::Agent` → attach to the Agent item (existing behavior).
3. `DefaultSessionMode::CloudAgent` → attach to the Cloud Oz item.
4. `DefaultSessionMode::Terminal` → attach to the "Terminal" item.

Note: Per-shell shortcut label logic (i.e., showing Cmd+T on the specific shell item when a shell is made default on Windows) is not implemented in v1. The shortcut label always appears on the "Terminal" item when mode is `Terminal`, regardless of which shell was selected via "Make default".

## End-to-end flow

### Hovering a tab config in the menu
1. User hovers over a tab config item in the dropdown (works identically in horizontal and vertical tabs).
2. `handle_new_session_menu_event` → `ItemHovered` → `update_new_session_sidecar()`.
3. Match identifies the item as a user tab config (by checking if the action is `SelectTabConfig`).
4. Populates `tab_config_action_sidecar` with the config's name, path, and all three buttons.
5. Sets `show_tab_config_action_sidecar = true`, hides `show_new_session_sidecar`.
6. Render places the sidecar overlay anchored to the hovered item.

### "Make default" for a tab config
1. User clicks "Make default" in the action sidecar.
2. Sidecar dispatches `WorkspaceAction::TabConfigSidecarMakeDefault { mode: TabConfig, tab_config_path: Some(path), shell: None }`.
3. Workspace handler sets `default_session_mode_internal` to `TabConfig` and `default_tab_config_path` to the file path.
4. Menu closes. Next time it opens, the Cmd+T shortcut label appears on that config's menu item.
5. Settings dropdown updates via the `AISettingsChangedEvent` subscription.

### Cmd+T with tab config default
1. User presses Cmd+T → native menu dispatches `CustomAction::NewTab` → `AddDefaultTab` action.
2. Handler checks `DefaultSessionMode::TabConfig`.
3. Reads `default_tab_config_path`, finds matching config in `WarpConfig::tab_configs()` via `resolved_default_tab_config()`.
4. Calls `open_tab_config()` → shows params modal if needed, else opens directly.
5. If config file is missing, clears settings to `Terminal` and opens a normal terminal tab.

### "Remove" flow
1. User clicks "Remove" in the sidecar.
2. Sidecar dispatches `WorkspaceAction::TabConfigSidecarRemoveConfig { name, path }`.
3. Workspace opens `RemoveTabConfigConfirmationDialog` with the config name and path.
4. User confirms → dialog emits `Confirm { path }`.
5. Handler deletes the file. If removed config was the default, clears `default_tab_config_path` and sets mode to `Terminal`.
6. Filesystem watcher reloads configs; menu updates on next open.

## Risks and mitigations

- **Backward compat for `DefaultSessionMode` serialization**: Adding a `TabConfig` variant changes serialized values. Old clients reading a `TabConfig` value will fail to deserialize and fall back to the default (`Terminal`). This is acceptable — the worst case is losing the default preference on downgrade.
- **Race between file deletion and watcher**: After "Remove" deletes the file, there's a brief window where the config is still in `WarpConfig::tab_configs()`. The watcher debounce handles this. The sidecar closes the menu on removal, so the user won't see a stale entry.
- **Large number of tab configs**: The settings dropdown and menu will list all configs. No pagination is needed for v1, but configs with identical names are disambiguated by file path in the sidecar subtitle.

## Testing and validation

- **Unit tests**:
  - `TabConfig.source_path` is populated after parsing.
  - `DefaultSessionMode::TabConfig` + `default_tab_config_path` round-trips through serialization.
  - `resolved_default_tab_config()` returns `None` when path is empty, missing, or not in loaded configs.
  - `resolve_file_target_with_editor_choice` is used with `open_code_panels_file_editor` for all tab config file opens.
- **Integration / computer-use**:
  - **Both horizontal and vertical tabs**: Open the tab configs menu in each mode, hover items, verify sidecar appears and is positioned correctly.
  - Make default → verify Cmd+T behavior and settings dropdown sync.
  - Edit config → verify correct editor opens.
  - Remove → verify confirmation dialog, file deletion, menu update, and default fallback.
  - Keyboard navigation through menu → sidecar updates.
  - Vertical tabs: verify the action sidecar doesn't conflict with the vertical tabs detail sidecar (they are independent systems).
- **Regression**:
  - "New worktree config" repo-list sidecar unchanged.
  - Existing `DefaultSessionMode::Terminal` and `Agent` behavior unchanged.
  - Vertical tabs detail sidecar (hover-to-preview pane details) unaffected.

## Follow-ups
- Potential for tab config reordering in the menu.
- Richer sidecar content (preview of pane layout, param summary).
