# Product Spec: Tab Configs in Command Palette and keyboard shortcuts

**Issue:** [warpdotdev/warp#9176](https://github.com/warpdotdev/warp/issues/9176)
**Figma:** none provided

## Summary

Make Tab Configs (TOML configs at `~/.warp/tab_configs/`) accessible through Warp's keyboard-first surfaces — the Command Palette and keyboard-driven activation — matching the affordances Launch Configurations have today. Specifically: index Tab Configs alongside Launch Configurations in the Command Palette, allow keyboard activation through the same path users already know, and document the sub-surface so per-config keybindings can land in a follow-up without reshaping the data model.

This addresses the regression the issue calls out: Tab Configs are positioned as the recommended replacement for Launch Configurations, but the new system loses the keyboard-driven entry points (Cmd+P search and dedicated shortcut) the legacy system always had. A user who migrates is forced back to the mouse for an action they perform many times a day.

## Problem

Today's affordances by surface:

| Surface | Launch Configurations | Tab Configs |
|---|---|---|
| Click `+` button → context menu | ✅ | ✅ |
| Right-click `+` button | ✅ | ✅ |
| **Command Palette search** | ✅ | ❌ |
| **Cmd+Ctrl+L (or equivalent)** | ✅ | ❌ |
| **Menu bar entry** | ✅ | ❌ |

The reporter rates the importance 4/5 and explicitly identifies this as a *regression* from migrating to Tab Configs. The user's stated workaround — *"continue using the legacy Launch Configurations (YAML) to get keyboard access"* — defeats the purpose of the migration.

## Goals

- Tab Configs appear in the Command Palette alongside Launch Configurations, matching display, search, and selection behavior pixel-for-pixel.
- A keyboard-driven activation path lands the same Tab-Config-launch behavior as clicking the `+` button surface.
- The TOML data model and on-disk layout of Tab Configs (under `~/.warp/tab_configs/`) is unchanged. This feature is purely a new entry surface; it does not migrate, rewrite, or otherwise touch existing user files.
- Search-relevance ranking treats Tab Configs and Launch Configurations as peers — neither is shadowed when their names overlap.

## Non-goals (V1 — explicitly deferred to follow-ups)

- **Per-Tab-Config keyboard shortcut bindings.** The reporter mentions this as "ideally"; it requires a new keybinding surface (a keymap entry per config). V1 ships the Command Palette surface so power users can use Cmd+P + filter as their keyboard path, then a follow-up adds dedicated chord bindings on top.
- **Migrating Launch Configurations into the Tab Config palette entry.** Both continue to appear independently in V1. Once Launch Configurations are deprecated, V2 can fold the two surfaces; V1 deliberately keeps them parallel.
- **A new menu-bar entry for Tab Configs.** The reporter mentions menu-bar parity; out of V1 scope. Once Cmd+P search lands, the value of a separate menu-bar entry diminishes.
- **Renaming the Cmd+Ctrl+L Launch Configuration shortcut to cover both.** V1 keeps the existing shortcut bound to Launch Configurations exactly. Cmd+P → typing "tab config" → enter is the keyboard path for Tab Configs.

## User experience

### Searching for a Tab Config in the Command Palette

1. User opens the Command Palette (`Cmd+P` on macOS, `Ctrl+P` on Linux/Windows — existing keybinding, unchanged).
2. User types part of a Tab Config's name, or types `tab config` to filter to all of them.
3. The palette renders matching Tab Configs in the same row layout as Launch Configurations: title (the Tab Config's `name`), an icon distinguishable from Launch Configurations (so visual scanning works), and a subtitle showing the source path (`~/.warp/tab_configs/<name>.toml`). The first matching Tab Config gets keyboard focus by default if the user typed `tab config` (so hitting Enter goes straight to the most likely target).
4. User selects an entry. The same launch behavior fires as clicking the `+` button → Tab Config submenu → that entry: parameters modal opens (if the config declares any), then panes spawn per the config's `panes` array.

### Mixing with Launch Configurations in search results

When both result types match the user's search:

- Both kinds appear in the result list; neither suppresses the other.
- Visual differentiation comes from the row icon. Tab Configs use a distinct icon from Launch Configurations (existing icons: `bundled/svg/tab.svg` or similar — pick one that's recognizably different from `bundled/svg/launch.svg` or whatever the Launch Configuration palette row uses today).
- Sort order: by relevance to the typed query (existing palette ranking — no changes).

### Empty / error states

1. **No Tab Configs defined.** The palette simply shows no Tab Config rows; no special "you haven't created any" affordance. Matches today's Launch Configuration behavior when the user has none.
2. **A Tab Config file exists but fails to parse.** The palette shows a row in error state for that config: the row title is the file's stem, the subtitle is *"Failed to load: `<reason>`"*, and selecting it opens the file in Warp's code editor (so the user can fix it) rather than attempting to launch.
3. **The Tab Config references a parameter the user hasn't filled in.** Selection opens the parameters modal, same behavior as today's `+` button path.

## Configuration shape

No changes to the on-disk Tab Config TOML format. No new settings. The feature is exposed by indexing existing files in an existing location.

The Command Palette infrastructure already supports composable result sources (Launch Configurations, MCP servers, etc.); adding Tab Configs is one more source registration, not a new system.

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. With at least one Tab Config defined, opening the Command Palette and typing `tab config` shows that config in the result list within one frame of the keystroke.
2. Selecting a Tab Config from the Command Palette fires the same launch path as clicking the `+` button → Tab Config submenu → that entry — same parameter modal (if any), same pane-spawn results.
3. With both Tab Configs and Launch Configurations defined, a search query that matches names from both kinds returns both kinds in the result list. Neither suppresses the other.
4. Tab Configs and Launch Configurations are visually distinguishable in the result list via row icon.
5. A Tab Config TOML file with a parse error appears in the result list as an error-state row labeled *"Failed to load: `<reason>`"*. Selecting that row opens the file in Warp's code editor, not the launch path.
6. Renaming or deleting a Tab Config TOML file on disk causes the next Command Palette open to reflect the change — no Warp restart, no manual refresh.
7. The existing Cmd+Ctrl+L Launch Configuration shortcut (or its platform equivalent) is unchanged in behavior. V1 does not bind a new shortcut for Tab Configs.
8. Selecting a Tab Config from the palette emits a telemetry event distinct from the Launch Configuration palette telemetry, so adoption can be measured.

## Open questions

- **Result-list label.** "Tab Config: `<name>`" or just `<name>` with the icon doing the disambiguation? Recommend just `<name>` to match Launch Configuration row layout, leaning on the icon for the type signal. Consistent with "show, don't tell" and reduces noise when users have many configs.
- **Telemetry event naming.** A new `TabConfigPaletteSelected` event vs. extending the existing Launch Configuration palette telemetry with a discriminator field. Recommend a new event name to keep dashboards clean.
- **Future menu bar entry.** Out of V1 scope but worth confirming with maintainers whether this is acceptable to defer. The Command Palette surface is keyboard-equivalent; the menu bar adds discoverability for non-keyboard users, who already have the `+`-button submenu.
