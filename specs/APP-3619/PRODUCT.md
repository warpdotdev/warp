# Product Spec: Plugin Installation Fallback Modal

## Problem

When the Warp notification plugin can't be auto-installed (SSH session, or a previous install attempt failed), the user currently has no way to learn how to install it manually. We need a modal that shows step-by-step manual installation instructions.

## Current Behavior

- A green "Install Warp plugin" chip appears in the CLI agent footer when the plugin isn't installed (`agent_input_footer/mod.rs:611-633`)
- Clicking it runs auto-install via `claude plugin` CLI commands (`plugin_manager/claude.rs:29-37`)
- On failure: an error toast appears with a link to logs
- On SSH: the chip visibility has a bug (see below)

## Chip Visibility Fix (Remote Sessions)

`should_show_install_plugin_button` hides the chip when `manager.is_installed()` returns true. But `is_installed()` reads the **local** filesystem (`~/.claude/plugins/installed_plugins.json`), not the remote machine's. In any remote session (warpified SSH, legacy SSH, Docker via SSH) where Claude Code runs on the remote, this check is wrong:

- Plugin installed locally but not on remote → chip hidden, user stuck with no instructions

Fix: the `CLIAgentSession` tracks an `is_remote` flag (set at session creation from `active_session_is_local()`). When `is_remote` is true, skip the `is_installed()` check and rely solely on whether the session has an active listener. If no listener → show the chip.

## Two Chip Modes

The chip has two modes depending on context:

### Mode 1: Auto-Install (current behavior)

**When:** local session, no prior install failure for this session.

- Chip label: "Install Warp plugin"
- Chip tooltip: "Install the Warp plugin to enable rich agent notifications within Warp"
- On click: runs auto-install (existing `handle_install_plugin` flow)
- On success: chip disappears (listener registers)
- On failure: transitions to Mode 2 for the rest of the session

### Mode 2: Manual Instructions Modal

**When:** SSH session, OR auto-install previously failed in this session.

- Chip label: "Plugin install instructions"
- Chip tooltip: "View instructions to install the Warp plugin"
- Chip icon: `Icon::Info` (instead of `Icon::Download`)
- On click: opens a modal with manual installation steps

## Modal Design

### Layout

Custom modal view following the `CodexModal` pattern (centered overlay, semi-transparent backdrop, Escape to close, click-outside to dismiss via `Dismiss` element).

- Title from `PluginInstallInstructions.title` (e.g. "Install Warp Plugin for Claude Code")
- Subtitle from `PluginInstallInstructions.subtitle`
- Numbered steps, each with:
  - A short description of what the step does
  - A monospace code block rendered via `render_code_block_plain` with a copy button
- Close button (X) in the top-right corner
- Copying a command shows a "Copied to clipboard" ephemeral toast

### Content for Claude Code

These are in-session slash commands (the user is already running Claude Code).

Step 1: "Add the Warp plugin marketplace repository"
```
/plugins marketplace add warpdotdev/claude-code-warp
```

Step 2: "Install the Warp plugin"
```
/plugins install warp@claude-code-warp
```

Step 3: "Reload plugins to activate"
```
/reload-plugins
```

Subtitle: "Ensure that jq is installed on your machine. Then, run these commands inside your Claude Code session."

Auto-install success toast: "Warp plugin installed. Please run /reload-plugins to activate."

### Extensibility

Each agent provides its own modal view. Common rendering helpers (backdrop, title bar, step layout, code blocks) are shared. Adding a new agent's modal means:

- Implementing a new view that uses the shared helpers
- Returning the appropriate view from a factory function keyed on `CLIAgent`

## State Tracking

### `plugin_install_failed` on `AgentInputFooter`

A per-session boolean (scoped to the `AgentInputFooter` instance) that tracks whether auto-install has failed. Set to `true` in the `handle_install_plugin` error callback. This determines whether the chip is in Mode 1 or Mode 2 for local sessions.

Reset to `false` if the plugin activates (listener connects — which already hides the chip entirely).

### No persistence across sessions

Failure state is not persisted. A new terminal session starts fresh in Mode 1 (auto-install).

## Behavior Summary

- Plugin active (listener present) → chip hidden
- Local, plugin installed on disk → chip hidden
- Local, plugin not installed, no prior failure → chip shown, Mode 1 (auto-install)
- Local, plugin not installed, prior failure → chip shown, Mode 2 (modal)
- Remote (any SSH), no listener → chip shown, Mode 2 (modal)
- Remote (any SSH), listener present → chip hidden
- Agent has no plugin support → chip hidden
- Install in progress → chip hidden

## Edge Cases

- **User installs plugin manually mid-session (without using the chip):** The listener will connect on next `SessionStart` event, chip disappears automatically.
- **User clicks chip in Mode 2 then installs manually:** Modal stays open until dismissed. Chip disappears on next render once listener is present.
- **Multiple terminal tabs with same agent:** Each tab has its own `AgentInputFooter` with independent failure tracking. This is correct — one tab's failure shouldn't affect another.
- **Warpified SSH (tmux wrapper):** Even though the local filesystem is accessible via tmux, the agent runs on the remote machine. The `is_remote` flag is set for all SSH sessions (warpified or legacy), so Mode 2 applies to all remote sessions.
