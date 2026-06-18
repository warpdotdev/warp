# APP-3736: Allow specifying tab-close behavior in tab configs

Linear: [APP-3736](https://linear.app/warpdotdev/issue/APP-3736/allow-for-specifying-closing-of-tab-behavior)

## Summary

Add an optional top-level close hook to tab configs so a tab instance opened from a config can run best-effort cleanup behavior when the tab is explicitly closed. The primary use case is worktree cleanup: deleting the worktree directory on close, with an optional second step that also deletes the worktree branch.

## Problem

Tab configs can already create a worktree when a tab opens, but they cannot declare what should happen when that tab is later closed. Users who use tab configs as ephemeral worktree environments have to remember to manually clean up the worktree directory and branch. The current template and bundled `tab-configs` skill teach creation-time worktree commands, but not cleanup.

## Goals

- Let tab config authors opt into tab-close behavior on a per-config basis.
- Make worktree cleanup a first-class documented tab-config pattern.
- Support both cleanup variants:
  - remove the worktree only
  - remove the worktree and delete the branch
- Use the actual resolved values for the specific tab instance at close time, including params and any auto-generated worktree branch name.
- Update the default template comments and bundled `tab-configs` skill examples to show the new close behavior.

## Non-goals

- Per-pane close hooks.
- Guaranteeing cleanup when Warp crashes, is force-quit, or loses power.
- Guaranteeing cleanup after app restore or undo-close. Close behavior is resolved for the live tab instance when the tab opens and is not persisted into workspace snapshots.
- Managing worktrees created outside the tab config flow.
- New confirmation UI or modal flow for close cleanup.
- Automatically inferring cleanup commands from open-time commands; config authors must declare close behavior explicitly.

## Figma / design references

Figma: none provided.

## User experience

### Authoring the config

Tab configs may optionally include a top-level `[on_close]` table.

- `on_close` applies to the tab as a whole, not to individual panes.
- `[on_close]` may specify:
  - `directory` (optional): working directory used for close-time commands.
  - `commands` (required when `[on_close]` is present): ordered shell commands to run when the tab closes.
- `directory` and `commands` support the same template variables that tab configs already support for `title`, pane `directory`, and pane `commands`.
- Close-time template expansion uses the values that were used when the tab instance was opened. Warp must not re-prompt for params when the tab closes.
- Template rendering follows the same quoting rules as open-time config rendering: `directory` receives unquoted values so paths remain valid, and `commands` receive shell-quoted values.

### Triggering close behavior

- Close behavior runs when the user explicitly closes a tab instance opened from a tab config, such as from the close button, tab context menu, or keyboard shortcut.
- If a config does not define `[on_close]`, closing behaves exactly as it does today.
- Close behavior runs once per closing tab instance, even if the tab contains multiple panes.
- There is no additional confirmation prompt at close time.
- Close cleanup starts asynchronously during tab close, and the tab is removed immediately without waiting for cleanup commands to finish.

### Command execution semantics

- Warp runs `on_close.commands` in order.
- If a command fails, Warp still closes the tab. Failures are best-effort and do not block close.
- If a command fails, Warp stops running the remaining close commands for that tab instance, logs the failure, and shows a persistent non-blocking error toast.
- If Warp cannot access local shell state for cleanup, close commands are skipped, the tab still closes, and Warp shows a persistent non-blocking error toast.
- For worktree configs, the common pattern is to set `on_close.directory` to the repo root so cleanup does not depend on the tab's live shell state.

### Worktree example: remove the worktree, keep the branch

```toml
name = "New Worktree"
title = "{{worktree_branch_name}}"

[[panes]]
id = "main"
type = "terminal"
directory = "{{repo}}"
commands = [
  "git worktree add -b {{worktree_branch_name}} ../{{worktree_branch_name}} {{branch}}",
  "cd ../{{worktree_branch_name}}",
]

[on_close]
directory = "{{repo}}"
commands = [
  "git worktree remove ../{{worktree_branch_name}}",
]
```

Closing a tab opened from this config removes the worktree directory but leaves the branch in place.

### Worktree example: remove the worktree and delete the branch

```toml
name = "Ephemeral Worktree"
title = "{{worktree_branch_name}}"

[[panes]]
id = "main"
type = "terminal"
directory = "{{repo}}"
commands = [
  "git worktree add -b {{worktree_branch_name}} ../{{worktree_branch_name}} {{branch}}",
  "cd ../{{worktree_branch_name}}",
]

[on_close]
directory = "{{repo}}"
commands = [
  "git worktree remove ../{{worktree_branch_name}}",
  "git branch -D {{worktree_branch_name}}",
]
```

Closing a tab opened from this config first removes the worktree and then deletes the associated branch.

### Autogenerated worktree names

If a worktree config relies on an auto-generated branch name, close behavior uses the resolved branch name for that tab instance. Authors can reference that runtime value in `[on_close]` with `{{autogenerated_branch_name}}`, matching the open-time placeholder used by autogenerate worktree configs.

### Template and skill updates

- `app/resources/tab_configs/new_tab_config_template.toml` updates its commented worktree example to show close cleanup.
- `resources/bundled/skills/tab-configs/SKILL.md` updates its schema reference and examples to include `[on_close]`.
- Both documentation surfaces should show both worktree variants:
  - remove the worktree only
  - remove the worktree and delete the branch
- The branch-deleting example must clearly read as destructive and opt-in.

## Success criteria

1. A tab config without `[on_close]` closes with no behavior change from today.
2. A tab config with `[on_close]` runs its close commands once when the user explicitly closes the tab.
3. Close-time commands use the resolved values from the tab instance that is closing; Warp does not reopen the param modal.
4. A worktree config can remove only the worktree on close while leaving the branch untouched.
5. A worktree config can remove the worktree and then delete the branch on close.
6. If the worktree removal step fails, Warp still closes the tab immediately, shows a persistent error toast, logs the cleanup failure, and does not run later branch-deletion commands.
7. The default tab config template includes a commented example of close cleanup for worktrees.
8. The bundled `tab-configs` skill documentation describes `[on_close]` and includes both worktree cleanup variants.

## Validation

- Unit tests for parsing `[on_close]`, rendering its templated `directory` and `commands`, and preserving per-tab resolved values.
- Unit or integration tests that verify close commands run once per tab close, stop after the first failure, and surface a persistent error toast.
- Manual verification with a real git repo:
  - open a config that creates a worktree and removes only the worktree on close
  - open a config that creates a worktree and removes both worktree and branch on close
  - verify the correct repo, worktree, and branch state after each close
- Manual verification that tabs still close immediately if a cleanup command fails, and that the failure is shown in a persistent toast and logged rather than blocking tab close.
- Manual verification that close cleanup is skipped gracefully if local shell state is unavailable and that a persistent toast is shown.
- Manual verification that the updated template and bundled skill examples are internally consistent and produce valid TOML.

## Open questions

(None outstanding.)
