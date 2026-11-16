# Tech Spec: OpenCode Plugin Install & Update Flow

See `specs/APP-3690/PRODUCT.md` for the product spec.

## 1. Problem

`plugin_manager_for(CLIAgent::OpenCode)` returns `None` (`plugin_manager/mod.rs:90`), so OpenCode sessions never show the install/update chip. We need an `OpenCodePluginManager` that implements the `CliAgentPluginManager` trait and wires into the existing footer/modal infrastructure.

Unlike Claude Code, OpenCode has no CLI for plugin management, so this implementation is manual-only (instructions modal, no auto-install). This requires a small trait refactor to make auto-operations optional and to generalize the per-agent minimum version.

## 2. Relevant Code

- `plugin_manager/mod.rs` — trait definition, `plugin_manager_for()` factory, `PluginInstructions` types
- `plugin_manager/claude.rs` — reference implementation; owns `compare_versions` and `MINIMUM_PLUGIN_VERSION`
- `agent_input_footer/mod.rs (677-744)` — `plugin_chip_kind()` determines which chip to show
- `agent_input_footer/mod.rs (748-758)` — `should_use_manual_mode()` determines auto vs modal
- `agent_input_footer/mod.rs (760-849)` — `handle_plugin_operation()` shared async handler (unused by OpenCode)
- `agent_input_footer/mod.rs (978-999)` — render logic matching `(chip_kind, manual)` to buttons
- `workspace/view/plugin_install_modal.rs` — generic instructions modal (already agent-agnostic, takes `&'static PluginInstructions`)
- `cli_agent_sessions/mod.rs (86-104)` — `CLIAgentSession` struct with `plugin_version`, `listener`
- `terminal/view.rs:10732-10771` — `register_cli_agent_listener()` uses `MINIMUM_PLUGIN_VERSION` from `claude.rs`
- `settings/ai.rs:1093-1105` — `plugin_chip_dismissed_for_version` setting

## 3. Current State

The plugin manager system works well for Claude Code:
- The trait has six required methods, all implemented by `ClaudeCodePluginManager`.
- The footer routes between auto-install and manual-modal based on `should_use_manual_mode()`.
- The modal is already generic (takes `&'static PluginInstructions`, not a `CLIAgent`).

Three things need generalizing:
1. **`compare_versions` and `MINIMUM_PLUGIN_VERSION`** live in `claude.rs` but are imported by `agent_input_footer/mod.rs:65` and `terminal/view.rs:134`. With two agents needing different minimum versions, these should come from the trait.
2. **All trait methods are required.** OpenCode doesn't need `install()`, `update()`, `is_installed()`, or `needs_update()`. These should have sensible defaults.
3. **Install chip flicker.** Claude avoids flicker by checking `is_installed()` on the filesystem. OpenCode has no filesystem checks, so the install chip would flash on session start before the plugin connects. A debounce is needed.

## 4. Proposed Changes

### 4a. Trait Refactor (`plugin_manager/mod.rs`)

**Move `compare_versions` from `claude.rs` to `mod.rs`.** It's a generic semver utility, not Claude-specific. Same signature, just a new home.

**Add two new required methods to the trait:**

```rust
fn minimum_plugin_version(&self) -> &'static str;
fn can_auto_install(&self) -> bool;
```

`minimum_plugin_version()` replaces the hardcoded `MINIMUM_PLUGIN_VERSION` constant at all call sites. Each agent returns its own constant.

`can_auto_install()` tells the footer whether this agent can auto-install/update. Claude returns `true`, OpenCode returns `false`.

**Add default implementations for the four auto-operation methods:**

```rust
fn is_installed(&self) -> bool { false }
fn needs_update(&self) -> bool { false }

async fn install(&self) -> Result<(), PluginInstallError> {
    Err(PluginInstallError {
        message: "Auto-install not supported for this agent".to_owned(),
        log: String::new(),
    })
}

async fn update(&self) -> Result<(), PluginInstallError> {
    Err(PluginInstallError {
        message: "Auto-update not supported for this agent".to_owned(),
        log: String::new(),
    })
}
```

Claude overrides all four. OpenCode uses the defaults. The defaults return `Err` — the caller (`handle_plugin_operation`) already logs errors from failed operations. These should never be reached since `should_use_manual_mode()` returns `true` for agents where `can_auto_install()` is `false`.

### 4b. OpenCode Plugin Manager (`plugin_manager/opencode.rs`)

New file. Minimal implementation — just the two required methods plus instructions:

- `can_auto_install()` → `false`
- `minimum_plugin_version()` → `"0.1.0"` (keep in sync with `opencode-warp` npm package version)
- `install_instructions()` → static `PluginInstructions` with title "Install Warp Plugin for OpenCode", steps to add `"opencode-warp"` to the `plugin` array in `opencode.json` + restart
- `update_instructions()` → static `PluginInstructions` with title "Update Warp Plugin for OpenCode", steps to `rm -rf ~/.cache/opencode/node_modules/opencode-warp` + restart

All auto-operation methods (`is_installed`, `needs_update`, `install`, `update`) use the trait defaults.

### 4c. Factory Registration (`plugin_manager/mod.rs`)

```rust
CLIAgent::OpenCode => Some(Box::new(opencode::OpenCodePluginManager)),
```

### 4d. Footer: Generalize `MINIMUM_PLUGIN_VERSION` (`agent_input_footer/mod.rs`)

Replace the import of `claude::{compare_versions, MINIMUM_PLUGIN_VERSION}` with:
- `use super::compare_versions` (from `mod.rs`)
- All references to `MINIMUM_PLUGIN_VERSION` become `manager.minimum_plugin_version()`

This affects three places in `plugin_chip_kind()`:
- `agent_input_footer/mod.rs:704` — version comparison for connected plugin
- `agent_input_footer/mod.rs:712` — dismissed version comparison
- `agent_input_footer/mod.rs:732` — dismissed version comparison (filesystem fallback path)

And one place in the dismiss handler:
- `agent_input_footer/mod.rs:1583` — storing the dismissed version

For the dismiss handler, the manager needs to be resolved to get its `minimum_plugin_version()`. Since `plugin_chip_kind()` already resolves the manager, and the dismiss handler knows whether it's dismissing an update chip, this is straightforward — resolve the manager from the session's agent.

### 4e. Footer: `should_use_manual_mode()` (`agent_input_footer/mod.rs:748`)

Add a check for `can_auto_install()` before the existing conditions:

```rust
fn should_use_manual_mode(&self, app: &AppContext) -> bool {
    let sessions_model = CLIAgentSessionsModel::as_ref(app);
    let session = match sessions_model.session(self.terminal_view_id) {
        Some(s) => s,
        None => return false,
    };
    if let Some(manager) = plugin_manager_for(session.agent) {
        if !manager.can_auto_install() {
            return true;
        }
    }
    if session.is_remote() {
        return true;
    }
    sessions_model.has_plugin_auto_failed(session.agent, &session.remote_host)
}
```

For OpenCode, this always returns `true`, so the footer always opens the modal instead of attempting auto-install. The `handle_plugin_operation` / `handle_install_plugin` / `handle_update_plugin` code paths are never reached.

### 4e-ii. Footer: Chip Button Labels

The install chip buttons are created at construction time with hardcoded labels:
- `install_plugin_button`: "Enable Claude Code notifications" — Claude-specific
- `plugin_instructions_button`: "Notifications setup instructions"
- `update_plugin_button`: "Update Warp plugin"
- `update_instructions_button`: "Plugin update instructions"

The auto-install button label hardcodes "Claude Code". Make it dynamic using the agent's `display_name()`: format as `"Enable {name} notifications"` where `name` comes from `session.agent.display_name()` (e.g., "Enable Claude Code notifications", "Enable OpenCode notifications"). Since the button is created once at construction, update the label via `set_label` in the render path (or when the session changes), the same way the compose button label is already updated dynamically (`agent_input_footer/mod.rs:374`).

### 4f. `terminal/view.rs`: Generalize `register_cli_agent_listener`

`terminal/view.rs:10753` hardcodes `MINIMUM_PLUGIN_VERSION` from `claude.rs`. Replace with:

```rust
let plugin_version = plugin_manager_for(agent)
    .map(|m| m.minimum_plugin_version().to_owned());
```

This returns the correct minimum version for whichever agent is active, and `None` if the agent has no plugin manager (which preserves the existing wasm fallback behavior).

### 4g. Install Chip Debounce (`agent_input_footer/mod.rs`)

**Problem:** For agents without filesystem checks (OpenCode), `plugin_chip_kind()` would show the install chip immediately on session start, before the plugin has time to connect and send `SessionStart`.

**Solution:** Add `plugin_chip_ready: bool` to `AgentInputFooter`. It starts `false` and is set to `true` after a debounce timer fires. When a `Started` event fires for a non-auto-install agent, spawn a one-shot timer via `ctx.spawn(Timer::after(PLUGIN_CHIP_DEBOUNCE), ...)`. When the timer fires, set `plugin_chip_ready = true` and call `ctx.notify()` to trigger a re-render. In the timer callback, check whether a listener has connected in the meantime — if so, skip setting the flag.

In `plugin_chip_kind()`, the "no listener" branch checks:

```rust
if !manager.can_auto_install() && !self.plugin_chip_ready {
    return None;
}
```

Reset `plugin_chip_ready = false` when a session ends or when a listener connects.

For Claude (which has `can_auto_install() == true`), this check is skipped — Claude uses `is_installed()` instead.

`PLUGIN_CHIP_DEBOUNCE` is a constant in `agent_input_footer/mod.rs` (`Duration::from_secs(3)`).

### 4h. Claude Module Cleanup (`plugin_manager/claude.rs`)

- Remove `compare_versions` (moved to `mod.rs`)
- Add `minimum_plugin_version()` returning `MINIMUM_PLUGIN_VERSION`
- Add `can_auto_install()` returning `true`
- Keep `MINIMUM_PLUGIN_VERSION` as a module-level constant (still useful for the `needs_update()` and `update()` implementations within this module)

## 5. End-to-End Flow

### Install
1. User starts OpenCode in Warp. Command detection creates a `CLIAgentSession`. The footer's `Started` subscription fires, spawning a debounce timer (`plugin_chip_ready` starts `false`).
2. Footer renders. `plugin_chip_kind()` finds `plugin_manager_for(OpenCode)` = `Some`. No listener. `can_auto_install()` is `false`. `plugin_chip_ready` is `false` → returns `None`. No chip.
3. If plugin is installed: `SessionStart` arrives within ~1s, listener is created, `plugin_version` is set, `plugin_chip_ready` reset to `false`. Footer re-renders. Chip never appears.
4. If plugin is not installed: timer fires after 3s, sets `plugin_chip_ready = true`, calls `ctx.notify()`. `plugin_chip_kind()` returns `Install`. `should_use_manual_mode()` returns `true`. Footer shows the instructions chip.
5. User clicks → modal opens with install steps (add to `opencode.json`, restart).
6. User follows steps, restarts OpenCode. Plugin connects, sends `SessionStart`. Chip disappears.

### Update
1. We bump `MINIMUM_PLUGIN_VERSION` in `opencode.rs` to `"0.2.0"`.
2. Plugin connects with `plugin_version: "0.1.0"`.
3. `plugin_chip_kind()`: listener present, `compare_versions("0.1.0", "0.2.0")` is `Less` → `Update`.
4. `should_use_manual_mode()` returns `true`. Footer shows update instructions chip.
5. User clicks → modal shows cache-clear + restart steps.
6. User follows steps. On restart, Bun re-resolves `opencode-warp` from npm (cache was cleared), installs latest. Plugin connects with `"0.2.0"`. Chip disappears.

## 6. Risks and Mitigations
**Risk: `PluginInstructionStep.command` contains JSON, not a shell command.**
**Risk: `PluginInstructionStep.command` contains JSON, not a shell command.** The install modal's copy button copies the `command` field to clipboard. For OpenCode install, this will be a JSON snippet. **Mitigation:** Already fine — the modal copies any string. A JSON snippet is useful to copy even if it's not a terminal command.

**Risk: Stale `MINIMUM_PLUGIN_VERSION`.** The minimum version is compiled into the Warp binary. **Mitigation:** Same as Claude — by design. We only prompt updates when Warp needs new plugin behavior.

## 7. Testing and Validation

### Unit tests (`plugin_manager/opencode_tests.rs`)
- `opencode_manager_can_auto_install_is_false`
- `opencode_manager_returns_install_instructions` — non-empty steps
- `opencode_manager_returns_update_instructions` — non-empty steps
- `opencode_manager_minimum_version` — returns expected value

### Unit tests (`plugin_manager/mod_tests.rs`)
- Update `returns_none_for_unsupported_agents` — remove `CLIAgent::OpenCode` from assertion
- Add `returns_manager_for_opencode`
- Move `compare_versions` tests here from `claude_tests.rs`

### Unit tests (`plugin_manager/claude_tests.rs`)
- Remove `compare_versions` tests (moved)
- Add `claude_manager_can_auto_install_is_true`
- Add `claude_manager_minimum_version`

### Manual testing
- Start an OpenCode session → install chip appears after ~3s debounce (not immediately)
- If plugin is installed: chip never appears (plugin connects before debounce)
- Click install chip → modal opens with correct OpenCode-specific instructions
- Follow install steps, restart → chip disappears
- Bump minimum version locally → update chip appears
- Click update chip → modal shows cache-clear instructions
- Dismiss install chip → stays hidden
- Dismiss update chip → stays hidden; bump minimum → reappears
- Claude flow unchanged — verify no regressions

## 8. Follow-Ups

- **Auto-install:** If OpenCode adds a plugin management CLI, implement `install()` and `update()` on `OpenCodePluginManager` and flip `can_auto_install()` to `true`.
- **Publish `opencode-warp` to npm:** Must happen before this feature ships.

## 9. Files Changed

- **New:** `plugin_manager/opencode.rs` — `OpenCodePluginManager`, `MINIMUM_PLUGIN_VERSION`, install/update instructions
- **New:** `plugin_manager/opencode_tests.rs`
- **Modified:** `plugin_manager/mod.rs` — `compare_versions` moved here, `minimum_plugin_version()` + `can_auto_install()` added to trait, default impls for `is_installed`/`needs_update`/`install`/`update`, factory wires OpenCode
- **Modified:** `plugin_manager/claude.rs` — remove `compare_versions` (moved), add `minimum_plugin_version()` + `can_auto_install()` overrides
- **Modified:** `plugin_manager/claude_tests.rs` — remove `compare_versions` tests (moved), add new trait method tests
- **Modified:** `plugin_manager/mod_tests.rs` — add OpenCode factory test, receive `compare_versions` tests
- **Modified:** `agent_input_footer/mod.rs` — add `plugin_chip_ready: bool` + debounce timer, update imports (`compare_versions` from `mod.rs`), `plugin_chip_kind()` uses `manager.minimum_plugin_version()` + `plugin_chip_ready` guard, `should_use_manual_mode()` checks `can_auto_install()`, dismiss handler resolves minimum version from manager, rename auto-install chip label to be agent-generic
- **Modified:** `terminal/view.rs` — `register_cli_agent_listener()` uses `plugin_manager_for(agent)?.minimum_plugin_version()` instead of Claude's constant
