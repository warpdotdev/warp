# Product Spec: Gemini CLI Plugin Install & Update Flow

Linear: [APP-4067](https://linear.app/warpdotdev/issue/APP-4067/enable-gemini-notifications)
Figma: none provided

## Summary

Add install and update chip support for the Gemini CLI Warp plugin (`gemini-warp`), matching the existing Claude Code chip UX. Enable the listener so Warp processes structured notifications from Gemini sessions (session status, permission requests, task completion).

## Problem

The `gemini-warp` extension already exists and uses the same structured OSC 777 notification protocol as Claude Code (`warp://cli-agent` title, JSON body with `agent: "gemini"`). However, Warp currently ignores these notifications because:

1. `create_handler` in the listener returns `None` for `CLIAgent::Gemini`.
2. `plugin_manager_for` returns `None` for `CLIAgent::Gemini` — there's no install/update chip.
3. There is no `GeminiNotifications` feature flag to gate the rollout.

## Goals

- Gemini users see install/update chips that prompt them to set up the plugin.
- Clicking the chip auto-installs or auto-updates the plugin (Gemini CLI has first-class extension management commands).
- A manual instructions fallback is available for cases where auto-install fails.
- Structured notifications from the Gemini plugin are received and displayed (session status, blocked/success states, permission requests).

## Non-Goals

- Changing the gemini-warp plugin itself (it's maintained separately in `warpdotdev/gemini-cli-warp`).
- Platform plugin / Oz harness support for Gemini (future work).

## How Gemini CLI Extensions Work

Gemini CLI has a first-class extension system:

- **Install**: `gemini extensions install <github-url-or-local-path>` — copies the extension into `~/.gemini/extensions/<name>/`.
- **Update**: `gemini extensions update <name>` — pulls the latest version from the source.
- **Uninstall**: `gemini extensions uninstall <name>`.
- **Manifest**: Each extension has a `gemini-extension.json` with `name`, `version`, `description`, and hook/MCP/command definitions.
- **Hooks**: Defined in `hooks/hooks.json` within the extension directory (not in the manifest). The gemini-warp plugin uses `SessionStart`, `AfterAgent`, `Notification`, `BeforeAgent`, and `AfterTool` hooks.

Key difference from Claude Code: Gemini extensions install from a GitHub URL directly (no separate marketplace add step). The install and update are each a single CLI command.

Key difference from OpenCode: Gemini CLI *does* have CLI commands for extension management, so auto-install and auto-update are viable.

## User Experience

### Install Chip

When a user starts a Gemini session and the plugin has never connected (no listener, no `plugin_version` reported):

- A green chip appears: "Notifications setup instructions"
- Clicking triggers auto-install via `gemini extensions install https://github.com/warpdotdev/gemini-cli-warp --consent`.
  - `--consent` skips the interactive confirmation prompt that would block the non-interactive shell execution.
- On success, a toast confirms installation and tells the user to restart Gemini.
- On failure, a toast appears with an error, and the user can click the info (ⓘ) button to open manual instructions in a split pane.

**Manual install instructions (split pane):**
- Title: "Install Warp Plugin for Gemini CLI"
- Subtitle: "Run the following command, then restart Gemini CLI."
- Steps:
  1. "Install the Warp extension" — command: `gemini extensions install https://github.com/warpdotdev/gemini-cli-warp --consent`
- Post-install notes: "Restart Gemini CLI to activate the plugin."

### Update Chip

When the plugin is connected but reports a version below `MINIMUM_PLUGIN_VERSION`:

- Label: "Plugin update available"
- Clicking triggers auto-update via `gemini extensions update gemini-warp`.
  - The extension name is `gemini-warp` (the installed directory name under `~/.gemini/extensions/`).
- On success, a toast confirms the update and tells the user to restart Gemini.
- On failure, a toast appears with an error, and the user can click the info (ⓘ) button to open manual instructions in a split pane.

**Manual update instructions (split pane):**
- Title: "Update Warp Plugin for Gemini CLI"
- Subtitle: "Run the following command, then restart Gemini CLI."
- Steps:
  1. "Update the Warp extension" — command: `gemini extensions update gemini-warp`
- Post-install notes: "Restart Gemini CLI to activate the update."

### Version Detection

Two signals, matching the Claude Code pattern:

1. **Filesystem check**: `~/.gemini/extensions/gemini-warp/gemini-extension.json` — read the `version` field to determine if the plugin is installed and whether it's outdated. Symlinks are followed (supports `gemini extensions link`).
2. **Runtime signal**: `plugin_version` field from the `SessionStart` event (emitted by `on-session-start.sh` in the plugin).

The filesystem check eliminates install chip flicker (the chip never appears if the plugin is already on disk) and enables update detection before the plugin connects.

### Notification Listening

Once the plugin is installed, the Gemini session handler works identically to Claude Code / OpenCode: the `DefaultSessionListener` receives structured JSON events and forwards them to the sessions model. No special parsing is needed — the plugin already speaks the same protocol.

### Chip Visibility Logic

Same as Claude Code:

1. Plugin connected, version >= minimum → **no chip**
2. Plugin connected, version < minimum → **update chip**
3. No listener, plugin installed on disk with version >= minimum → **no chip** (wait for connection)
4. No listener, plugin installed on disk with version < minimum → **update chip**
5. No listener, plugin not installed on disk → **install chip** (unless dismissed)
6. Chip dismissed → **no chip** (install and update have independent dismiss state)
7. Notifications disabled in settings → **no chip**

### Dismiss Behavior

Same as Claude Code / OpenCode: install chip and update chip have independent dismiss state. The update chip tracks which minimum version was dismissed; a new minimum causes it to reappear.

## Edge Cases

- **Plugin installed via `gemini extensions link` (symlink):** Filesystem check follows symlinks, so linked extensions are detected as installed. Version is read from the symlinked `gemini-extension.json`.
- **`gemini` CLI not on PATH:** Auto-install fails; user sees error toast and can use the manual instructions pane. The manual instructions pane commands will also need `gemini` on PATH, but the user can adapt (e.g. use full path or install Gemini CLI).
- **Extension already installed:** `gemini extensions install` may fail or warn. This is fine — the user already has the plugin.
- **SSH sessions:** Same experience as local. The `gemini` CLI must be available on the remote host. Filesystem checks don't apply to remote sessions (same as Claude Code).

## Success Criteria

1. Starting a Gemini CLI session in Warp when the plugin is not installed shows a green install chip.
2. Clicking the install chip runs `gemini extensions install` and shows a success/failure toast.
3. After restart, the plugin connects, notifications appear in the inbox, and the chip disappears.
4. When the plugin reports a version below `MINIMUM_PLUGIN_VERSION`, an update chip appears.
5. Clicking the update chip runs `gemini extensions update gemini-warp` and shows a success/failure toast.
6. The info (ⓘ) button opens a split pane with manual instructions (not a modal).
7. Dismissing the chips works identically to other agents (independent dismiss state, version-tracked update dismiss).
8. Gemini notifications (stop, blocked, permission request) surface in the agent inbox when HOANotifications is enabled.
9. The install/update chip is gated behind both `HOANotifications` and `GeminiNotifications` feature flags. Notification listening only requires `HOANotifications` (consistent with other agents).

## Validation

- Unit tests: `GeminiPluginManager` trait implementations (minimum version, can_auto_install, install/update instructions).
- Unit tests: `is_installed()` / `needs_update()` filesystem detection with temp directories.
- Unit tests: `is_agent_supported` returns `true` for `CLIAgent::Gemini` when feature flags are enabled.
- Unit tests: `plugin_manager_for(CLIAgent::Gemini)` returns `Some(...)` when flags are enabled.
- Manual testing: full install flow with a real Gemini CLI session.
- Manual testing: update flow — verify `gemini extensions update` pulls new version.
- Manual testing: notification flow — verify stop, blocked, permission request notifications appear.

