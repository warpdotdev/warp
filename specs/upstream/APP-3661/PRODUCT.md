# Product Spec: Plugin Update Flow

## Problem

We're releasing a new version of the Warp notification plugin for Claude Code (v2.0.0). Existing users on v1.1.0 won't automatically receive the update because Claude Code's plugin update system is unreliable. We need Warp to detect outdated plugin versions and prompt users to update.

## Current Chip Behavior

Today, a green chip appears in the CLI agent footer when the plugin isn't installed:

- Local session → chip auto-installs on click
- SSH session or prior install failure → chip opens a modal with manual steps
- Plugin is active (connected) or installed → chip is hidden
- User dismissed the chip → chip is hidden

There is no concept of an "outdated" plugin. A user on v1.1.0 will never see a prompt to update.

## New Behavior: Update Chip

When the plugin is installed but on an old version, show an update chip:

- Label: "Update Warp plugin"
- Tooltip: "A new version of the Warp plugin is available"
- Same green styling and dismiss (X) button as the install chip
- On local sessions: clicking runs the update automatically
- On SSH or after a failed auto-update: clicking opens a modal with manual update steps

The update chip replaces the install chip in the same position — they never appear simultaneously.

## How Version Detection Works

The plugin reports its own version via a `plugin_version` field in the `SessionStart` event when it connects. Warp compares this against a minimum required version. This works identically for local and remote sessions — no filesystem check needed for update detection.

A missing `plugin_version` (from a plugin that predates version reporting) is treated as outdated.

## When Each Chip Appears

1. Plugin connected, version >= minimum → **no chip**
2. Plugin connected, version < minimum or not reported → **update chip** (new)
3. Plugin not connected, not installed locally → **install chip** (existing behavior)
4. Plugin not connected, installed locally, on-disk version outdated → **update chip** (filesystem fallback for plugins too old to send structured events)
5. Plugin not connected, installed locally, on-disk version current → **no chip** (waiting for connection)
6. Remote session, no listener → **install chip** (can't check filesystem remotely)
7. Just completed install/update → **no chip** (assume current version until next `SessionStart`)
8. Chip dismissed for this version → **no chip**

## Auto-Update (Local Sessions)

On click, Warp runs `marketplace add` (to refresh the local clone) + `plugin update` via the CLI. Same UX pattern as auto-install: persistent toast while running, success/failure toast on completion. A post-update sanity check verifies the on-disk version actually changed.

On success: "Warp plugin updated. Please run /reload-plugins to activate."
On failure: transition to manual mode (modal) for the rest of the session.

## Manual Update (SSH / Failed Auto-Update)

Opens a modal with step-by-step update instructions. Unlike the install modal (which uses in-session `/plugin` slash commands), the update modal uses CLI commands (`claude plugin ...`) because there is no working in-session slash command for updating plugins. Users are instructed to run the commands in a separate terminal, or inside Claude Code by typing `!` before each command.

## Dismiss Behavior

The install chip and update chip have **independent** dismiss state:

- Dismissing the install chip hides the install chip (existing boolean behavior, unchanged)
- Dismissing the update chip hides the update chip for the current minimum version
- If we later release a newer version (e.g., v3.0.0), the update chip reappears

This means tracking *which version* was dismissed for the update chip, not just whether it was dismissed.

## Edge Cases

- **User updates manually in Claude Code:** The listener reconnects with a new `plugin_version`, chip disappears automatically.
- **Plugin doesn't report version:** Treated as outdated — these are pre-versioning builds that definitely need an update.
- **Plugin too old to send structured events:** Falls back to on-disk version check. If the on-disk version is below minimum, the update chip appears even without a listener.
- **Just completed install/update (mid-session):** The session's `plugin_version` is set to `MINIMUM_PLUGIN_VERSION` to suppress the update chip until the user runs `/reload-plugins` and the plugin sends a real `SessionStart`.
- **Multiple tabs:** All tabs see the same session state. Update failure tracking is shared across tabs.
- **Plugin connected over SSH:** Version detection works the same way — the plugin reports its own version regardless of where it's running.
- **Old plugin over SSH (pre-structured-events):** The old public plugin (v1.1.0) doesn't send structured events, so no listener connects and we can't check the remote filesystem. The install chip shows instead of the update chip. This is functionally correct — the install instructions work to upgrade — but the label says "install" rather than "update".
