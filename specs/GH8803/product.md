# Product Spec: User-configurable language servers

**Issue:** [warpdotdev/warp#8803](https://github.com/warpdotdev/warp/issues/8803)
**Figma:** none provided

## Summary

Let users add language servers Warp does not ship out of the box by declaring them in a settings file (binary path, arguments, file types, LSP language identifier). When the user opens a file matching a configured server's file types, Warp offers to enable that server for the workspace. Once enabled, the user-configured server **takes over** code intelligence (diagnostics, hover, goto, completions) for those file types in that workspace, replacing any built-in server that would otherwise handle them.

This is the contributor-facing alternative to baking each language server into the Warp binary. It directly addresses why PRs adding specific built-in LSPs (e.g. PHP Intelephense in #9562, JSON in #9568) were closed in favor of this product direction.

## Problem

Warp currently ships a closed set of language servers as variants of `crates/lsp/src/supported_servers::LSPServerType`. Every new language requires:

1. A new `LSPServerType` enum variant.
2. A new `LanguageServerCandidate` impl with detection, install, and `fetch_latest_server_metadata` logic.
3. A new `LanguageId` enum variant in `crates/lsp/src/config.rs` plus extension mapping.
4. A new entry in `LanguageId::lsp_language_identifier`.

This scales poorly and pulls language-specific install logic into the core. Users with niche languages (Lua, Zig, Swift, Elixir, Solidity, Bash, Tailwind, etc.) cannot add support without modifying the binary. Maintainers cannot accept PRs adding specific servers without committing to ongoing maintenance of those servers' install/version-fetch code paths.

## Goals

- A user can add a language server to Warp by editing a settings file — no binary modifications, no agent involvement.
- A user-configured server **supersedes** any built-in server for the file extensions it declares (in workspaces where it is enabled). Two servers do not run for the same file type in the same workspace.
- Configuration is portable across workspaces and discoverable in Warp's existing Settings UI.
- Enablement is per-workspace by default with a clear opt-in moment, not silent global activation.
- Mis-configuration produces a visible, actionable error — not a silent disabled state.

## Non-goals

- **Auto-installation of user-configured servers.** The user installs the binary themselves (npm, cargo, system package manager, brew, etc.). Warp does not run package managers on the user's behalf for these.
- **Version pinning / auto-update of user-configured servers.** Out of scope; the user owns the lifecycle.
- **Per-file dynamic switching.** Within a workspace, a single file extension is associated with at most one user-configured server. No "try server A, fall back to server B" runtime logic.
- **Coexisting with a built-in server on the same file extension.** When a user-configured server is enabled in workspace W and matches file type X, the built-in server that would otherwise serve X in W is **not** spawned for W (per @kevinyang372's review of the original draft — running both is undesirable).
- **Cross-workspace global enablement on first open.** A configured server is *defined* globally but *enabled* per-workspace.
- **Marketplace / discovery of community configs.** Out of scope; users find configs themselves.
- **Per-server initialization options (V0).** `initializationOptions` forwarding is deferred to a follow-up; warp does not yet have settings-file shape for arbitrary nested config payloads.

## User experience

### Adding a server

1. User edits their Warp settings file (TOML, located at the standard Warp settings path) and adds an `[[lsp.servers]]` entry. (See the "Configuration shape" section below.)
2. Warp detects the new entry on settings reload. No restart required.
3. If the entry is malformed (missing `name`, missing `command`, empty `file_types`, missing `language_id`, duplicate `name`, or duplicate `file_types` across user-configured entries), Warp shows a non-blocking notification: *"Custom LSP `<name>` is misconfigured: <reason>. See settings."* with a button to open the settings file at the offending line.

### First time opening a matching file

1. User opens a file whose extension matches a configured server's `file_types` (e.g. opens `foo.lua` with a Lua server configured).
2. Warp detects the configured server is *defined* but not yet *enabled* for this workspace.
3. The editor footer shows a chip: *"Enable `<name>` for this workspace?"* with `Enable` / `Dismiss` buttons.
4. If `Enable` is pressed, Warp:
   - Records the per-workspace enablement.
   - Suppresses any built-in server that would otherwise serve `file_types` for this workspace (shutting it down for this workspace if currently running).
   - Spawns the user's server process with the configured command and args.
   - Starts driving LSP traffic for files matching `file_types` in this workspace.
5. If `Dismiss` is pressed, the chip is suppressed for this workspace until the user re-enables it from Settings UI. The built-in server (if any) continues to handle the file type.

### Subsequent opens in an enabled workspace

1. User opens any file matching `file_types` in a workspace where the server is already enabled.
2. The user-configured server is already running; no UI surfaces. Diagnostics, hover, goto, completions all behave as they do for built-in servers.

### Disabling, re-enabling, and managing servers in a workspace

The disable / re-enable flow piggybacks on Warp's existing **Settings → Code → Indexing and Projects** UI pattern (per @kevinyang372's direction; no new "Code intelligence" section is introduced for V0):

1. User opens **Settings → Code**. The existing per-workspace project list grows a sub-row per user-configured LSP that has any per-workspace state in this workspace (enabled, dismissed, or pending re-confirmation after a `command` change).
2. The row shows the server's name and current state, with a single context-appropriate action:
   - **Enabled:** `Disable` button. Shuts down the server's process for this workspace within 1s, removes the per-workspace enablement record **and any dismissal record for the same server**, and re-enables the built-in server for that file type if one exists. Clearing both records ensures the chip can re-appear on the next matching file open.
   - **Dismissed:** `Re-enable` button. Clears the dismissal so the chip surfaces on the next matching file open. Does not start the server immediately — the user re-enters the chip flow.
   - **Pending re-confirmation:** `Re-enable` button. Records the new (changed) `command` as confirmed by the user and starts the server. This is the same affordance the chip offers for the security-driven re-confirmation flow described below.

### Misconfiguration scenarios

1. **Binary not on PATH:** When a user enables a server whose `command[0]` is not on PATH, Warp shows: *"Could not start `<name>`: binary `<cmd>` not found on PATH."* The chip's `Enable` button is replaced with `Open settings`.
2. **Binary on PATH but spawn fails:** Warp shows: *"`<name>` exited with status `<n>`. Last 200 bytes of stderr: `<...>`."* with `Open settings` and `Retry` buttons.
3. **Spawn or `initialize` hangs:** Warp bounds the LSP `initialize` request with a fixed default timeout (5s, not user-configurable in V0 per @kevinyang372). On timeout, Warp shows: *"`<name>` did not respond to `initialize` within 5s."* The server's process is killed and reaped.

### Command/args change after enablement (security path)

If the user edits the `command` or `args` of an already-enabled server, Warp does **not** silently respawn the new binary. Instead:

1. The currently running process is shut down cleanly (`shutdown` → `exit` → kill if needed).
2. The server's enablement is moved to a "needs re-confirmation" state.
3. On the next file open matching `file_types`, the chip reappears: *"`<name>` command changed — re-enable?"*. The user must explicitly re-confirm before the new binary runs.

This protects users whose settings are edited by another tool (sync, IDE, malicious process) from silently executing a different binary in an already-trusted workspace.

## Configuration shape

The user's Warp settings TOML grows a new `[lsp]` table. Multiple servers via array-of-tables `[[lsp.servers]]`:

```toml
[[lsp.servers]]
name = "intelephense"
command = ["intelephense", "--stdio"]
file_types = ["php", "phtml"]
language_id = "php"

[[lsp.servers]]
name = "bash-language-server"
command = ["bash-language-server", "start"]
file_types = ["sh", "bash"]
language_id = "shellscript"
```

| Field | Required | Type | Notes |
|---|---|---|---|
| `name` | yes | string | Display name. Must be unique across all configured servers. |
| `command` | yes | array of strings | First element is the binary; rest are args. Resolved against PATH. Must be non-empty. |
| `file_types` | yes | array of strings | File extensions (no dot) the server handles. Must be non-empty. **No two configured servers may declare overlapping `file_types`** — Warp rejects the settings on parse if they do. |
| `language_id` | yes | string | The LSP `languageId` Warp will send in `textDocument/didOpen.languageId` for files matching `file_types`. Required because file extension is not a reliable proxy (`sh` → `shellscript`, `phtml` → `php`). |

Settings reload re-reads the entire `[lsp]` table; servers whose `command` or `args` changed enter the re-confirmation flow described above. Removed entries shut down. Added entries become available (but are not auto-enabled). Pure metadata changes (e.g. `language_id` only, no `command` change) restart the process in place without re-confirmation.

**Out of V0 (deferred to follow-ups):** `root_files` (Warp's existing root-repo detection is used), `initialization_options`, `start_timeout_ms` (fixed default in code), `~`/`$VAR` expansion in `command[0]`.

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. A `[[lsp.servers]]` entry with `name`, `command` (non-empty array), `file_types` (non-empty array), and `language_id` (non-empty string) is accepted at settings parse time.
2. A `[[lsp.servers]]` entry missing any required field, with empty `command` / `file_types`, with a duplicate `name`, or whose `file_types` overlap with another configured entry's `file_types`, is rejected at parse time and surfaces a settings-error notification with the offending line range.
3. Opening a file whose extension is in `file_types` of a configured-but-not-enabled server shows the "Enable" chip in the editor footer for that file's workspace exactly once per (server, workspace) pair until the user dismisses or enables.
4. Pressing "Enable" on the chip starts a server process via `command[0]` with `command[1..]` as args, sends an LSP `initialize` request, and on receiving a successful response begins routing LSP traffic for files matching `file_types` in that workspace.
5. If `command[0]` is not on PATH at enablement time, no process is spawned and the user sees an error notification with `Open settings` action.
6. If the spawned process exits non-zero before sending an `initialize` response, the user sees an error notification with the exit status and last 200 bytes of stderr.
7. If `initialize` does not return within the fixed 5s timeout, the spawned process is explicitly killed and reaped (`kill()` + `wait()` semantics, not Drop alone), the LSP client is torn down cleanly, and the user sees a timeout notification.
8. After a server is enabled in workspace W, opening any file matching `file_types` in W routes LSP requests to that server **without** showing the chip again, and **without** a built-in server for the same file types running concurrently in W.
9. Enabling a user-configured server in workspace W that matches a file type also handled by a built-in shuts down the built-in for W within 1s; disabling the user-configured server restores the built-in for W within 1s.
10. After settings change to a server entry's metadata fields only (e.g. `language_id`, `file_types` ordering), an enabled-and-running server is restarted with the new metadata within 1s of settings reload, with no Warp restart and no re-confirmation prompt required.
11. After settings change to an enabled server's `command` or args, the process is shut down, enablement moves to "needs re-confirmation", and the "command changed — re-enable?" chip reappears on next matching file open until the user re-confirms. This chip surfaces **even if the server was previously dismissed in that workspace**: a `command` change is a security signal and overrides dismissal.
12. Disabling a server via Settings UI shuts down its process for the targeted workspace within 1s, removes the per-workspace enablement record, restores the built-in for the file type if any, and the chip reappears on next file open.
13. Restarting Warp preserves per-workspace enablement state — workspaces where a server was enabled before restart auto-spawn the user-configured server on the next file open without the chip reappearing, and the built-in for the same file types is not spawned.
14. The chip is **not** shown for a configured server in a workspace that has explicitly dismissed it; the user must re-enable from Settings UI.
15. The `languageId` field in `textDocument/didOpen` for any file routed to a user-configured server equals that server's configured `language_id`, regardless of file extension.
16. Shutting down the LSP system (e.g. on Warp quit) sends `shutdown` then `exit` to all running custom servers and waits up to 1s for graceful exit before issuing an explicit `kill()` + `wait()`.

## Open questions

- **Should we ship example configs?** A `docs/custom-lsp-examples.md` with intelephense / lua-language-server / zls / bash-language-server entries would shorten time-to-first-success. Recommend yes; not part of the core feature gate, but in scope for the same release.
- **Settings UI placement specifics.** This spec says "follow Settings → Code → Indexing and Projects pattern." Confirming the exact row layout (one row per server with workspace sub-rows, vs one row per (server, workspace) tuple) is an implementation-time decision aligned with the existing pattern.
