# Onboarding Tab Config Modal — Product Spec

Linear: [APP-3680](https://linear.app/warpdotdev/issue/APP-3680/onboarding-tab-config-flow-new-tab-config-created-for-user)
Figma: [House of Agents — node 7077-23101](https://figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7077-23101&m=dev)

## Problem

After completing onboarding, users land in an empty terminal tab with no guidance on configuring their first working session. There is no streamlined way to pick a session type (terminal vs. agent), choose a project directory, enable worktree support, and optionally persist that setup as a reusable tab config — all in a single flow.

## Summary

A new modal ("Create your default tab config") appears after onboarding completes, overlayed on the terminal. It collects three inputs — session type, directory, and worktree preference — then creates a persistent tab config TOML in `~/.warp/tab_configs/` and opens it in the current tab.

## Goals

- Let users configure their first session immediately after onboarding in a single modal.
- Support Built in agent (Oz), third-party CLI agents (Claude, Codex, Gemini), and Terminal as session types.
- Always persist the configuration as a reusable tab config TOML in `~/.warp/tab_configs/`.
- Keep the modal implementation reusable so it can be surfaced in other contexts later (e.g., from a menu or command palette).

## Non-goals

- Worktree name generation (Moira is handling this in parallel; hardcode the branch name for now).
- Editing or managing existing tab configs from this modal.
- Showing this modal on every new tab — it only appears post-onboarding in the open source onboarding rev.
- Web/WASM support — this is local-only (`local_fs`).

## User Experience

### When the modal appears

The modal appears once, immediately after the user completes the onboarding slide flow. It only appears when `OpenWarpNewSettingsModes` is enabled — this is the new onboarding path. Users on the old onboarding flow never see this modal. It is rendered as a centered overlay on top of the terminal workspace (not as a full-screen onboarding slide). The user cannot interact with the terminal behind it while the modal is open.

### Modal layout (per Figma)

- **Title:** "Create your default tab config"
- **Subtitle:** "Select if you'd like to work in the terminal versus with an agent of your choosing."
- **Session type:** A row of selectable pill-style buttons that wrap. Options (in order): Built in agent, Claude, Codex, Gemini, Terminal. Only one can be selected at a time. Built in agent is selected by default.
- **Select directory:** A button that opens a native OS folder picker. Displays the selected path left-aligned (defaults to `~`). Text is semibold, no folder icon.
- **Enable worktree support:** A checkbox. Disabled with a tooltip ("Select a git repository to enable worktree support") when the selected directory is not a git repository. Unchecked by default.
- **"Get warping" button:** Primary action button with Enter keyboard shortcut.

### Session type behavior

Each session type determines what happens when the user clicks "Get warping":

- **Terminal:** Opens a terminal session in the selected directory. No command is auto-run.
- **Oz:** Opens the tab into agent view / Oz agent mode (not a CLI command). The tab starts in the selected directory with the Oz agent UI active.
- **Claude / Codex / Gemini:** Opens a terminal session in the selected directory and auto-runs the corresponding CLI command (`claude`, `codex`, `gemini`).

### Directory selection

Clicking the directory button opens a native OS folder picker (folders only, same as the existing project slide picker). After selection, the button text updates to show the selected path. The default before any selection is `~` (the user's home directory).

### Enable worktree support

When checked and a git repo is selected, the session (and tab config, if saved) will include git worktree creation commands. The worktree branch name is hardcoded for now (e.g., `"my-feature-branch"`).

**Disabled state:** The checkbox is visually disabled and non-interactive when the selected directory does not contain a `.git` directory (or is not inside a git repo). A tooltip explains: "Select a git repository to enable worktree support." If the user changes the directory from a git repo to a non-git directory, the checkbox unchecks automatically and becomes disabled.

### "Get warping"

Clicking "Get warping" always saves a tab config and opens it:

1. Writes a new TOML file to `~/.warp/tab_configs/`. The file is named `startup_config.toml` (or `startup_config_1.toml`, `startup_config_2.toml`, etc. if the name is taken).
2. The TOML file contains:
   - `name = "Startup Config"` (or with a numeric suffix matching the file name).
   - A single `[[panes]]` entry with `type`, `cwd`, and optional `commands` (see "Tab config TOML generation" below).
   - If worktree is enabled, `[params]` section with `worktree_branch_name` (type `text`, default `"my-feature-branch"`).
3. Dismisses the modal.
4. Opens the newly created tab config in the current tab (replacing it), using the same flow as `open_tab_config` — which means if the config has params (worktree case), the params modal appears first.
5. If write fails, falls back to opening the tab config without persisting it.

### Tab config TOML generation

The TOML structure depends on the combination of selections:

**Terminal only (no worktree):**
```toml
name = "Startup Config"

[[panes]]
id = "main"
type = "terminal"
cwd = "/absolute/path/to/dir"
```

**CLI agent (e.g., Claude), no worktree:**
```toml
name = "Startup Config"

[[panes]]
id = "main"
type = "terminal"
cwd = "/absolute/path/to/dir"
commands = ["claude"]
```

**Terminal + worktree:**
```toml
name = "Startup Config"
title = "{{worktree_branch_name}}"

[[panes]]
id = "main"
type = "terminal"
cwd = "/absolute/path/to/dir"
worktree_name_autogenerated = true
commands = [
  "git worktree add -b {{worktree_branch_name}} ../{{worktree_branch_name}}",
  "cd ../{{worktree_branch_name}}",
]

[params.worktree_branch_name]
type = "text"
description = "New worktree branch name"
default = "my-feature-branch"
```

**CLI agent + worktree:**
```toml
name = "Startup Config"
title = "{{worktree_branch_name}}"

[[panes]]
id = "main"
type = "terminal"
cwd = "/absolute/path/to/dir"
worktree_name_autogenerated = true
commands = [
  "git worktree add -b {{worktree_branch_name}} ../{{worktree_branch_name}}",
  "cd ../{{worktree_branch_name}}",
  "claude",
]

[params.worktree_branch_name]
type = "text"
description = "New worktree branch name"
default = "my-feature-branch"
```

**Oz session:** Uses `type = "agent"` on the pane, which causes the tab to open in agent view via `PaneMode::Agent`. The `DefaultSessionMode` setting is also set to `Agent` so future new tabs default to agent view. When the feature flag for this new onboarding modal is off, the existing behavior (where selecting `AgentDrivenDevelopment` sets the mode to `Agent`) remains unchanged.
```toml
name = "Startup Config"

[[panes]]
id = "main"
type = "agent"
cwd = "/absolute/path/to/dir"
```

### Keyboard interaction

- **Enter:** Activates "Get warping" (same as clicking the button).
- **Escape:** Closes the modal without taking any action — the user lands on an empty terminal tab.
- Arrow keys / Tab: Navigate between session type pills and checkboxes.

### Dismissal

The modal can be dismissed by:
- Clicking "Get warping" (takes action).
- Pressing Escape (no action taken).
- Clicking outside the modal (no action taken).
- There is no explicit close/X button in the Figma.

### Reusability

The modal's core logic (collecting session type, directory, worktree preference, save preference) should be a self-contained view that produces a structured output (e.g., a struct with all selected values). The caller decides what to do with that output. For the onboarding flow, the caller replaces the current tab and optionally writes a tab config. This makes the modal reusable in other contexts.

## Success Criteria

1. After completing onboarding, the modal appears overlayed on the terminal.
2. Selecting "Terminal" + a directory + "Get warping" writes a tab config TOML to `~/.warp/tab_configs/` and replaces the current tab with a session in that directory.
3. Selecting a CLI agent + a directory + "Get warping" writes a tab config TOML, replaces the current tab, sets the working directory, and auto-runs the agent CLI command.
4. Selecting "Built in agent" + a directory + "Get warping" writes a tab config TOML, replaces the current tab, and opens Oz agent view in that directory.
5. The written TOML appears in the + tab menu.
6. The worktree checkbox is disabled when the selected directory is not a git repo, and enabled when it is.
7. Checking worktree + save produces a TOML with `{{worktree_branch_name}}` params and worktree commands.
8. File naming avoids collisions (appends `_1`, `_2`, etc.).
9. Escape dismisses the modal without side effects.
10. The modal does not appear again after the user completes it once (or dismisses via Escape).

## Validation

- **Unit tests:** TOML generation for each combination of session type × worktree × directory produces the expected output.
- **Manual testing:** Walk through the full onboarding flow, verify the modal appears, select each session type, toggle worktree and save, confirm correct tab behavior.
- **Tab config integration:** After saving, verify the config appears in the + menu, can be opened, and the params modal works for worktree configs.
- **UI verification:** Compare the rendered modal against the Figma mock (session type pills, directory button, checkbox, CTA button).

## Resolved Decisions

1. **Oz in tab config TOML:** Oz uses `type = "agent"` on the pane node, which maps to `PaneMode::Agent` and causes the pane to enter agent view automatically. `DefaultSessionMode` is also set to `Agent` so future new tabs default to agent view. This must not break the existing behavior when the feature flag for this modal is disabled.
2. **Worktree base branch:** The `{{branch}}` param is omitted. The `git worktree add` command uses HEAD implicitly (no base branch argument). The worktree branch name is hardcoded to `"my-feature-branch"` as a default. **TODO(moira):** Once worktree name generation is ready, replace the hardcoded `"my-feature-branch"` default with the generated name and revisit whether a base branch param should be added.
3. **Session type list:** Hardcoded to Built in agent, Claude, Codex, Gemini, Terminal (in that order). Not dynamically derived from the `CLIAgent` enum.
