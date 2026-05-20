# REMOTE-1573 — Tech spec

## Context

This spec implements the behavior described in `specs/REMOTE-1573/PRODUCT.md`. The work touches five areas:

1. **New AI settings** — two booleans (`cloud_handoff_enabled`, `ampersand_handoff_enabled`) in the `AISettings` settings group (`app/src/settings/ai.rs:710-1463`).
2. **Settings UI** — a new widget section on the AI settings page (`app/src/settings_view/ai_page.rs`) following the pattern of `CloudAgentComputerUseWidget` (line 6191).
3. **Handoff surface gating** — using the effective setting value to gate the `&` prefix (`app/src/terminal/input.rs:3721-3751`), the `/handoff` slash command (`app/src/terminal/input/slash_commands/mod.rs:882-910`), the footer chip (`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2043-2053`), and the workspace action handler (`app/src/workspace/view.rs:20681-20691`).
4. **Snapshot gating** — adding `snapshot_disabled` to `SpawnAgentRequest` and setting it from the cloud-conversation-storage state in every client-side spawn path (`app/src/terminal/view/ambient_agent/model.rs:967-994`, `build_handoff_spawn_request` at line 569).

### How the effective handoff value is derived

The effective value is `false` when any of these is true (PRODUCT.md invariant 9):
- `AISettings::is_any_ai_enabled()` returns `false`
- Cloud conversation storage is effectively disabled (user-level `is_cloud_conversation_storage_enabled == false` on `PrivacySettings`, or org-level `cloud_conversation_storage_settings == Disable` on `WorkspaceSettings`)
- Feature flags `OzHandoff` or `HandoffLocalCloud` are off (existing gate in `is_local_to_cloud_handoff_available()` at `app/src/ai/blocklist/mod.rs:11-16`)
- The user has toggled the setting off

This should be computed as a single helper function so all four gating sites share the same logic.

### How `snapshot_disabled` propagates

`SpawnAgentRequest` is serialized to JSON and sent to `POST /agent/run`. The server stores it on the queued execution input. The cloud agent reads it from task metadata in `AgentSDK::fetch_secrets_and_attachments` (`app/src/ai/agent_sdk/mod.rs:957`) and wires it into `AgentDriverOptions.snapshot_disabled`. The existing `run_snapshot_upload` (`app/src/ai/agent_sdk/driver.rs:2368-2427`) already respects this field. So the only client-side work is adding the field to `SpawnAgentRequest` and setting it before spawn.

We pass `snapshot_disabled` through the spawn request rather than checking `PrivacySettings` directly in the driver because the driver never reads user-configurable settings singletons (`PrivacySettings`, `AISettings`, `UserWorkspaces`). All its configuration flows in through `AgentDriverOptions`, which is populated from CLI args and server task metadata. The only singletons the driver accesses are `FeatureFlag` (compile-time/runtime flags) and `ServerApiProvider` (for API calls). Passing `snapshot_disabled` on the request keeps the driver's existing input-driven pattern intact.

## Proposed changes

### 1. Add settings to `AISettings`

**`app/src/settings/ai.rs`**: Add two new entries to `define_settings_group!(AISettings, ...)`, following the pattern of `orchestration_enabled`:

- `cloud_handoff_enabled: CloudHandoffEnabled` — `bool`, default `true`, TOML path `agents.warp_agent.other.cloud_handoff_enabled`, desktop-only, cloud-synced.
- `ampersand_handoff_enabled: AmpersandHandoffEnabled` — `bool`, default `true`, TOML path `agents.warp_agent.other.ampersand_handoff_enabled`, desktop-only, cloud-synced.

Add two derived helpers on `AISettings`:

- `is_cloud_handoff_enabled(&self, app) -> bool` — returns `false` when any prerequisite is missing: AI disabled, setting off, feature flags off (delegates to `is_local_to_cloud_handoff_available()`), or cloud conversation storage off (user-level via `PrivacySettings` or org-level via `AdminEnablementSetting::Disable`).
- `is_ampersand_handoff_enabled(&self, app) -> bool` — returns `is_cloud_handoff_enabled(app) && *self.ampersand_handoff_enabled`.

### 2. Add `snapshot_disabled` to `SpawnAgentRequest`

inside `SpawnAgentRequest`: Add an `Option<bool>` field `snapshot_disabled` after `initial_snapshot_token`, with `skip_serializing_if = "Option::is_none"`.

### 3. Set `snapshot_disabled` at spawn time

**`app/src/terminal/view/ambient_agent/model.rs`**: In `build_default_spawn_config`, after computing `computer_use_enabled`, read cloud conversation storage state and set `snapshot_disabled` on the returned request. Since `AgentConfigSnapshot` doesn't carry `snapshot_disabled` (it's a `SpawnAgentRequest`-level field), the flag must be set in `spawn_agent` and `build_handoff_spawn_request` directly.

In `spawn_agent` (~line 968) and `build_handoff_spawn_request` (~line 569), set `snapshot_disabled: should_disable_snapshot(ctx).then_some(true)` on the `SpawnAgentRequest`.

Add a private helper `should_disable_snapshot(ctx: &AppContext) -> bool` that returns `true` when cloud conversation storage is off — either user-level (`PrivacySettings::is_cloud_conversation_storage_enabled == false`) or org-level (`AdminEnablementSetting::Disable`).

### 4. Gate handoff surfaces

All four sites switch from the current `is_local_to_cloud_handoff_available()` check to the new `AISettings::is_cloud_handoff_enabled(app)`:

- **`&` prefix** (`app/src/terminal/input.rs:3721-3733`): `can_activate_cloud_handoff_prefix` — replace the `is_local_to_cloud_handoff_available()` call with `AISettings::as_ref(ctx).is_ampersand_handoff_enabled(ctx)`.
- **`/handoff` slash command** (`app/src/terminal/input/slash_commands/mod.rs:882-887`): Replace the feature-flag check with `AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)`.
- **Footer chip** (`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2043-2053`): Replace `is_local_to_cloud_handoff_available()` with `AISettings::as_ref(app).is_cloud_handoff_enabled(app)`.
- **Workspace action** (`app/src/workspace/view.rs:13228`): `start_local_to_cloud_handoff` — replace the feature-flag check with the setting check. This is the final safety net.

`is_local_to_cloud_handoff_available()` in `app/src/ai/blocklist/mod.rs` is retained as a pure feature-flag check and called internally by `is_cloud_handoff_enabled`. Existing callers are migrated to the new setting-aware helper.

### 5. Settings UI widget

**`app/src/settings_view/ai_page.rs`**: Add a new `CloudHandoffWidget` struct implementing `SettingsWidget`, placed in the "Experimental" section near `CloudAgentComputerUseWidget`. Pattern:

- `should_render`: return `true` when `OzHandoff` and `HandoffLocalCloud` flags are on. The widget handles the disabled state internally.
- `render`: Compute force-disabled state from cloud-conversation-storage. When force-disabled, render the toggle as disabled with a tooltip. Beneath the parent toggle, conditionally render the `&` sub-toggle only when the effective handoff value is true (hidden when parent is off).

Follow the exact pattern of `AgentAttributionWidget` (line 6093) for org-forced-disabled toggles with tooltips, and `CloudAgentComputerUseWidget` (line 6191) for the overall section layout.

## Testing and validation

**Unit tests** (new file `app/src/settings/ai_tests.rs` or inline `#[cfg(test)]` block):
- `is_cloud_handoff_enabled` returns `false` when AI disabled, cloud convos off, or setting toggled off. Covers PRODUCT.md invariants 3, 9, 11.
- `is_ampersand_handoff_enabled` returns `false` when parent is off or sub-setting is off. Covers invariants 6, 7, 8.
- `should_disable_snapshot` returns `true` when cloud conversation storage is user-disabled or org-disabled. Covers invariants 16, 18.

**Existing tests**:
- `app/src/terminal/input_tests.rs` already has `&`-prefix tests (line 5733+). Add a case where `cloud_handoff_enabled` is false and verify `&` does not activate handoff compose.

**Manual validation**:
- Toggle handoff off in settings → verify `&`, `/handoff`, and footer chip are all suppressed.
- Disable cloud conversation storage → verify handoff toggle becomes force-disabled with tooltip.
- Spawn a cloud agent with cloud convos off → verify `snapshot_disabled: true` appears in the spawn request (network log or `log::info` in `spawn_agent`).
- Re-enable cloud convos → verify handoff toggle becomes interactive again.

**Compilation**: `cargo check -p warp` and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`.

## Parallelization

This task is not large enough to benefit from parallel sub-agents. The changes are tightly coupled (the setting definition feeds the UI widget, the gating sites, and the snapshot flag), and the total scope is ~200-300 LOC across ~8 files. Sequential implementation is the right approach.
