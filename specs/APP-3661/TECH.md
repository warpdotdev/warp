# Tech Spec: Plugin Update Flow

See `specs/APP-3661/PRODUCT.md` for the product spec.

## 1. Trait Changes

`plugin_manager/mod.rs`

Keep `is_installed() -> bool` on the trait (filesystem check: is the plugin key present in `installed_plugins.json`?). Add `update()`, `update_instructions()`, and `needs_update()` methods:

```rust
trait CliAgentPluginManager: Send + Sync {
    fn is_installed(&self) -> bool;
    fn needs_update(&self) -> bool;
    async fn install(&self) -> Result<(), PluginInstallError>;
    async fn update(&self) -> Result<(), PluginInstallError>;
    fn install_instructions(&self) -> &'static PluginInstructions;
    fn update_instructions(&self) -> &'static PluginInstructions;
}
```

The **update** chip is primarily driven by `plugin_version` reported in the `SessionStart` event (see section 3b). As a fallback for plugins too old to send structured events, `needs_update()` checks the on-disk version. Also add `PluginModalKind { Install, Update }` enum for event plumbing.

## 2. Renamed Instructions Struct

`plugin_manager/mod.rs`

Rename `PluginInstallInstructions` → `PluginInstructions` and `PluginInstallStep` → `PluginInstructionStep`:

```rust
pub(crate) struct PluginInstructionStep {
    pub description: &'static str,
    pub command: &'static str,
}

pub(crate) struct PluginInstructions {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub steps: &'static [PluginInstructionStep],
    pub success_toast: &'static str,
}
```

## 3. Claude Implementation

`plugin_manager/claude.rs`

### is_installed()

Unchanged — reads `installed_plugins.json`, returns true if the `PLUGIN_KEY` entry exists and is non-empty.

### MINIMUM_PLUGIN_VERSION

New `pub(crate)` `&str` constant (initially `"2.0.0"`). Exported so the footer can compare against it.

Must be kept in sync with the plugin version in `warpdotdev/claude-code-warp`. Add a comment on the constant pointing to the plugin repo, and a reciprocal comment in the plugin repo's README.

### compare_versions()

New `pub(crate)` helper. Compares two `X.Y.Z` version strings using simple integer comparison. Unparseable components treated as 0.

### needs_update()

Checks the on-disk version in `installed_plugins.json`. Returns true if the installed version is below `MINIMUM_PLUGIN_VERSION`, or if the entry exists but has no version field (very old plugin). Used as a fallback when no listener has connected (e.g., the plugin is too old to send structured events).

### update()

Runs `marketplace remove` + `marketplace add` (to ensure the local clone is fresh) + `plugin install` (to reinstall the plugin from the freshly added marketplace). We use `plugin install` instead of `plugin update` because `marketplace remove` unlinks the plugin, so `plugin update` would fail with `Plugin "warp" is not installed`.

As an internal sanity check, re-reads `installed_plugins.json` and checks the version. If still below minimum → returns `Err` with a message like "Plugin update did not take effect". This triggers the fallback to manual mode in the footer.

## 3b. Session Version Tracking

`cli_agent_sessions/mod.rs`

Add `plugin_version: Option<String>` to `CLIAgentSession`. Populated via two paths:
1. **`SessionStart` event path:** `register_listener()` now accepts `plugin_version` as a parameter, threaded from the `SessionStart` notification payload. This is the primary path.
2. **Mid-session install path:** `register_cli_agent_listener()` (called after install/update succeeds) sets `plugin_version` to `MINIMUM_PLUGIN_VERSION` to suppress the update chip until the user runs `/reload-plugins`.

Also set in `apply_event()` when the event type is `SessionStart` (for subsequent `SessionStart` events after the initial one).

This is the **authoritative signal** for whether the plugin is outdated. It works for both local and remote sessions because the plugin reports its own version. A `None` value means the plugin predates version reporting and is definitely outdated.

### update_instructions()

Returns a `&'static PluginInstructions` (via `LazyLock`) with:

- Title: "Update Warp Plugin for Claude Code"
- Subtitle: "Run the following commands in Claude Code by typing ! before each command, or in a separate terminal."
- Steps:
  1. `claude plugin marketplace remove claude-code-warp`
  2. `claude plugin marketplace add warpdotdev/claude-code-warp`
  3. `claude plugin install warp@claude-code-warp`
  4. Restart Claude Code to activate → `/exit`
- Success toast (auto-update): "Warp plugin updated. Please run /reload-plugins to activate." (the one-click flow registers the listener programmatically, so `/reload-plugins` suffices)
- Success toast (manual modal): tells user to restart Claude Code (manual installs require a full restart for hooks to fire)

Note: the update modal uses CLI commands (not in-session slash commands) because there is no working `/plugin update` slash command in Claude Code. The install modal continues to use slash commands since `/plugin install` works in-session.

## 4. Modal Changes

`workspace/view/plugin_install_modal.rs`

The modal becomes a generic instructions renderer. Replace `agent: Option<CLIAgent>` with `instructions: Option<&'static PluginInstructions>`. The `set_agent(agent)` method becomes `set_instructions(instructions: &'static PluginInstructions)` which stores the reference and resizes `step_code_handles` for the steps.

Copy button on each step copies the command to clipboard and shows a green success toast.

## 5. Footer Changes

`agent_input_footer/mod.rs`

### New buttons

Add two new `ActionButton` views (same `InstallPluginButtonTheme` and construction pattern as the existing install buttons):

- `update_plugin_button`: "Update Warp plugin", `Icon::Download`, dispatches `AgentInputFooterAction::UpdatePlugin`
- `update_instructions_button`: "Plugin update instructions", `Icon::Info`, dispatches `AgentInputFooterAction::ShowPluginInstructionsModal`

### Chip visibility and mode

The footer uses a `PluginChipKind` enum (`Install` / `Update`) returned by `plugin_chip_kind()`. The logic uses three layers of version detection:

**Install chip** (pre-connection, local only):

- No listener, local session, `is_installed()` returns false, install chip not dismissed → `PluginChipKind::Install`
- Same conditions as today (unchanged)

**Update chip** (post-connection, local and remote):

- Listener connected, `session.plugin_version` is `None` or < `MINIMUM_PLUGIN_VERSION` → `PluginChipKind::Update`
- (`None` means the plugin predates version reporting and definitely needs an update)
- Update chip dismissed for current minimum version → hidden
- Listener connected, `session.plugin_version` >= minimum → no chip

**Update chip** (pre-connection filesystem fallback, local only):

- No listener, `is_installed()` true, `needs_update()` true → `PluginChipKind::Update`
- Handles plugins too old to send structured events (no listener ever connects)
- Only works locally — for remote sessions with old plugins that don't send structured events, we fall through to the install chip since we can't check the remote filesystem

**No chip:**

- No listener, plugin installed on disk, on-disk version current → waiting for connection
- Notifications disabled
- Operation in progress

### Chip mode selection

The render method selects which button based on install-vs-update and auto-vs-manual:

- Install + auto → `install_plugin_button`
- Install + manual → `plugin_instructions_button` (install instructions modal)
- Update + auto → `update_plugin_button`
- Update + manual → `update_instructions_button` (update instructions modal)

Manual mode is triggered by: remote session, or prior auto-operation failure for this agent/host.

### Failure tracking

Reuse the existing `plugin_install_failures: HashSet<(CLIAgent, Option<String>)>` on `CLIAgentSessionsModel` — rename to `plugin_auto_failures`. This single set covers both install and update failures. This works because `PluginStatus` already determines which operation the chip shows; there's no scenario where install failures and update failures need to be distinguished (and the set resets each session anyway).

### handle_plugin_operation()

Extract a shared helper from `handle_install_plugin` that both install and update use. The shared logic: set `plugin_operation_in_progress`, show persistent toast, spawn the async operation, on success emit `PluginInstalled` event, on failure record in `plugin_auto_failures` and show error toast. Replace the two separate `plugin_install_in_progress`/`plugin_update_in_progress` bools with a single `plugin_operation_in_progress: bool` (install and update are mutually exclusive based on `PluginStatus`).

The only differences between install and update are: (a) which async fn to call (`manager.install()` vs `manager.update()`), (b) progress/success/error toast messages. All three are passed as `&str` parameters.

## 6. Settings

`settings/ai.rs`

Add a new setting for update chip dismissal:

```
plugin_chip_dismissed_for_version: PluginChipDismissedForVersion {
    type: String,
    default: "",
    supported_platforms: SupportedPlatforms::DESKTOP,
    sync_to_cloud: SyncToCloud::Never,
    hierarchy: "private",
}
```

When the user dismisses the update chip, store the current `MINIMUM_PLUGIN_VERSION`. In `should_show_plugin_chip`, compare the dismissed version against the current minimum — if dismissed version >= current minimum, hide; otherwise show.

## 7. Event Plumbing

Replace the existing `ShowPluginInstallModal(CLIAgent)` with a single `ShowPluginInstructionsModal(CLIAgent, PluginModalKind)` where `PluginModalKind { Install, Update }`. This avoids duplicating an event variant through every layer of the chain.

`AgentInputFooterEvent` → `Input::Event` → `TerminalView::Event` → `pane_group::Event` → Workspace handler

The workspace handler matches on the kind, calls the appropriate `install_instructions()` or `update_instructions()`, and passes the result to `modal.set_instructions(...)` before opening.

## 8. Testing

### Unit tests (`plugin_manager/claude_tests.rs`)

Existing `check_installed` tests remain valid (now testing `is_installed()`).

Add:

- `compare_versions` — covers equal, less-than, greater-than, different major/minor/patch, unparseable components

### Unit tests (`cli_agent_sessions/mod_tests.rs`)

Rename existing `plugin_install_failure` tests to `plugin_auto_failure` and verify the renamed set works identically. No new test logic needed — just a rename.

### Unit tests (`plugin_manager/mod_tests.rs`)

Add tests for the new trait methods:
- `claude_manager_returns_update_instructions` — verify `update_instructions()` returns non-empty steps
- `claude_manager_returns_install_instructions` — verify `install_instructions()` returns non-empty steps

### View tests (`terminal/view_test.rs`)

Using the existing `App::test` + `CLIAgentSessionsModel` pattern:

- `update_chip_shown_when_plugin_version_below_minimum` — set `session.plugin_version` to `"1.1.0"`, verify update chip shown
- `update_chip_shown_when_plugin_version_is_none` — listener connected but no `plugin_version`, verify update chip shown
- `no_chip_when_plugin_version_meets_minimum` — set `session.plugin_version` to `"2.0.0"`, verify no chip
- `update_chip_hidden_when_dismissed_for_current_version`
- `update_chip_shown_when_dismissed_for_older_version`
- `update_chip_and_install_chip_dismiss_are_independent`

### Integration tests

Add to `integration/tests/integration/ui_tests.rs`:

- `test_plugin_update_chip_appears_for_outdated_plugin` — start a Claude Code session (via OSC event injection), set up a fake `installed_plugins.json` with an old version, verify the "Update Warp plugin" chip renders in the footer
- `test_plugin_update_modal_opens` — same setup as above but in manual mode (inject a failure first), click the instructions chip, verify the modal opens with update steps
- `test_plugin_update_chip_dismiss_persists` — click dismiss on the update chip, verify it stays hidden, then bump `MINIMUM_PLUGIN_VERSION` concept (or re-render), verify it reappears for a new minimum

## 9. Files Changed

- **Modified:** `plugin_manager/mod.rs` — renamed structs, `update()` + `update_instructions()` + `needs_update()` on trait, `PluginModalKind` enum, `PluginChipKind` enum (in footer)
- **Modified:** `plugin_manager/claude.rs` — `update()` implementation, `MINIMUM_PLUGIN_VERSION` constant (pub(crate)), `compare_versions` (pub(crate)), update instructions `LazyLock`
- **Modified:** `cli_agent_sessions/mod.rs` — `plugin_version: Option<String>` on `CLIAgentSession`, populated from `SessionStart` event; rename `plugin_install_failures` → `plugin_auto_failures`
- **Modified:** `workspace/view/plugin_install_modal.rs` — generic instructions rendering via `set_instructions()`, copy-to-clipboard with success toast
- **Modified:** `agent_input_footer/mod.rs` — new buttons, chip visibility via `plugin_chip_kind()`, `handle_plugin_operation` helper, `plugin_operation_in_progress`
- **Modified:** `settings/ai.rs` — `plugin_chip_dismissed_for_version` setting
- **Modified:** `terminal/input.rs`, `terminal/view.rs`, `pane_group/mod.rs`, `pane_group/pane/terminal_pane.rs` — `ShowPluginInstructionsModal` event (replaces old install-only variant)
- **Modified:** `workspace/view.rs`, `workspace/mod.rs` — modal rename, instruction-kind dispatch
- **Modified:** `workspace/util.rs` — rename modal state field
