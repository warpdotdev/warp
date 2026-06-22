# REMOTE-1545: Server-driven harness availability

## Context

Harness UI (selector dropdown, filter dropdown, details panel, management view metadata) was previously driven by a hard-coded list of `Harness` variants gated behind `FeatureFlag::AgentHarness`. The server already knows which harnesses a user/team can access and whether each is enabled, but the client never queried this — every user saw the same list.

The fix mirrors the established `LLMPreferences` pattern: a singleton model fetches server data, caches it locally, and all UI reads from the model instead of hard-coded lists.

### Relevant code

**Existing pattern — `LLMPreferences`** (`app/src/ai/llms.rs:519-1071`):
- `SingletonEntity` registered in `lib.rs`
- Subscribes to `NetworkStatus`, `AuthManager`, `UserWorkspaces` to trigger refresh
- Fetches via `ServerApiProvider::get_ai_client().get_feature_model_choices()`
- Caches in `private_user_preferences` under `"AvailableLLMs"` key
- `on_server_update` invalidates stale per-profile model selections
- Emits `LLMPreferencesEvent::UpdatedAvailableLLMs`

**Harness enum** — `crates/warp_cli/src/agent.rs:122-148`: `Harness` with variants Oz, Claude, OpenCode, Gemini, Codex, Unknown. Now derives `Serialize`/`Deserialize` for caching.

**GraphQL schema** — `crates/warp_graphql_schema/api/schema.graphql`: existing `AgentHarness` enum. New `HarnessInfo` type and `User.availableHarnesses` field.

**UI consumers** — hard-coded harness lists in:
- `app/src/terminal/view/ambient_agent/harness_selector.rs` — selector dropdown
- `app/src/ai/agent_management/view.rs` — filter dropdown + card metadata
- `app/src/ai/conversation_details_panel.rs` — details panel harness section
- `app/src/terminal/input.rs` — submission guard
- `app/src/terminal/input/agent.rs` — harness row visibility

## Proposed changes

### 1. GQL query + cynic module

New file `crates/graphql/src/api/queries/get_available_harnesses.rs`:
- `GetAvailableHarnesses` query on `User.availableHarnesses`
- Returns `Vec<HarnessInfo>` with `harness: AgentHarness`, `displayName: String`, `enabled: Bool`

Schema additions in `schema.graphql`:
- `HarnessInfo` type, `AvailableHarnesses` wrapper, `User.availableHarnesses` field

Conversion: `convert_agent_harness_to_cli` maps `AgentHarness → warp_cli::agent::Harness` (same pattern as existing model conversions in `ai.rs`).

### 2. `HarnessAvailabilityModel` singleton

New file `app/src/ai/harness_availability.rs`. Intentionally follows the `LLMPreferences` pattern:

| Aspect | `LLMPreferences` | `HarnessAvailabilityModel` |
|---|---|---|
| Singleton | `SingletonEntity` in `lib.rs` | Same |
| Refresh triggers | `NetworkStatus::Online`, `AuthComplete`, `TeamsChanged` | Same three |
| Auth guard | `is_logged_in()` check before fetch | Same |
| Cache | `private_user_preferences.write_value("AvailableLLMs", json)` | `write_value("AvailableHarnesses", json)` |
| Default fallback | `ModelsByFeature::default()` (auto model) | `vec![HarnessAvailability { harness: Oz, enabled: true }]` |
| Diff + emit | Compare old/new, emit if changed | Same |

Key methods:
- `available_harnesses() → &[HarnessAvailability]` — full list for UI
- `should_show_harness_selector() → bool` — `FeatureFlag::AgentHarness.is_enabled() && enabled_count > 1`
- `has_any_enabled_harness() → bool` — submission guard
- `is_harness_enabled(harness) → bool` — selection validation

### 3. Selection invalidation in `AmbientAgentViewModel`

`LLMPreferences::on_server_update` clears stale profile model selections inside the model itself. Following this, `AmbientAgentViewModel` (not the `HarnessSelector` view) subscribes to `HarnessAvailabilityModel` and resets `self.harness` to `Harness::Oz` if the selected harness becomes unavailable. This ensures validation fires even when the selector view doesn't exist (e.g. agent management page, shared session joins).

This parallels the existing `validate_environment_after_initial_load` + `handle_cloud_model_event` pattern already in `AmbientAgentViewModel` for environment IDs.

### 4. UI migration: model reads replace hard-coded lists

All UI sites switch from iterating `[Harness::Oz, Harness::Claude, ...]` or checking `FeatureFlag::AgentHarness.is_enabled()` to reading `HarnessAvailabilityModel`:

- **Harness selector** (`harness_selector.rs`): `build_menu_items` takes `&[HarnessAvailability]`, renders disabled entries greyed-out. Subscribes to `Changed` to refresh menu.
- **Filter dropdown** (`agent_management/view.rs`): `build_harness_dropdown_items` reads from model.
- **Card metadata + filter visibility** (`agent_management/view.rs`): `should_show_harness_selector()` replaces `FeatureFlag::AgentHarness.is_enabled()`.
- **Details panel** (`conversation_details_panel.rs`): same `should_show_harness_selector()` gate.
- **Harness row** (`input/agent.rs`): same.
- **Submission guard** (`input.rs`): blocks submission with error toast when `!has_any_enabled_harness()`.

### 5. `FeatureFlag::AgentHarness` retained as kill switch

The feature flag is NOT removed — it gates CLI harness parsing, conversation loading, experiment bucketing, and telemetry. `should_show_harness_selector()` combines the flag check with the server-driven enabled count, so the flag can still kill harness UI entirely.

### 6. `Harness` serde support

`crates/warp_cli/src/agent.rs`: `Harness` gains `Serialize`/`Deserialize` derives (+ `serde` dep in `warp_cli/Cargo.toml`) so `HarnessAvailability` can be cached as JSON.

## Testing and validation

- **Manual**: log in as user on team with restricted harnesses → verify only enabled harnesses appear in selector and filter dropdown; disabled harnesses appear greyed-out; selecting a disabled harness is blocked; switching teams refreshes the list.
- **Offline fallback**: disconnect network after initial load → verify cached harness list persists across restart; reconnect → verify list refreshes.
- **Selection invalidation**: select Claude → admin disables Claude server-side → verify selection auto-resets to Oz without requiring selector view to be open.
- **Zero-harnesses**: all harnesses disabled → submission blocked with error toast.
- **Feature flag kill switch**: disable `FeatureFlag::AgentHarness` → verify all harness UI disappears regardless of server data.

## Follow-ups

- Remove debug `[lili]` log statements before merge
- **Block submission for disabled harness with `disableReason`**: show inline error when user tries to submit with a specifically-disabled harness (vs. zero-harnesses toast)
- **CLI `--harness` pre-validation**: currently deferred; server rejects with generic error
- Remove `FeatureFlag::AgentHarness` entirely after rollout stabilizes
- Remove `OzMultiHarnessExperiment` after all non-Oz harnesses GA
