# Tech Spec: Gemini CLI Plugin Install & Update Flow

See `specs/APP-4067/PRODUCT.md` for the product spec.

## 1. Problem

`plugin_manager_for(CLIAgent::Gemini)` returns `None` (`plugin_manager/mod.rs:189`), and `create_handler` in the listener also returns `None` for `CLIAgent::Gemini` (`listener/mod.rs:53`). This means Gemini sessions get no install/update chip and their structured OSC 777 notifications are silently dropped.

Gemini CLI has first-class `gemini extensions install/update` commands, so the implementation follows the Claude Code pattern: auto-install, auto-update, and filesystem-based version detection.

## 2. Relevant Code

- `plugin_manager/mod.rs` — `CliAgentPluginManager` trait, `plugin_manager_for()` / `plugin_manager_for_with_shell()` factory, `compare_versions`
- `plugin_manager/claude.rs` — reference auto-install implementation with filesystem detection and `LocalCommandExecutor`
- `plugin_manager/opencode.rs` — reference manual-only implementation (simpler, no filesystem checks)
- `listener/mod.rs:41-61` — `is_agent_supported()` and `create_handler()` — must add `CLIAgent::Gemini`
- `agent_input_footer/mod.rs (793-873)` — `plugin_chip_kind()` determines which chip to show
- `agent_input_footer/mod.rs (877-893)` — `should_use_manual_mode()`
- `agent_input_footer/mod.rs (925+)` — `handle_plugin_operation()` shared async handler
- `crates/warp_features/src/lib.rs:760-769` — existing `HOANotifications`, `OpenCodeNotifications`, `CodexNotifications` flags

## 3. Current State

The plugin manager infrastructure is fully generalized across Claude (auto-install + filesystem detection), OpenCode (manual-only), and Codex (manual-only, no update support). Adding a new auto-install agent requires:

1. A new struct implementing `CliAgentPluginManager` with `can_auto_install() == true`.
2. `is_installed()` and `needs_update()` filesystem checks.
3. `install()` and `update()` methods that shell out CLI commands via `LocalCommandExecutor`.
4. Wiring into `plugin_manager_for_with_shell()`.
5. Adding the agent to `create_handler()` and `is_agent_supported()` in the listener.
6. A feature flag to gate the rollout.

## 4. Proposed Changes

### 4a. Feature Flag (`crates/warp_features/src/lib.rs`)

Add `GeminiNotifications` to the `FeatureFlag` enum after `CodexNotifications`:

```rust
/// Enables the install/update chip for the Gemini CLI Warp extension.
/// Requires HOANotifications to also be enabled.
GeminiNotifications,
```

Add to `DOGFOOD_FLAGS`.

### 4b. Gemini Plugin Manager (`plugin_manager/gemini.rs`)

New file. Follows the Claude Code pattern closely.

**Constants:**
- `EXTENSION_REPO: &str = "https://github.com/warpdotdev/gemini-cli-warp"` — install source.
- `EXTENSION_NAME: &str = "gemini-warp"` — the installed directory name under `~/.gemini/extensions/`. Used for `gemini extensions update gemini-warp`.
- `MINIMUM_PLUGIN_VERSION: &str = "1.0.0"` — matches current plugin version.

**Struct:**
```rust
pub(super) struct GeminiPluginManager {
    executor: LocalCommandExecutor,
    path_env_var: Option<String>,
}
```

Same `new(shell_path, shell_type, path_env_var)` constructor pattern as `ClaudeCodePluginManager`.

**Filesystem detection:**

`gemini_extensions_dir()` — returns `~/.gemini/extensions` (no env var override like Claude's `CLAUDE_HOME`, Gemini CLI doesn't support one).

`is_installed()` — checks if `~/.gemini/extensions/gemini-warp/gemini-extension.json` exists and parses as valid JSON. `fs::read_to_string` follows symlinks, so `gemini extensions link` is handled.

`installed_version()` — reads `~/.gemini/extensions/gemini-warp/gemini-extension.json`, parses the `version` field. The JSON structure is flat: `{"name": "warp", "version": "1.0.0", ...}`.

`needs_update()` — calls `installed_version()`, compares against `MINIMUM_PLUGIN_VERSION` using `compare_versions`. Returns `true` if version is lower, or if installed but no version field.

**Auto-install/update:**

`install()`:
```
gemini extensions install https://github.com/warpdotdev/gemini-cli-warp --consent
```
`--consent` skips the interactive security confirmation prompt.

`update()`:
```
gemini extensions update gemini-warp
```

Both delegate to the shared `run_cli_command_logged()` helper in `mod.rs` via a thin `run_logged()` wrapper method. The shared helper takes a CLI name, args, executor, and env vars, runs the command via `LocalCommandExecutor::execute_local_command_in_login_shell`, and returns `Result<(), PluginInstallError>`.

**Instructions (fallback):**

Install instructions: single step — `gemini extensions install https://github.com/warpdotdev/gemini-cli-warp --consent`.

Update instructions: single step — `gemini extensions update gemini-warp`. Post-install note: "Restart Gemini CLI to activate the update."

### 4c. Factory Registration (`plugin_manager/mod.rs`)

Add `pub(crate) mod gemini;` to module declarations.

Add match arm in `plugin_manager_for_with_shell()` before the wildcard:

```rust
CLIAgent::Gemini
    if FeatureFlag::GeminiNotifications.is_enabled()
        && FeatureFlag::HOANotifications.is_enabled() =>
{
    Some(Box::new(GeminiPluginManager::new(
        shell_path,
        shell_type,
        path_env_var,
    )))
}
```

### 4d. Listener Registration (`listener/mod.rs`)

**`is_agent_supported()`** — add `CLIAgent::Gemini`:

```rust
matches!(
    agent,
    CLIAgent::Claude | CLIAgent::OpenCode | CLIAgent::Codex | CLIAgent::Gemini
)
```

**`create_handler()`** — add `CLIAgent::Gemini` alongside Claude/OpenCode (same structured JSON protocol, uses `DefaultSessionListener`):

```rust
CLIAgent::Claude | CLIAgent::OpenCode | CLIAgent::Gemini => {
    Some(Box::new(DefaultSessionListener))
}
```

No feature flag checks needed here — the call sites (`register_cli_agent_listener` in `terminal/view.rs`) already gate on `HOANotifications`.

### 4e. Chip Behavior

Since `can_auto_install() == true` and `is_installed()` does a filesystem check, the chip behavior matches Claude Code exactly:
- Installed + up to date → no chip (no flicker)
- Installed + outdated → update chip immediately
- Not installed → install chip immediately
- The debounce guard (`plugin_chip_ready`) only applies to `!manager.can_auto_install()` agents, so Gemini skips it

## 5. End-to-End Flow

### Install
1. User starts Gemini CLI in Warp. `CLIAgent::Gemini` detected, session created.
2. `plugin_manager_for(Gemini)` returns `Some(GeminiPluginManager)`.
3. Footer: `plugin_chip_kind()` → no listener, `is_installed()` checks `~/.gemini/extensions/gemini-warp/gemini-extension.json` → not found → `PluginChipKind::Install`.
4. `should_use_manual_mode()` → `false` (auto-install, local, no prior failure).
5. User clicks → `handle_plugin_operation()` → `gemini extensions install https://github.com/warpdotdev/gemini-cli-warp --consent`.
6. Success toast → user restarts Gemini → plugin hooks fire, `SessionStart` reports `plugin_version: "1.0.0"` → chip disappears.

### Update
1. `MINIMUM_PLUGIN_VERSION` bumped to `"1.1.0"`.
2. On session start, `is_installed()` → `true`, `needs_update()` → `true` (on-disk version `"1.0.0"` < `"1.1.0"`) → `PluginChipKind::Update`.
3. User clicks → `gemini extensions update gemini-warp` → success → restart → version `"1.1.0"` → chip gone.

### Auto-install failure fallback
1. `gemini` not on PATH → `install()` returns `Err`.
2. Error toast + `has_plugin_auto_failed` set.
3. Next render: `should_use_manual_mode()` → `true`. Chip becomes instructions button.
4. User clicks → split pane with manual instructions.

## 6. Risks and Mitigations

**Risk: `gemini` not on PATH in the login shell.** `npm install -g` with nvm may not be on PATH in non-interactive login shells. **Mitigation:** `path_env_var` from the terminal session captures the user's interactive PATH. Manual instructions fallback also covers this.

**Risk: `--consent` not available in older Gemini CLI versions.** **Mitigation:** Extensions framework (and `--consent`) shipped in v0.4.x. Users on older versions wouldn't have extensions support at all, so this is a non-issue.

**Risk: `~/.gemini` directory doesn't exist yet.** A fresh Gemini CLI install may not have created the extensions directory. **Mitigation:** `is_installed()` returns `false` when the path doesn't exist, which is correct — the install chip shows.

## 7. Testing and Validation

### Unit tests (`plugin_manager/gemini_tests.rs`)
- `can_auto_install_is_true`
- `minimum_version` — returns `"1.0.0"`
- `install_instructions_has_steps`
- `update_instructions_has_steps`
- `installed_when_extension_present` — write valid `gemini-extension.json` to temp dir, verify `true`
- `not_installed_when_extension_missing` — empty temp dir, verify `false`
- `not_installed_when_json_invalid` — invalid JSON in manifest, verify `false`
- `installed_version_returns_version_when_present` — verify version string extraction
- `installed_version_returns_none_when_no_version_field` — verify `None` when version field missing
- `installed_version_returns_none_when_file_missing` — verify `None` when no manifest
- `needs_update_logic_true_when_version_outdated` — version `"0.9.0"` against minimum `"1.0.0"`, verify update needed
- `needs_update_logic_false_when_version_current` — version `"1.0.0"` against minimum `"1.0.0"`, verify no update needed

### Unit tests (`plugin_manager/mod_tests.rs`)
- `returns_manager_for_gemini` — requires both `GeminiNotifications` and `HOANotifications` enabled
- Remove `CLIAgent::Gemini` from `returns_none_for_unsupported_agents`

### Manual testing
- Start Gemini CLI session → install chip appears (not installed on disk)
- Click install → auto-install succeeds → toast → restart → notifications work
- Bump minimum version → update chip appears (before plugin connects)
- Click update → auto-update succeeds → toast
- Disconnect `gemini` from PATH → auto fails → manual instructions pane works
- Feature flags off → no chip, no listener

## 8. Follow-Ups

- **Publish `warpdotdev/gemini-warp` to GitHub** — must happen before shipping to external users.
- **Platform plugin / Oz harness support** — future work.
- **Promote `GeminiNotifications` from dogfood** — after validation.

## 9. Files Changed

- **New:** `plugin_manager/gemini.rs` — `GeminiPluginManager`, filesystem detection, install/update via `LocalCommandExecutor`, instructions
- **New:** `plugin_manager/gemini_tests.rs` — unit tests
- **Modified:** `plugin_manager/mod.rs` — add `pub(crate) mod gemini;`, wire `CLIAgent::Gemini` into factory, add `install_success_message` / `update_success_message` default trait methods, extract shared `run_cli_command_logged` and `path_env_from_var` helpers
- **Modified:** `plugin_manager/claude.rs` — override `install_success_message` and `update_success_message` with Claude-specific reload-plugins messages, refactor to use shared `run_cli_command_logged` helper
- **Modified:** `plugin_manager/mod_tests.rs` — add Gemini factory test, update unsupported agents test
- **Modified:** `listener/mod.rs` — add `CLIAgent::Gemini` to `is_agent_supported()` and `create_handler()`
- **Modified:** `agent_input_footer/mod.rs` — `handle_install_plugin` / `handle_update_plugin` now query the plugin manager for per-agent success messages
- **Modified:** `crates/warp_features/src/lib.rs` — add `GeminiNotifications` variant + `DOGFOOD_FLAGS`
