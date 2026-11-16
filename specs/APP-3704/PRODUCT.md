# Save Current Tab as New Config — Product Spec

Linear: [APP-3704](https://linear.app/warpdotdev/issue/APP-3704/save-current-tab-as-a-new-config)
Figma: [House of Agents — node 7123-32864](https://figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7123-32864&m=dev)

## Summary

Add a "Save as new config" item to the tab right-click context menu that snapshots the current tab's pane layout (splits, working directories, focus, and color) and writes it as a new tab config TOML file in `~/.warp/tab_configs/`.

## Problem

Users can create complex pane layouts interactively (splits, different working directories per pane, tab color) but have no way to persist that layout as a reusable tab config. The only path today is to hand-author TOML from scratch, which requires understanding the schema and manually transcribing the layout.

## Goals

- Let users save their current tab's pane layout as a new tab config TOML with one click.
- Preserve the full spatial structure: splits, working directories, focus state, and tab color.
- Open the generated file in the user's editor so they can rename it and customize further.
- The saved config appears immediately in the `+` tab menu (the filesystem watcher picks it up).

## Non-goals

- **"Save and update config"** (overwriting an existing tab config the tab was opened from). This requires tracking the source config path per tab and is a separate ticket.
- **"Combine panes into single tab"** — shown greyed out in the Figma, separate feature.
- Saving non-terminal pane content (notebooks, code panes, settings panes). These are not replayable from a TOML config.
- Parameterization. The saved config has no `[params]` section; the user can add params manually.

## Figma

Figma: [House of Agents — node 7123-32864](https://figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7123-32864&m=dev)

The updated tab right-click menu adds a new section between the "close" section and the "color" section:
- **Save as new config** — saves the current tab as a new tab config TOML.

Note: The Figma also shows "Save and update config" and "Combine panes into single tab" in this section. Those are out of scope for this ticket (see Non-goals).

## User Experience

### Triggering the save

1. User right-clicks a tab.
2. The context menu includes "Save as new config" (between the close-tab section and the color section). This item is only visible when the `TabConfigs` feature flag is enabled.
3. User clicks "Save as new config".

### What happens

1. Warp snapshots the tab's pane tree: split directions, each pane's current working directory, focus state, and tab color.
2. Warp writes a new `.toml` file to `~/.warp/tab_configs/` with an auto-generated filename (`my_tab_config.toml`, or `my_tab_config_1.toml` if the name is taken).
3. Warp opens the file in the user's configured editor (same behavior as "Create new tab config...").
4. The filesystem watcher picks up the new file, and it appears in the `+` tab menu as "New Tab: My Tab Config".

### TOML structure

The saved config uses the flat `[[panes]]` schema from APP-3575. Example for a two-pane horizontal split:

```toml
name = "My Tab Config"
color = "blue"

[[panes]]
id = "p1"
split = "horizontal"
children = ["p2", "p3"]

[[panes]]
id = "p2"
type = "terminal"
cwd = "/Users/me/project-a"
is_focused = true

[[panes]]
id = "p3"
type = "terminal"
cwd = "/Users/me/project-b"
```

### Non-terminal pane handling

When the tab contains non-terminal panes (notebook, code, settings, etc.), those panes are replaced with an empty terminal pane in the saved config. This preserves the spatial layout (splits are maintained) while substituting content that cannot be replayed from TOML. The replacement terminal pane has no `cwd` set (omitted from TOML; defaults to the user's home directory when opened).

### Tab title and color

- If the tab has a custom title, it is saved as the `title` field.
- If the tab has a color, it is saved as the `color` field.
- The config `name` is always `"My Tab Config"` — the user can rename it in the file.

### Error handling

- If writing the file fails (permissions, disk full), a warning is logged and no file is opened. No toast or modal is shown — this is a low-probability error.

## Success Criteria

1. Right-clicking a tab with `FeatureFlag::TabConfigs` enabled shows "Save as new config" in the context menu.
2. Clicking "Save as new config" on a single-pane terminal tab writes a valid TOML with one `[[panes]]` entry and the correct `cwd`.
3. Clicking "Save as new config" on a two-pane horizontal split writes a TOML with a split root and two leaf children with correct cwds.
4. Clicking "Save as new config" on a tab with a blue color writes `color = "blue"` in the TOML.
5. The focus state is preserved: the focused pane has `is_focused = true`; unfocused panes omit the field entirely.
6. A tab with a non-terminal pane (e.g., a notebook pane in a split) produces a TOML that replaces the notebook with a terminal pane, preserving the split.
7. The written TOML file round-trips: `toml::from_str::<TabConfig>(toml::to_string_pretty(&config))` produces the same `TabConfig`.
8. The file appears in the `+` menu after saving.
9. Opening the saved config from the `+` menu reproduces the original pane layout.
10. The menu item does not appear when `FeatureFlag::TabConfigs` is off.

## Validation

- **Unit tests:** `tab_config_from_pane_snapshot` for single pane, split, 2x2 grid, non-terminal replacement, focus handling, and TOML round-trip.
- **Manual testing:** Right-click → "Save as new config" on various tab layouts, verify the TOML, open the config from the `+` menu, confirm the layout matches.

## Open Questions

(None outstanding.)
