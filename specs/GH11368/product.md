# Product Spec: Support 'agy' (Antigravity) CLI Agent in Warp

**Issue:** [warpdotdev/warp#11368](https://github.com/warpdotdev/warp/issues/11368)
**Figma:** none provided

## Summary

Add native support for the `agy` (Antigravity) CLI agent in Warp. This enables the terminal to automatically identify `agy` command executions, transition the active pane into "Agent Mode" (with dedicated layouts, toolbars, and branding), and support one-click installation and update flows for the `agy-warp` plugin. Warp will listen to and display structured OSC 777 session notifications (session status, permission requests, tool output, and task completion).

## Problem

The Antigravity CLI agent (`agy`) is an autonomous developer tool that performs code editing, terminal commands, and workspace analysis. While it is designed to communicate with the terminal using the standard `warp://cli-agent` OSC 777 protocol, Warp currently lacks native support for it:

1. Running `agy` does not trigger the terminal's Agent Mode layout or toolbar.
2. Warp does not listen to or parse structured notifications from `agy` sessions.
3. No install/update chip or setup instructions are available to help the user configure the `agy-warp` integration plugin.
4. No feature flag exists to gate or control `agy` integrations.

## Goals

- Native command detection: typing or running `agy` triggers Agent Mode immediately.
- Custom branding: render the dedicated Antigravity toolbar with custom colors (Indigo `#6366F1`) and a custom logo.
- Auto-install and auto-update: provide inline terminal chips that automatically configure or update the `agy-warp` plugin.
- Fallback instructions: offer a split pane with manual setup commands if auto-installation fails.
- Notification streams: process and display structured notifications (e.g. blocked, success, permission requests) in the agent inbox.

## Non-Goals

- Hosting or modifying the `agy-warp` plugin codebase (maintained in a separate repository, e.g., `warpdotdev/agy-warp`).
- Implementing remote execution environment / Oz harness support for `agy` (future milestone).

## How agy CLI Extensions Work

The Antigravity CLI agent supports an extension ecosystem:

- **Install Command**: `agy extensions install <github-url>` — clones or downloads the extension to `~/.antigravitycli/extensions/<name>/`.
- **Update Command**: `agy extensions update <name>` — updates the extension to the latest upstream release.
- **Manifest**: Located at `~/.antigravitycli/extensions/<name>/agy-extension.json`. Contains metadata fields like `version` and `description`.
- **Hooks**: Triggers hook scripts (e.g. `SessionStart`, `AfterAgent`, `Notification`) which emit the OSC 777 `warp://cli-agent` sequences.

## User Experience

### Install Chip

If a user launches an `agy` session and the extension is not found on disk:
- A green chip appears in the input footer: "Notifications setup instructions".
- Clicking the chip triggers auto-installation via:
  ```bash
  agy extensions install https://github.com/warpdotdev/agy-warp
  ```
- On success, a toast is shown: "Warp plugin installed. Please restart the session to activate."
- On failure, an error toast is shown, and the user can click (ⓘ) to open manual instructions in a split pane.

**Manual Instructions (split pane):**
- Title: "Install Warp Plugin for Antigravity"
- Subtitle: "Run the following command, then restart Antigravity."
- Steps:
  1. "Install the Warp extension" — command: `agy extensions install https://github.com/warpdotdev/agy-warp`
- Post-install notes: "Restart the session to activate the plugin."

### Update Chip

When the plugin is active but its version is below `MINIMUM_PLUGIN_VERSION`:
- A chip appears: "Plugin update available".
- Clicking triggers auto-updates via:
  ```bash
  agy extensions update agy-warp
  ```
- On success, a toast is shown: "Warp plugin updated. Please restart the session to activate."
- On failure, an error toast is shown, and the user can click (ⓘ) to open manual update instructions.

### Version Detection

Determined by:
1. **Filesystem check**: Read the `version` field from `~/.antigravitycli/extensions/agy-warp/agy-extension.json`.
2. **Runtime notification**: Check the `plugin_version` payload in the `SessionStart` OSC 777 event.

### Chip Visibility Logic

Consistent with other agents (Claude Code, Gemini CLI):
1. Connected + version >= minimum → **no chip**
2. Connected + version < minimum → **update chip**
3. Not connected + installed on disk + version >= minimum → **no chip** (wait for connect)
4. Not connected + installed on disk + version < minimum → **update chip**
5. Not connected + not installed on disk → **install chip**
6. Notifications disabled in settings → **no chip**

## Success Criteria

1. Running `agy` in Warp launches Agent Mode and styles the pane with Antigravity branding.
2. Green install chips appear on the first launch if the plugin is missing.
3. Clicking the install chip runs the setup command and resolves the setup state.
4. Outdated plugin manifests prompt an update chip.
5. Setup instructions display correctly in a split pane.
6. Structured events from the `agy` session are handled and displayed in the terminal UI.

## Validation

- **Unit tests**: Validate filesystem verification (`is_installed`, `needs_update`) using temporary directory structures.
- **Unit tests**: Test CLI detection, brand styling, and plugin instructions.
- **Manual verification**: Verify the end-to-end setup and notifications flow inside a live terminal window.
