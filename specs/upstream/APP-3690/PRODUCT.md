# Product Spec: OpenCode Plugin Install & Update Flow

Linear: [APP-3690](https://linear.app/warpdotdev/issue/APP-3690)
Figma: none provided

## Summary

Add install and update chip support for the OpenCode Warp plugin (`opencode-warp`), matching the existing Claude Code chip UX. Clicking the chip opens a modal with manual installation/update instructions.

## Problem

When a user runs OpenCode inside Warp, the Warp plugin enriches the experience with native notifications and session status tracking. Today, Warp has no way to prompt OpenCode users to install or update this plugin — `plugin_manager_for(CLIAgent::OpenCode)` returns `None`.

## Goals

- OpenCode users see install/update chips that prompt them to set up the plugin.
- Clicking the chip opens a modal with clear, copy-pasteable instructions.
- Outdated plugin versions are detected via the `SessionStart` event and users are prompted to update.

## Prerequisites

The `opencode-warp` npm package must be published before this feature ships. The name is currently available on npm.

## Non-Goals

- Auto-install (one-click install that runs commands automatically). See "Why no auto-install" below.
- Filesystem-based version detection. All version information comes from the plugin's `SessionStart` event.

## Why No Auto-Install

Claude Code has a CLI for plugin management (`claude plugin install`, `claude plugin update`), so the Claude flow shells out those commands for one-click install/update. OpenCode has no equivalent — there are no CLI commands for managing plugins.

OpenCode plugins are installed by either:
1. Adding the npm package name to the `"plugin"` array in `opencode.json` (requires JSON config editing).
2. Placing a `.js`/`.ts` file in `~/.config/opencode/plugins/` (requires downloading a file).

Both paths require either manipulating JSON config files from Rust (fragile — OpenCode supports JSONC with comments, multiple config locations, etc.) or downloading files via curl/npm from a CDN (fragile — depends on curl availability, CDN reachability, correct URL construction). Neither is as clean or reliable as Claude's dedicated CLI.

Since the install is a one-time action and the instructions are simple (add one line to a JSON file), a manual instructions modal is the right tradeoff: reliable, works everywhere (local and SSH), and avoids a class of edge cases.

## How OpenCode Plugins Work

OpenCode has two plugin loading mechanisms:

1. **npm packages** listed in `opencode.json` under the `"plugin"` array. OpenCode auto-installs these via Bun into `~/.cache/opencode/node_modules/` at startup.
2. **Local files** placed in `~/.config/opencode/plugins/` (global) or `.opencode/plugins/` (project). These are loaded directly at startup.

OpenCode has no CLI commands for plugin management (no `/plugin install`, no `/plugin update`, etc.).

**Bun caching behavior:** Once a package is installed in `node_modules`, `bun install` checks that the package exists with an appropriate version and skips re-downloading. This means npm plugins do **not** auto-update — the cached version persists until explicitly cleared.

## User Experience

### Install Chip

When a user starts an OpenCode session and the plugin has never connected (no listener, no `plugin_version` reported):

- A green chip appears: "Notifications setup instructions"
- Clicking opens a modal with manual install instructions.

**Install instructions modal:**
- Title: "Install Warp Plugin for OpenCode"
- Subtitle: "Add the Warp plugin to your OpenCode configuration, then restart OpenCode."
- Steps:
  1. "Add the plugin to your config" — copyable snippet:
     ```json
     { "plugin": ["opencode-warp"] }
     ```
     with explanatory text: "Add `"opencode-warp"` to the `plugin` array in your `opencode.json` (project root) or `~/.config/opencode/opencode.json` (global)."
  2. "Restart OpenCode to activate"

### Update Chip

When the plugin is connected but reports a version below `MINIMUM_PLUGIN_VERSION`:

- Label: "Plugin update instructions"
- Same green styling, same dismiss (X) button as install chip.
- Same chip position — install and update never appear simultaneously.

**Why updates don't happen automatically:** Bun caches installed npm packages and does not re-resolve to `latest` on subsequent startups. The cached version persists until the user explicitly clears it.

**Version detection:**
- Sole signal: `plugin_version` field from the `SessionStart` event.
- No filesystem-based version checks. If the plugin hasn't connected, we don't know its version — we show the install chip instead.
- Since the opencode-warp plugin has never been released, there is no legacy version that predates version reporting. Every installed version will report `plugin_version`.

**Update instructions modal:**
- Title: "Update Warp Plugin for OpenCode"
- Subtitle: "Clear the cached plugin and restart OpenCode to pull the latest version."
- Steps:
  1. "Remove the cached plugin" — copyable command: `rm -rf ~/.cache/opencode/node_modules/opencode-warp`
  2. "Restart OpenCode" — "OpenCode will re-download the latest version on startup."

### Chip Visibility Logic

Simplified from the Claude Code flow since there are no filesystem checks:

1. Plugin connected, version >= minimum → **no chip**
2. Plugin connected, version < minimum or not reported → **update chip**
3. No listener connected → **install chip** (unless dismissed)
4. Chip dismissed → **no chip** (install and update have independent dismiss state)
5. Notifications disabled in settings → **no chip**

Note: without filesystem checks, we cannot distinguish "not installed" from "installed but hasn't connected yet." To avoid a flicker where the install chip appears briefly before the plugin connects, the chip should be debounced — wait a few seconds after session start before showing the install chip. If the plugin connects and sends its `SessionStart` event during that window, the chip never appears.

### Dismiss Behavior

Same as Claude Code: install chip and update chip have independent dismiss state. The update chip tracks which minimum version was dismissed, so a new minimum causes it to reappear.

## Edge Cases

- **Plugin connects after brief delay:** Install chip shows momentarily, then disappears when `SessionStart` arrives. This is the expected startup race.
- **User installs via npm config path:** Plugin connects, reports version, chip disappears. Works identically.
- **User installs via local file path:** Same — plugin connects, reports version.
- **SSH sessions:** Same modal experience as local. The instructions work on any machine.
- **Multiple tabs:** All tabs share the same session state. Dismiss state is shared via the existing settings infrastructure.

## Success Criteria

1. Starting an OpenCode session in Warp when the plugin has never connected shows a green "Notifications setup instructions" chip (after a brief debounce).
2. Clicking the chip opens a modal with install instructions including a copyable config snippet.
3. After the user installs and restarts OpenCode, the plugin connects and the chip disappears.
4. When the plugin reports a version below `MINIMUM_PLUGIN_VERSION`, the "Plugin update instructions" chip appears.
5. Clicking the update chip opens a modal with cache-clear instructions.
6. Dismissing the update chip hides it for the current minimum version; a newer minimum causes reappearance.
7. Dismissing the install chip hides the install chip (independent from update dismiss).
8. The install and update chips never appear simultaneously.

## Validation

- Unit tests: `compare_versions()` (reuse from Claude module or extract to shared utility).
- Unit tests: chip visibility for each state in the chip visibility logic.
- Manual testing: full install flow on a real OpenCode session — verify instructions work, restart activates the plugin, notifications appear.
- Manual testing: update flow — verify cache clear instructions work, restart pulls new version.
- Manual testing: SSH session — verify modal appears and instructions work on the remote.
