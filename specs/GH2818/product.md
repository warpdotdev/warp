# Product Spec: Open a launch configuration or tab config with a keyboard shortcut

**Issue:** [warpdotdev/warp#2818](https://github.com/warpdotdev/warp/issues/2818)

**Figma:** none provided

## Summary

Let users bind a keyboard shortcut to any saved **launch configuration** or **tab config**, so a
single keystroke opens a new tab (or set of tabs) in a specific directory with a specific layout.
Every config the user has on disk shows up as an assignable action in **Settings → Keyboard
Shortcuts**, exactly like a built-in action, and the assignment is stored in the existing
`keybindings.yaml`. No new file format and no new settings surface are introduced.

This is the pathway proposed in the issue: rather than a one-off "new tab in directory X"
binding, shortcuts attach to the two config primitives Warp already has for "open a session in a
known place with a known shape".

## Problem

Warp can set a *default* directory for new tabs, and it can open a saved launch configuration or
tab config from the command palette, the `+` menu, the app menu, or a `warp://` URI. What it
cannot do is open a specific one from the keyboard. A developer who bounces between three
repositories has to reach for the palette, type, and confirm every time; a muscle-memory
`ctrl-alt-1` / `ctrl-alt-2` / `ctrl-alt-3` is not possible.

The keybinding system today is a fixed, compiled-in list of actions. A user-authored config is not
an action, so there is nothing to bind to. The feature is "make each saved config an action".

## Goals

- Every loaded launch configuration and tab config is bindable from Settings → Keyboard Shortcuts
  and from `keybindings.yaml`, with no shortcut assigned by default.
- Pressing the shortcut opens the config **in the active window** so the result is a new tab (or
  tabs), matching the issue's intent.
- Configs added, removed, or renamed on disk update the set of bindable actions without a
  restart, reusing the existing file watcher.
- Reuse the existing keybinding editor, conflict indication, persistence, and reset/remove
  affordances unchanged.

## Non-goals

- A generic "open a new tab at `<path>`" binding with the path written inline in
  `keybindings.yaml`. Bindings stay a flat `action → keystroke` map; the path lives in the config.
- Declaring the shortcut *inside* the launch config / tab config file (a shareable default). Listed
  as a follow-up; it layers cleanly on top of this design.
- Hot-reloading `keybindings.yaml` itself. As today, edits to that file take effect on the next
  launch; edits made through Settings take effect immediately.
- Showing config actions in the Resource Center keybindings cheat-sheet (its sections are curated
  static lists).
- The headless TUI (`warp_tui`) — it has a separate keybinding system and no configs.

## Behavior invariants

### Discoverability and assignment

1. For every launch configuration loaded from `~/.warp/launch_configurations/`, Settings →
   Keyboard Shortcuts lists an action described as `Open launch configuration "<name>"`, where
   `<name>` is the config's `name` field. It has no shortcut until the user assigns one.
2. For every tab config loaded from `~/.warp/tab_configs/`, the page lists
   `Open tab config "<name>"`, where `<name>` is the config's `name` field. It has no shortcut until
   the user assigns one.
3. Searching the Keyboard Shortcuts page by the config's name, or by "launch configuration" /
   "tab config", finds the action.
4. Assigning, changing, removing (`none`), or resetting a shortcut for a config action behaves
   identically to a built-in action: the same editor row, the same conflict indication when the
   keystroke is already taken, the same persistence. Reset returns the action to *unbound*, since
   there is no compiled-in default.
5. The assignment is persisted in `keybindings.yaml` under a stable key:
   - launch configuration: `"launch_config:open:<name>"` — `<name>` is the config's `name`, the
     same identifier `warp://launch/<name>` accepts;
   - tab config: `"tab_config:open:<file-stem>"` — the TOML file name without extension, the
     same identifier `warp://tab_config/<file-stem>` accepts.
   A user who writes such a key by hand (quoted, as all action keys are) gets the same result after
   the next launch.

### Triggering

6. Pressing a bound launch-configuration shortcut while a Warp window is focused opens that
   configuration into the **active window**: its tabs are appended to the current window and the
   configuration's active tab is focused — the same behavior as the command palette's
   "open in active window" affordance. If the configuration defines more than one window, it
   opens new windows exactly as it does from the palette or `+` menu today.
7. Pressing a bound tab-config shortcut opens that tab config in the active window: immediately
   when the config has no parameters, otherwise the existing parameter modal opens first — the
   same behavior as choosing it from the `+` menu.
8. Config shortcuts fire in the same contexts as `workspace:new_tab` (including when a terminal
   pane has keyboard focus) and are suppressed in the same contexts (e.g. while a pane is being
   dragged).
9. If the bound config cannot be found when the key is pressed (it was deleted and the reload
   raced the keystroke), Warp shows an ephemeral toast `Launch configuration "<name>" not found`
   / `Tab config "<file-stem>" not found` and does nothing else.

### Lifecycle

10. Adding, removing, or renaming a config file updates the set of actions without a restart. If
    the Keyboard Shortcuts page is open, its list refreshes.
11. Removing a config leaves its `keybindings.yaml` entry in place and inert. Re-adding a config
    with the same name / file stem re-activates the stored shortcut without user action.
12. If two launch configurations share a `name` (case-insensitive), or two tab configs share a
    file stem, exactly one action is registered — the first loaded — and a warning is logged.
    This mirrors how `warp://launch/<name>` resolves duplicates today.
13. Config actions are gated by the same availability as the configs themselves: launch
    configuration actions are absent when launch configurations are disabled for the account,
    tab config actions are absent when tab configs are disabled.
14. With the feature's rollout flag off, no config actions are registered; any stored
    `launch_config:open:*` / `tab_config:open:*` entries in `keybindings.yaml` are ignored.

### Edge cases

15. Names containing spaces, quotes, colons, or non-ASCII characters are used verbatim in the
    description and in the `keybindings.yaml` key; keys are quoted so YAML parses them.
16. A tab config whose file stem is not valid UTF-8 is skipped with a warning; every other config
    still registers.
17. Accessibility: the Settings row is announced with its full description
    (`Open launch configuration "<name>"`) and is fully operable from the keyboard, like every
    other row. The toast in (9) is announced like other ephemeral toasts.

## Example

```yaml
# ~/.warp/launch_configurations/backend.yaml
name: backend
windows:
  - tabs:
      - title: api
        layout:
          cwd: ~/code/backend
```

```toml
# ~/.warp/tab_configs/frontend.toml
name = "Frontend"
title = "web"
[[panes]]
id = "root"
type = "terminal"
directory = "~/code/frontend"
```

After the two files are loaded, Settings → Keyboard Shortcuts shows
`Open launch configuration "backend"` and `Open tab config "Frontend"`. The user assigns
`ctrl-alt-1` and `ctrl-alt-2`. `keybindings.yaml` now contains:

```yaml
"launch_config:open:backend": ctrl-alt-1
"tab_config:open:frontend": ctrl-alt-2
```

From then on `ctrl-alt-1` appends an `api` tab rooted at `~/code/backend` to the current window
and focuses it; `ctrl-alt-2` opens a `web` tab rooted at `~/code/frontend`.

## Open questions

1. **Active window vs. new window for launch configurations.** This spec proposes "active window"
   for the keyboard path because the issue is about tabs and the URI/menu paths already cover the
   new-window case. Alternative: keep new-window parity with the palette's plain Enter, and offer a
   second action per config for the active-window variant. Two actions per config doubles the list
   in Settings; the proposal is one action, active-window semantics.
2. **Shortcut declared in the config file.** Should `keybinding:` be accepted in the launch
   config YAML / tab config TOML as a *default* (overridable in Settings)? It makes shortcuts
   travel with shared configs. Deferred to a follow-up so this change stays inside the existing
   keybinding model; nothing here precludes it.
3. **Telemetry granularity.** The launch-configuration open event already records the UI
   location; a `Keybinding` value is added. Tab config open events currently do not record a
   source; adding one touches the exhaustive telemetry table and is left to the maintainers'
   preference.
