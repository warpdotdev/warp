# Sidecars for Tab Config Menu

Linear: [APP-3886](https://linear.app/warpdotdev/issue/APP-3886/sidecars-for-tab-config-menu)
Figma: [House of Agents – sidecar design](https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7877-45160&m=dev)

## Summary
Add a sidecar panel to every actionable item in the tab configs menu that exposes per-item actions: **Make default**, **Edit config**, and **Remove** (**Edit config** and **Remove** will not be shown for non-user-created tab-configs (i.e. terminal & agent)). Unify the "Make default" choice with the existing "Default mode for new sessions" setting so there is a single source of truth for what Cmd+T does. Flatten the Windows-only Terminal shell submenu into top-level menu items so every item can have a sidecar.

## Problem
Users can create tab configs but have no lightweight way to manage them from the menu. Editing requires manually navigating to `~/.warp/tab_configs/`, there is no way to delete a config from the UI, and there is no way to set a tab config as the default Cmd+T action. The "Default mode for new sessions" setting only offers Terminal and Agent, with no way to select a tab config.

## Goals
- Surface "Make default", "Edit config", and "Remove" actions via a sidecar on menu items.
- Extend the "Default mode for new sessions" setting to include all tab configs, creating a single source of truth for what Cmd+T opens.
- Flatten the Windows Terminal shell submenu into top-level items so every item gets uniform sidecar treatment.
- Respect the correct editor setting (`open_code_panels_file_editor`) when opening tab config files.

## Non-goals
- Inline editing of tab config TOML content from within the sidecar.
- Drag-to-reorder tab configs in the menu.
- A new UI for creating tab configs (the existing "New Tab Config" and "New worktree config" flows remain).
- A sidecar for the "New worktree config" submenu item (it keeps its existing repo-list sidecar and is not something a user would set as default).
- A sidecar for the "New Tab Config" item (it's a creation action, not a selectable default).

## User experience

### Menu structure changes
The tab configs menu items become:

1. **Agent** (if AI enabled)
2. **Terminal** (on all platforms; on Windows this will be multiple items for each terminal type, including the default terminal item)
3. **Additional shell variants** (Windows only — e.g., PowerShell, CMD — listed as individual top-level items instead of nested in a Terminal submenu)
4. **Cloud Oz** (if AI enabled + AgentView + CloudMode flags)
5. **User tab configs** (from `~/.warp/tab_configs/` and `~/.warp/default_tab_configs/`)
6. Separator
7. **New worktree config** (submenu with repo-list sidecar — unchanged)
8. **New Tab Config** (creation action — unchanged)

Items 1–5 each get the new action sidecar. Items 7–8 do not.

### Sidecar trigger
When a user hovers over an actionable item (items 1–5 above), a sidecar panel appears to the right of the menu, following the existing sidecar positioning and safe-zone pattern. The sidecar replaces itself as the user moves between items.

When the user hovers over a separator, "New worktree config", or "New Tab Config", the action sidecar hides. "New worktree config" continues to show its own repo-list sidecar as it does today.

### Sidecar layout
The sidecar panel contains:
1. **Title**: The item name (e.g., "Terminal", "PowerShell", "Oz", or the tab config's `name` field).
2. **Subtitle**: For user tab configs, the full path to the `.toml` source file displayed in a subdued/secondary text style (e.g., `~/.warp/tab_configs/my_config.toml`). For built-in items, no subtitle.
3. **Buttons**: Varies by item type (see below).

### Sidecar buttons by item type

**Built-in items (Terminal, shell variants, Oz, Cloud Oz):**
- "Make default" only.

**User tab configs:**
- "Make default"
- "Edit config"
- "Remove" (destructive/red styling)

### "Make default" behavior
Sets the selected item as the action triggered by Cmd+T (`workspace:new_tab` keybinding).

This directly writes the **"Default mode for new sessions"** setting (see below). The sidecar choice and the settings page dropdown are always in sync — changing one updates the other.

- **Terminal**: Sets default to `Terminal`.
- **A shell variant** (Windows): Sets default to `Terminal`. (Shell-specific defaults are out of scope; this just ensures Cmd+T opens a terminal.)
- **Oz**: Sets default to `Agent`.
- **Cloud Oz**: Sets default to `Cloud Agent` (i.e. the tab you open to when you click this option now)
- **A user tab config**: Sets default to that config (identified by source file path).

When Cmd+T fires:
- If default is `Terminal` → open a terminal tab (existing behavior).
- If default is `Agent` → open an agent tab (existing behavior).
- If default is a tab config → open that config using the same flow as clicking it in the menu (params modal if it has params, direct open if not).

### Extending "Default mode for new sessions"
The existing `DefaultSessionMode` enum (`Terminal | Agent`) is extended to also include a tab-config variant that references a config by source file path. The settings dropdown in the AI/session settings page lists:
- Terminal
- Agent
- Every currently-loaded tab config (by name)

Selecting a tab config from the dropdown sets it as the Cmd+T default, same as clicking "Make default" in the sidecar. The dropdown should update dynamically as tab configs are added/removed (driven by the filesystem watcher).

If the selected tab config is deleted (file removed from disk), the setting falls back to `Terminal` (or whatever `DefaultSessionMode`'s default is).

### Visual indicator for current default
The menu item that is currently the default shows a **Cmd+T keybinding indicator** (matching the Figma design). This is the same keybinding hint pattern used elsewhere in the menu. Only one item displays this indicator at a time — whichever item is the current default.

### "Edit config" behavior
Opens the tab config `.toml` file in the user's configured editor, respecting the **"Choose an editor to open files from the code review panel, project explorer, and global search"** setting (`open_code_panels_file_editor`), **not** the "Choose an editor to open file links" setting (`open_file_editor`).

When the editor resolves to Warp, the file opens in a new tab with the file tree open and focused on the config file.

This editor-setting fix also applies to:
- The existing **"New Tab Config"** button at the bottom of the menu.
- The **"Open file"** action from tab config error toasts (`OpenTabConfigErrorFile`).

### "Remove" behavior
1. A Warp modal (following existing modal prior art, e.g., the close-session confirmation dialog) appears asking the user to confirm deletion, indicating the config name and that the file will be permanently deleted.
2. On confirm, the `.toml` file is deleted from disk.
3. The filesystem watcher picks up the deletion and removes the config from the menu.
4. If the removed config was the current default, the setting reverts to `Terminal`.
5. On cancel, nothing happens.

### Edge cases
- **Default config file deleted externally**: When Cmd+T fires and the stored config path no longer exists, clear the default silently and fall through to `Terminal` behavior. The next watcher event will also clean up the settings dropdown.
- **Config parse error after edit**: Handled by the existing error toast mechanism. The sidecar does not need special handling.
- **Menu keyboard navigation**: Arrow keys update the sidecar to reflect the currently-selected item, matching the existing sidecar behavior.
- **Sidecar safe zone**: Uses the existing safe-zone mechanism so the mouse can travel from the menu to the sidecar without it closing.
- **Multiple configs with the same name**: Each is listed separately; the default is identified by file path, not name, so there is no ambiguity.
- **Windows Terminal flattening**: Removing the Terminal submenu means `NewSessionSidecarKind::Terminal` and `configure_terminal_new_session_sidecar` are no longer needed. The shell items become regular top-level menu entries.

## Success criteria
1. Hovering over any actionable item (Terminal, shell variants, Oz, user tab configs) in the tab configs menu shows a sidecar.
2. The sidecar for user tab configs shows the config name, file path, and three buttons (Make default, Edit config, Remove).
3. The sidecar for built-in items shows only "Make default".
4. Clicking "Make default" updates the "Default mode for new sessions" setting.
5. The settings dropdown in the AI/session settings page lists Terminal, Agent, and all loaded tab configs, and stays in sync with sidecar choices.
6. Pressing Cmd+T opens the currently-configured default (terminal, agent, or tab config).
7. The default persists across app restarts.
8. Clicking "Edit config" opens the `.toml` file using `open_code_panels_file_editor`.
9. Clicking "Remove" shows a confirmation dialog; confirming deletes the file and removes the config from the menu.
10. If the default config is removed (via UI or externally), Cmd+T reverts to Terminal.
11. Keyboard navigation in the menu updates the sidecar.
12. "New Tab Config" and tab config error toasts also respect `open_code_panels_file_editor`.
13. On Windows, Terminal shell variants appear as individual top-level items instead of a submenu.

## Validation
- **Unit tests**: `DefaultSessionMode` extension stores and retrieves tab config paths correctly. Falls back to `Terminal` when the stored path doesn't exist. `resolve_file_target` for tab config files uses `open_code_panels_file_editor`.
- **Manual / computer-use verification**: Open the tab configs menu, hover over items, verify sidecar appears with correct content. Exercise Make default, Edit config, and Remove. Verify Cmd+T behavior changes. Verify settings dropdown stays in sync. Verify Cmd+T reverts after removing the default config.
- **Regression**: Existing "New worktree config" sidecar behavior is unaffected. Existing Cmd+T behavior is unchanged when no custom default is set.
