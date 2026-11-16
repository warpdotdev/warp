# APP-3743: Unified New Tab Menu

Linear: [APP-3743](https://linear.app/warpdotdev/issue/APP-3743/new-worktree-ui)

## Summary

Unify the horizontal tab bar's chevron menu and the vertical tab bar's `+` icon menu into a single menu structure. Add a "Worktree in" item with a repo sidecar for one-click worktree creation, including a scrollable search row at the top of the sidecar content and a pinned "Add new repo" footer. On Windows, Terminal gets a sidecar for shell selection; on other platforms Terminal is a simple menu item. The "New Tab Config" item opens the starter TOML template directly as the V0 experience.

## Problem

The horizontal and vertical tab menus are diverging — they show different items, in different orders, with different labels. This is confusing for users who switch between layouts. Additionally, creating a worktree requires opening a modal and filling in multiple fields (repo, branch, checkbox). Power users want a faster flow: pick a repo, get a worktree immediately. Finally, creating a new tab config requires hand-editing TOML — we can instead invoke the `tab-configs` skill to guide the user interactively.

## Goals

- Unify the horizontal chevron and vertical `+` menus into a single item list.
- Add a "Worktree in" item with a searchable repo sidecar for instant worktree creation.
- On Windows, add a Terminal sidecar for shell selection. On macOS/Linux, Terminal is a regular item with the ⌘T shortcut.
- Introduce a default worktree tab config at `~/.warp/default-tab-configs/` that is parameterized by repo and auto-generates the branch name.
- Add a "New Tab Config" menu item that opens the starter TOML template as the V0 experience.

## Non-goals

- Removing the existing New Worktree modal entirely (it may remain accessible via other paths).
- Changing the right-click tab context menu.
- Pixel-perfect submenu styling (the `Menu` component has hardcoded constants; see Known Limitations from APP-3578).
- Implementing nested submenus beyond one level (Terminal submenu and Worktree in submenu are both one level deep from the top menu).

## Figma

- Main menu item (Agent): https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7447-81155&m=dev
- Terminal submenu item (Default): https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7447-82318&m=dev
- Worktree in repo submenu (Search repos): https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7447-83458&m=dev

## User Experience

### Menu unification

The horizontal tab bar's chevron dropdown and the vertical tab bar's `+` button now open the **same** menu with the **same** items. The `toggle_new_session_dropdown_menu` code path no longer branches on `is_vertical_tabs` for item generation — only for positioning and width.

### Top-level menu items (in order)

1. **Agent** — Opens an agent tab. Shows ⌘T keybinding when default session mode is Agent. Icon: `LayoutAlt01`. Hidden if AI is disabled.
3. **Terminal** — On macOS/Linux, opens a terminal tab directly and shows ⌘T when the default session mode is Terminal. Icon: `LayoutAlt01`. On Windows, this is a submenu parent that shows a sidecar with available shells on hover.
3. **Cloud Oz** — Opens a cloud agent tab. Icon: `LayoutAlt01`. Hidden unless `AgentView` + `CloudMode` flags are enabled.
4. **Worktree in** → opens a repo sidecar on hover (see below). Icon: `Dataflow02`.
5. **[User tab configs]** — One item per loaded tab config from `~/.warp/tab_configs/`. Icon: `LayoutAlt01` for non-worktree configs, `Dataflow02` for worktree configs. (Same detection logic as APP-3578.)
6. **Separator**.
7. **New Tab Config** — Auto-runs the `tab-configs` skill. Icon: `Plus`.

### Items removed from both menus

- "Restore Closed Tab" (moved to right-click context menu / keybinding only).
- "Learn about Launch Configs..." link.
- "New Terminal Tab" as a standalone top-level item (replaced by Terminal submenu → Default).
- Launch config items ("Launch {name}") — removed from this menu entirely.
- The split `[+][v]` button in horizontal tabs — replaced by a single button that opens the unified menu.

### Terminal (platform behavior)

On **macOS and Linux**, Terminal is a regular menu item that opens a terminal tab directly. The ⌘T keyboard shortcut is displayed on the item (when the default session mode is Terminal). No submenu or sidecar is shown.

On **Windows**, Terminal is a submenu parent. Hovering it opens a sidecar with a "Default Terminal" row plus available shells (Cmd, PowerShell, WSL, etc.) from `AvailableShells`. The default entry carries the keyboard shortcut and opens the default shell.

### Worktree in sidecar

Hovering "Worktree in" opens a sidecar panel with:

1. **Search repos row** — The first row of the sidecar content is a compact search field labeled "Search repos". It is part of the scrollable content (not pinned), so it scrolls away with the repo list. Typing filters repos by case-insensitive substring match on the repo path.
2. **Known repos list** — Populated from `PersistedWorkspace.workspaces()`, filtered to repos whose path exists and to the current search query. Each item shows the repo path. Icon: `Folder`.

**Clicking a repo**:
1. The system loads the default worktree tab config from `~/.warp/default-tab-configs/worktree.toml`.
2. The `repo` parameter is filled with the selected repo path.
3. The branch name is auto-generated (using the existing `generate_worktree_branch_name()` logic, producing `worktree-1`, `worktree-2`, etc.).
4. The tab config is executed immediately — a new tab opens running `git worktree add` and `cd` commands.
5. The menu closes.

No modal is shown. This is the "fast path" for worktree creation.

**Pinned "Add new repo" footer**: The sidecar keeps an "Add new repo" action pinned to the bottom of the panel while the repo list scrolls independently above it. Clicking it opens a folder picker to register a new repo in `PersistedWorkspace`. After selection, the repo appears in the list.

### Default worktree tab config

A new directory `~/.warp/default-tab-configs/` stores built-in default tab configs that ship with Warp (distinct from user-created configs in `~/.warp/tab_configs/`).

The default worktree config at `~/.warp/default-tab-configs/worktree.toml`:

```toml
name = "Worktree"

[[panes]]
id = "main"
type = "terminal"
cwd = "{{repo}}"
worktree_name_autogenerated = true
commands = [
  "git worktree add -b {{branch_name}} ../{{branch_name}}",
  "cd ../{{branch_name}}",
]

[params.repo]
type = "repo"
description = "Repository to create worktree in"
```

When invoked from the "Worktree in" submenu, the `repo` param is pre-filled with the selected repo path and the `branch_name` is auto-generated (because `worktree_name_autogenerated = true`). The params modal is skipped entirely.

If this file does not exist at `~/.warp/default-tab-configs/worktree.toml`, it is created on first use from an embedded template (similar to how `new_tab_config_template.toml` works). The file is user-editable — users can customize the worktree commands, add additional panes, etc. Warp does not overwrite user modifications on updates.

### New Tab Config menu item

Clicking "New Tab Config" in the menu writes the starter tab-config template to the next unused file under `~/.warp/tab_configs/` and opens it in the user's configured editor. The filesystem watcher then picks it up and it appears in the menu once saved.

## Edge Cases

1. **No repos in PersistedWorkspace**: The "Worktree in" sidecar still shows the search row and the pinned "Add new repo" footer, with no repo rows in between.
2. **Default worktree config missing**: If `~/.warp/default-tab-configs/worktree.toml` doesn't exist, it is created from an embedded template on first invocation.
3. **AI disabled**: The "Agent" item is hidden. "New Tab Config" still appears and opens the TOML template file directly.
4. **No shells detected (Windows)**: The Terminal sidecar shows a single "Terminal" fallback item.
5. **Worktree creation fails**: If `git worktree add` fails (e.g., branch already exists, not a git repo), the error is shown in the terminal output — same behavior as today when a tab config command fails.
6. **Sidecar positioning**: Sidecars open to the right of the parent item, anchored to the hovered item's position.
7. **Feature flags**: Terminal sidecar shells are gated behind `ShellSelector` (Windows only). Cloud Oz is gated behind `AgentView` + `CloudMode`. Tab configs section and Worktree in are gated behind `TabConfigs`.

## Success Criteria

1. The horizontal chevron menu and vertical `+` menu show identical items.
2. On macOS/Linux, Terminal is a regular item with ⌘T. On Windows, Terminal has a sidecar with a default terminal row plus available shells.
3. "Worktree in" shows a sidecar with a scrollable "Search repos" row, filtered known repos from `PersistedWorkspace`, and a pinned "Add new repo" footer.
4. Typing in the sidecar search field filters repo items live.
5. Clicking a repo in the "Worktree in" sidecar immediately opens a new tab with a worktree, using an auto-generated branch name — no modal.
6. The default worktree tab config exists at `~/.warp/default-tab-configs/worktree.toml` and is created from a template if missing.
7. "New Tab Config" creates and opens the starter template under `~/.warp/tab_configs/`.
8. Generated or saved tab configs appear in the menu via the filesystem watcher.

## Validation

- Open both horizontal and vertical tab menus — verify they show the same items.
- Click "Terminal" — verify a terminal tab opens (macOS/Linux). On Windows, verify sidecar shows the default terminal row plus shells.
- Hover "Worktree in" — verify sidecar shows the search row, known repos, and the pinned footer.
- Type in "Search repos" — verify repo rows filter live and the footer stays pinned.
- Move mouse diagonally toward sidecar — verify safe triangle prevents premature closing.
- Click a repo — verify a new tab opens running `git worktree add` with an auto-generated branch name.
- Click "New Tab Config" — verify a starter tab-config file is created and opened in the configured editor.
- Re-open the menu — verify new tab configs appear in the list.

## Open Questions

(None outstanding — all resolved.)
