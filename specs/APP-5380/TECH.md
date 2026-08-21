# APP-5380: TUI custom LLM endpoints — technical specification

## Context

`PRODUCT.md` defines a TUI-only split between file-backed endpoint definitions and secure API keys. The GUI continues to persist complete `CustomEndpoint` values in the monolithic `AiApiKeys` secure-storage blob.

Research was performed at `4cd1c77c498821785baf0801bbd026f3693d2544`.

- [`app/src/ai/execution_profiles/config.rs (21-219) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/execution_profiles/config.rs#L21-L219) stores execution profiles as an `IndexMap` keyed by validated identity. Its display `name` is separate and is not the uniqueness boundary. A changed or removed map key changes or removes profile identity, while a display-name edit does not. GUI-created profiles still use `profile-<uuid>` keys. APP-5380 reuses the collection shape but makes the map key the display name so it does not copy the opaque-ID workflow.
- [`app/src/ai/execution_profiles/profiles.rs (71-171) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/execution_profiles/profiles.rs#L71-L171) enables one file-backed profile model for every TUI build and flagged GUI builds through `ProfileSource::{LegacyCloudObjects, SettingsCollection}`. [`profiles.rs (255-330, 500-719)`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/execution_profiles/profiles.rs#L255-L330) keeps reads and writes behind that source and performs a one-time GUI legacy import. Custom endpoints must follow this source-branch pattern.
- [`app/src/settings/ai.rs (1435-1468) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/settings/ai.rs#L1435-L1468) registers `agents.execution_profiles` with `max_table_depth: 2`. [`TuiStatuslineConfig`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/settings/ai.rs#L759-L850) confirms nested structured values are supported.
- [`app/src/terminal/input/slash_commands/data_source/core.rs (47-199) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/terminal/input/slash_commands/data_source/core.rs#L47-L199) keeps shared behavior and dependencies in one core while [`gui.rs`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/terminal/input/slash_commands/data_source/gui.rs#L34-L118) and [`tui.rs`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/terminal/input/slash_commands/data_source/tui.rs#L28-L80) retain only surface lifecycle and presentation. Custom endpoint logic uses the same core-plus-thin-adapter boundary.
- [`crates/ai/src/api_keys.rs (24-132) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/crates/ai/src/api_keys.rs#L24-L132) defines `ApiKeys`, `CustomEndpoint`, `CustomEndpointModel`, and `CustomEndpointSchema`. The existing model `config_key` is a UUIDv4 minted by GUI CRUD.
- [`crates/ai/src/api_keys.rs (278-382) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/crates/ai/src/api_keys.rs#L278-L382) loads `AiApiKeys` and persists provider-key changes. [`api_keys.rs (471-604)`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/crates/ai/src/api_keys.rs#L471-L604) mutates GUI custom endpoints and builds the request registry.
- [`app/src/lib.rs (505-527) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/lib.rs#L505-L527) appends `.tui` to the TUI secure-storage service. [`app/src/lib.rs (1458-1480)`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/lib.rs#L1458-L1480) registers that service before settings and `ApiKeyManager` initialization.
- [`app/src/ai/tui_api_keys.rs (10-45) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/tui_api_keys.rs#L10-L45) watches `api_keys.revision` and reloads `ApiKeyManager` after another process changes TUI secrets.
- [`crates/warp_tui/src/api_keys_menu.rs (28-721) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/crates/warp_tui/src/api_keys_menu.rs#L28-L721) implements fixed provider rows, masked editing, `(Connected)` state, `ctrl + x` clearing, filtering, and Warp credit fallback.
- [`crates/warp_tui/src/inline_menu.rs (1340-1478) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/crates/warp_tui/src/inline_menu.rs#L1340-L1478) renders row descriptions and state suffixes. It does not have an italic description style today.
- [`app/src/settings_view/custom_inference_modal.rs (1032-1104) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/settings_view/custom_inference_modal.rs#L1032-L1104) owns the current HTTPS and restricted-literal-host validation. That validation is GUI-private today.
- [`app/src/ai/llms.rs (1949-1997) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/llms.rs#L1949-L1997) builds synthetic custom `LLMInfo` values only for endpoints with a URL and key. [`llms.rs (1151-1160)`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/llms.rs#L1151-L1160) applies both entitlement and team policy to picker choices.
- [`app/src/ai/agent/api.rs (316-325) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/ai/agent/api.rs#L316-L325) currently passes only `is_custom_inference_enabled` into the custom-provider request builder. The triage claim that the request path also applies `are_member_byo_endpoints_allowed` was imprecise; this change must close that gap.
- [`app/src/settings/init.rs (236-281) @ 4cd1c77`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/app/src/settings/init.rs#L236-L281) reloads settings and emits `SettingsErrors`. [`crates/warp_tui/src/terminal_session_view.rs (1455-1478)`](https://github.com/warpdotdev/warp/blob/4cd1c77c498821785baf0801bbd026f3693d2544/crates/warp_tui/src/terminal_session_view.rs#L1455-L1478) turns that event into the existing TUI error hint.

## Proposed changes

### 1. Add a shared endpoint-definition core and the TUI settings collection

Add `crates/ai/src/custom_endpoints.rs` as the surface-neutral owner of:

- `CustomEndpointDefinitionsConfig`.
- `CustomEndpointDefinitionFile`.
- `CustomEndpointModelDefinitionFile`.
- Per-entry parsing and diagnostics.
- Definition validation.
- Deterministic model identity derivation.
- Definition/key joining and orphan reconciliation.

Add `settings_value`, `schemars`, `indexmap`, and `url` dependencies to `crates/ai` as required by the shared file representation and validator. `sha2` already exists there.

Use an `IndexMap<String, CustomEndpointDefinitionFile>` file shape. Do not prefix shared types or helpers with `Tui`; the TUI is only the first surface to select this format.

Register one setting:

```rust
custom_endpoints: CustomEndpointDefinitions {
    type: CustomEndpointDefinitionsConfig,
    default: CustomEndpointDefinitionsConfig::default(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Never,
    surface: settings::SettingSurfaces::TUI,
    private: false,
    toml_path: "cloud_platform.custom_endpoints",
    max_table_depth: 2,
    description: "Custom LLM endpoint definitions. API keys are managed in /api-keys.",
}
```

The map key is the endpoint name. Do not add a second `name` or `id` field. `models` is a `Vec<CustomEndpointModelDefinitionFile>` so the writer emits inline model records under each endpoint table, matching the `PRODUCT.md` example.

Use file-only types instead of adding `api_key: Option<_>` to a shared serializable definition. Mark endpoint and model file structs with `deny_unknown_fields`. This rejects an accidental plaintext `api_key`, a manually generated `config_key`, and misspelled fields.

`CustomEndpointDefinitionsConfig::from_file_value` parses the top-level object one entry at a time and stores:

- Valid endpoint definitions in file order.
- Invalid endpoint diagnostics keyed by the original map key.
- The set of names present in the file, including invalid entries.

Its `to_file_value` serializes only valid definitions. Writes for `cloud_platform.custom_endpoints` must be inhibited while invalid diagnostics exist so an unrelated in-process setting write cannot erase the user's broken entry. `/modify-settings` remains free to repair the file directly.

The JSON schema describes a string-keyed map of endpoint records and excludes runtime diagnostics. Add schema tests that trace the exact `cloud_platform.custom_endpoints` path and verify the setting is TUI-only.

### 2. Share validation and convert definitions

Move the URL validator and restricted-IP helpers from `custom_inference_modal.rs` into `crates/ai/src/custom_endpoints.rs`. Define one surface-neutral `validate_custom_endpoint_definition` entry point. The GUI modal adapts its legacy `CustomEndpointParams` into that validator and continues validating the API key separately. The settings parser calls the same entry point. This prevents URL, schema, name, and model rules from drifting before the GUI storage migration.

Validate one endpoint definition as a unit:

- Name is non-empty, already trimmed, and unique through the map.
- URL passes the existing GUI validator.
- Schema deserializes to `CustomEndpointSchema`.
- At least one model exists.
- Each model name is non-empty, already trimmed, and unique within that endpoint.
- A non-empty alias is already trimmed. An empty alias remains valid and falls back through `display_label`.
- Unknown fields fail deserialization.

The shared core converts every valid file definition to an existing `CustomEndpoint` with a supplied API key and derived model `config_key`. This preserves `LLMPreferences`, GUI picker metadata, and request-wire code as the common downstream representation.

### 3. Derive deterministic per-model `config_key` values

Add a pure helper in `crates/ai/src/custom_endpoints.rs`:

```rust
fn settings_custom_model_config_key(endpoint_name: &str, model_name: &str) -> String;
```

Compute the same value on every settings-backed surface:

```text
"custom-endpoint:v1:" + lowercase_hex(
  SHA-256(
    "warp.settings.custom_endpoint.config_key.v1\0"
    || u64_be(endpoint_name_utf8_length)
    || endpoint_name_utf8
    || u64_be(model_name_utf8_length)
    || model_name_utf8
  )
)
```

The domain separator and length prefixes prevent tuple-boundary ambiguity. SHA-256 collision risk is negligible. Exact duplicate model names are rejected before derivation.

Consequences:

- Endpoint-name or model-name changes produce a new ID.
- Alias, URL, schema, key, and model-order changes preserve the ID.
- Nothing writes the ID to `settings.toml`.
- Existing `LLMId` and request `config_key` fields accept strings; no downstream UUID parsing exists on the researched client path.
- The version prefix permits a future derivation change without mistaking two schemes.

Do not clear an unresolved ID from the execution profile. `LLMPreferences` already falls back when an ID does not resolve and deliberately preserves unknown profile IDs for QUALITY-866 cross-device safety. Changing back to the previous endpoint/model name can therefore restore the selection.

### 4. Select the persistence source once

Add `CustomEndpointSource` in a thin `app/src/ai/custom_endpoints.rs` coordinator, following `ProfileSource`:

```rust
enum CustomEndpointSource {
    LegacySecureBlob,
    SettingsCollection,
}
```

`CustomEndpointSource::for_launch_mode` selects:

- `SettingsCollection` for `LaunchMode::Tui`.
- `LegacySecureBlob` for GUI and test launches in v1.
- The existing behavior for launch modes that do not expose member custom endpoints.

Do not scatter `LaunchMode::Tui` checks through parsing, joining, picker, or request code. Select the source in `app/src/lib.rs`, map it once to `ApiKeyManager`'s crate-level `CustomEndpointPersistenceMode::{Monolithic, Split}`, and pass settings definitions through the coordinator. This is the execution-profile pattern: one model serves both surfaces while the source branch owns persistence differences.

The planned GUI follow-up can add a feature-gated GUI `SettingsCollection` selection and a one-time `LegacySecureBlob` import without changing validation, identity derivation, joining, picker, or request code.

### 5. Share split persistence, joining, and reconciliation

In `SettingsCollection` mode, use a second entry in the active surface's existing secure-storage service:

```text
CustomEndpointApiKeys = JSON object { "<endpoint name>": "<api key>", ... }
```

The active service provides isolation: TUI launches use the existing `.tui` service, while a future GUI migration uses the GUI service. Do not encode a surface name in the secure-storage entry or data type.

Maintain these source-neutral states:

- Endpoint definitions from `AISettings`.
- The name-to-key secure map.
- The effective `ApiKeys` projection returned by `keys()` and consumed by existing picker and request code.

At settings-backed startup, the shared coordinator:

1. Loads built-in provider keys from `AiApiKeys`.
2. Loads `CustomEndpointApiKeys`.
3. Reads valid definitions and present-name diagnostics from `AISettings::custom_endpoints`.
4. Joins definitions and keys by exact endpoint name.
5. Derives every model `config_key`.
6. Populates the effective `keys.custom_endpoints`.

Pass the startup file-parse state from `GlobalResourceHandlesProvider` when selecting `SettingsCollection`. If the settings file failed to parse, compose no definitions and skip orphan cleanup. This keeps recoverable keys until a successfully parsed settings document provides an authoritative name set.

Add result-returning methods with no surface prefix:

- `persist_custom_endpoint_key(name, Option<String>, ctx)`.
- `custom_endpoint_key_is_connected(name)`.

Persist the key map before publishing an in-memory key change. On success, rebuild the effective projection and emit `ApiKeyManagerEvent::KeysUpdated`. On failure, retain the old projection and return an error to the calling surface.

In `LegacySecureBlob` mode:

- Continue loading and saving complete GUI `CustomEndpoint` values in `AiApiKeys`.
- Keep `add_custom_endpoint`, `save_custom_endpoint`, `remove_custom_endpoint`, and `clear_custom_endpoints` behavior unchanged.
- Continue using persisted GUI UUID `config_key` values.

Route every `AiApiKeys` write through the selected source. `SettingsCollection` writes serialize `custom_endpoints: []`, so composed definitions cannot leak into the provider-key blob.

Subscribe the thin `app/src/ai/custom_endpoints.rs` coordinator, not `tui_api_keys.rs`, to `AISettingsChangedEvent::CustomEndpointDefinitions`. The coordinator passes the updated shared config into `ApiKeyManager`; the AI crate runs the common reconciliation. On a successfully parsed settings change:

1. Replace the definition set.
2. Keep keys for valid names.
3. Keep keys for names that are present but invalid, so correcting a typo restores the connection.
4. Remove keys absent from both valid and invalid names. This covers delete and rename.
5. Persist a changed key map and rebuild effective endpoints.
6. Emit `KeysUpdated`.

If orphan cleanup fails, remove the endpoint from the effective projection, report the secure-storage error, and retry cleanup on the next startup or definition reload. The inaccessible orphan must not enter a request.

`reload_keys_from_secure_storage` follows the selected source. In `SettingsCollection` mode it reloads `AiApiKeys` and `CustomEndpointApiKeys`, then calls the same shared join function used at startup and settings reload.

Keep `app/src/ai/tui_api_keys.rs` limited to TUI process coordination. After every successful TUI custom-endpoint set, replace, or clear, call the existing `notify_tui_api_keys_changed`. The current process has already updated its singleton; the revision write makes other TUI processes reload. A revision-write failure does not roll back a securely persisted mutation. Report the failure and show `The API key changed, but other running Warp processes could not be notified.`

Do not reconcile or clean orphans after a full-file parse error. Live reload exits before settings models change, and settings-backed startup receives the parse-error state explicitly.

### Shared and surface-specific responsibilities

Shared in `crates/ai`:

- Definition types and file schema.
- Per-entry parse diagnostics and validation.
- Deterministic settings-backed identities.
- Definition/key join and orphan classification.
- Monolithic versus split secure persistence.
- Effective `CustomEndpoint` projection, reload, and request registry.

Shared in `app`:

- One `CustomEndpointSource` launch decision.
- Existing `LLMPreferences` synthesis, entitlement/team-policy gates, and request attachment.

Surface-specific:

- GUI v1 keeps modal authoring and legacy monolithic CRUD. It adapts to the shared validator.
- TUI authors definitions through `settings.toml` or `/modify-settings`.
- TUI `/api-keys` presents key actions and policy states.
- TUI revision-file wiring notifies sibling TUI processes.
- GUI and TUI render their own rows, but both consume the same effective endpoint projection and `Custom · <endpoint>` description.

### 6. Preserve endpoint-level settings diagnostics

The generic `SettingsValue` contract returns one value or `None`; it cannot return a valid subset plus diagnostics. Keep the special handling in the shared custom-endpoint config instead of broadening every setting.

Add a helper in `app/src/settings/init.rs` that appends each invalid endpoint path to startup and hot-reload `InvalidSettings` keys:

```text
cloud_platform.custom_endpoints."<endpoint name>"
```

Call it after `validate_all_public_settings` and after `reload_all_public_settings`. Inhibit writes for the `cloud_platform.custom_endpoints` setting while any endpoint diagnostic exists.

The existing `WarpConfigUpdateEvent::SettingsErrors` path then shows the standard TUI invalid-values hint. Expose endpoint diagnostics through `AISettings::custom_endpoints` so `/api-keys` can also render the specific `(Skipped)` rows required by `PRODUCT.md`.

Do not retain the last-known-good version of an endpoint that is now a typed-invalid entry. Typed-invalid entries fail closed immediately. A full TOML syntax error still retains the prior document through `TomlBackedUserPreferences::reload_from_disk`.

### 7. Extend `/api-keys`

Change `TuiApiKeysRow` from static `Copy` data to owned, cloneable data. Add row kinds:

- `Provider(LLMProvider)`.
- `CustomEndpoint(String)`.
- `CustomEndpointStatus(CustomEndpointStatusKind)`.
- `InvalidCustomEndpoint(String)`.
- `WarpCreditFallbackSetting`.

Extend browsing, editing, footer, clear, and save states to carry a custom endpoint name. Custom endpoint set/edit/clear calls the shared `persist_custom_endpoint_key` API. Keep the editor masked and preserve current provider error copy.

Build rows in this order:

1. Existing built-in providers.
2. Valid custom endpoints sorted by name, or one entitlement/policy/empty status row.
3. Invalid endpoint rows sorted by name.
4. Warp credit fallback.

Subscribe the open menu to:

- `ApiKeyManagerEvent` for key and effective-definition changes.
- `AISettingsChangedEvent::CustomEndpointDefinitions` for invalid-only changes.
- `UserWorkspacesEvent::TeamsChanged` for entitlement and team-policy changes.

Add a dedicated inline-menu row style that renders the custom endpoint description in `builder.key_connected_suffix_style()` (muted italic) while preserving the connected state suffix. Do not encode formatting with literal underscores.

### 8. Refresh the model picker and requests

The joined effective `ApiKeys` projection lets the current `build_custom_llm_infos` filter continue to exclude unkeyed endpoints. `ApiKeyManagerEvent::KeysUpdated` already rebuilds `LLMPreferences.custom_llms`, and `TuiModelMenuModel` already refreshes on `LLMPreferencesEvent::UpdatedAvailableLLMs`.

For a custom `LLMInfo`, carry its existing GUI description (`Custom · <endpoint>`) into the TUI model row. Suppress the generic `(key connected)` suffix for custom endpoint models because `build_custom_llm_infos` already excludes unkeyed endpoints. Keep hosted/provider-key model rows unchanged.

Change the request gate in `app/src/ai/agent/api.rs` to:

```rust
let include_member_custom_endpoints =
    user_workspaces.is_custom_inference_enabled(app)
        && user_workspaces.are_member_byo_endpoints_allowed();
let custom_model_providers = api_key_manager
    .custom_model_providers_for_request(include_member_custom_endpoints);
```

Use the same combined predicate in `/api-keys` and `LLMPreferences`. This prevents a stale selected model or retained key from bypassing workspace policy.

### 9. Expected files

- `crates/ai/src/custom_endpoints.rs` and tests — shared config types, validation, deterministic identity, join, reconciliation, and effective projection.
- `crates/ai/Cargo.toml` — file-format and validation dependencies.
- `app/src/ai/custom_endpoints.rs` and tests — source selection plus `AISettings`/launch-mode adapter.
- `app/src/settings/ai.rs` and tests — setting registration, schema exposure, and partial diagnostics.
- `app/src/settings/init.rs` and tests — endpoint-level invalid-settings aggregation.
- `app/src/settings_view/custom_inference_modal.rs` and tests — shared validation call.
- `crates/ai/src/api_keys.rs` and tests — monolithic/split persistence selection, secure writes, and reload.
- `app/src/ai/tui_api_keys.rs` and tests — TUI revision notification and reload only.
- `app/src/lib.rs` — select and initialize `CustomEndpointSource`.
- `app/src/ai/llms.rs` and tests — effective custom models and stale-selection fallback coverage.
- `app/src/ai/agent/api.rs` and tests — combined entitlement and team-policy request gate.
- `crates/warp_tui/src/api_keys_menu.rs` and tests — dynamic rows and custom key actions.
- `crates/warp_tui/src/inline_menu.rs` and tests — muted italic custom-endpoint annotation.
- `crates/warp_tui/src/model_menu.rs` and tests — endpoint description on custom models.

## Decisions

### Use one core with source-selected persistence

- **Chosen:** Put format, validation, identity, join, reconciliation, and projection logic in `crates/ai`, with one app-level source selector and thin surface adapters.
- **Advantages:** GUI legacy and TUI settings paths share rules now. A future GUI migration selects the settings source and imports data instead of replacing a TUI subsystem.
- **Disadvantages:** V1 introduces a source abstraction before the GUI uses both branches.
- **Rejected:** Keep the join and lifecycle in `tui_api_keys.rs`. This is smaller for v1 but would force the GUI migration to duplicate or move the core later.
- **Rejected:** Migrate GUI storage in v1. That adds duplicate-name and selected-model identity migration to an already large TUI change.

### Use a surface-neutral split-secret key

- **Chosen:** Store settings-backed endpoint keys under `CustomEndpointApiKeys` in the active surface's secure-storage service.
- **Advantages:** Existing service namespaces preserve GUI/TUI isolation. Future GUI migration can use the same persistence code and schema.
- **Disadvantages:** The source mode, not the entry name, tells readers which surface owns the data.
- **Rejected:** `TuiCustomEndpointApiKeys`. This bakes the first adopter into shared storage code and requires a second GUI path.

## Risks and mitigations

### A partial setting can be rewritten without invalid entries

The typed value contains only valid endpoint definitions. An ordinary setting write could serialize that subset and erase broken entries. Inhibit writes for the complete `cloud_platform.custom_endpoints` key whenever diagnostics exist.

### Settings-backed mode can accidentally reserialize endpoint definitions into secure storage

Current provider-key persistence clones and serializes the full `ApiKeys` value. Route all `AiApiKeys` writes through the selected source and strip `custom_endpoints` in `SettingsCollection` mode. Assert that endpoint URL, schema, and models are absent from both secure blobs for a settings-backed surface except the key map's names.

### Rename cleanup can destroy a key during an incomplete edit

A valid TOML save that changes the map key is intentionally a rename and removes the old key. A typed-invalid entry retains its name in `present_names`, preventing cleanup while the user repairs its fields. A full TOML syntax error never replaces the last parsed document.

### Deterministic IDs change on user-visible names

This is the deliberate cost of avoiding user-authored stable IDs. The model picker falls back when an old selection no longer resolves. The product spec makes re-key and selection consequences explicit.

### Policy changes can leave stale in-memory selections

The combined gate hides custom choices and suppresses the request registry. Existing model resolution falls back when the selected ID is unavailable. Test entitlement and team policy independently.

### Shared core can still fork through surface adapters

Keep all definition parsing, validation, derivation, joining, reconciliation, and effective projection in `crates/ai/src/custom_endpoints.rs`. GUI and TUI adapters may select a source and present actions, but they must not reimplement a core rule. Add equivalence tests that pass the same definition through the legacy GUI validation adapter and settings parser.

## Testing and validation

### Unit and model tests

- `cargo nextest run -p ai -E 'test(api_keys|custom_endpoints)'`
  - Source-aware writes preserve legacy GUI behavior and strip definitions in settings-backed mode.
  - Set, replace, clear, and secure reload are atomic.
- `cargo nextest run -p warp -E 'test(custom_endpoint|custom_inference|settings)'`
  - The schema is TUI-only and omits `api_key`, `config_key`, and `id`.
  - Valid and invalid entries load independently.
  - File definitions join only by exact name.
  - Invalid-present names retain keys; absent names remove keys.
  - Deterministic IDs are stable and tuple-boundary-safe.
  - Endpoint/model rename and alias/URL/schema/reorder cases match `PRODUCT.md` Behaviors 27–30.
  - Every URL and model validation rule in `PRODUCT.md` Behavior 9 is covered.
  - The GUI legacy adapter and settings parser produce equivalent validation results.
  - `CustomEndpointSource` selects legacy GUI and settings-backed TUI behavior in v1.
  - Successful TUI mutations write the revision marker; revision failures preserve the committed key and return the partial-success error.
  - Startup and hot reload emit endpoint-specific invalid-settings paths.
  - GUI modal validation remains unchanged.
  - Picker rebuild and combined request gate match Behaviors 18–26.
- `cargo nextest run -p warp_tui -E 'test(api_keys_menu|model_menu|settings_reload)'`
  - Assert row order, filtering, italic style cells, status and invalid rows.
  - Assert masked set/edit, cancel, replace, empty-save, clear, and persistence failures.
  - Assert entitlement and team-policy transitions.
  - Assert custom model endpoint descriptions replace the redundant key-connected state.

### Repository checks

- `./script/format --check`
- `cargo clippy -p ai -p warp -p warp_tui --all-targets`
- `cargo nextest run -p ai -p warp -p warp_tui`

### Live TUI proof

Use `./script/run-tui` and the `tui-verify-change` workflow in a real PTY.

1. Start with the `PRODUCT.md` TOML example and no key.
2. Record `/api-keys`, setting a key, the model picker, clearing the key, and the resulting picker removal.
3. Record one invalid-entry recovery and one policy-disabled state.
4. Capture the screen through `tmux capture-pane`.
5. Produce a short asciinema/agg MP4 and a still image of the rendered TUI.
6. Attach the video and image to the implementation PR as verification artifacts.

The live run must also inspect one outbound request and confirm that the selected model's endpoint URL, schema, raw model name, deterministic `config_key`, and key are present only while both policy gates allow it.

## Parallelization

Use one implementation PR on the existing APP-5380 branch.

1. The lead implementer first lands `crates/ai/src/custom_endpoints.rs`, the app-level source selector, settings types, shared validator, deterministic key helper, and persistence-aware `ApiKeyManager` API. These interfaces are prerequisites.
2. After that foundation compiles, a local `tui-menu` child can work in `/workspace/warp-app-5380-tui-menu` on branch `factory/app-5380-tui-menu`. It owns `crates/warp_tui/src/api_keys_menu.rs`, `inline_menu.rs`, `model_menu.rs`, and their tests. It returns a commit for the lead to cherry-pick.
3. In parallel, the lead owns the shared core, `app/src/settings/**`, `app/src/ai/**`, `crates/ai/**`, policy gating, source integration, and storage tests.
4. The lead integrates the TUI commit, runs all repository checks, and performs the live TUI proof. Do not open separate implementation PRs.

## Follow-up architecture

A later, separately approved GUI migration should:

1. Gate `LaunchMode::App` onto `SettingsCollection`.
2. Read legacy complete endpoints from `AiApiKeys`.
3. Resolve duplicate endpoint names before materializing the map.
4. Write non-secret definitions into the GUI settings file and keys into `CustomEndpointApiKeys`.
5. Preserve or rewrite execution-profile and per-pane model selections from legacy UUID `config_key` values to deterministic settings identities.
6. Mark migration complete before stripping legacy endpoint values, with a rollback window modeled on file-backed execution profiles.

This follow-up reuses the v1 parser, validator, derivation, join, reconciliation, effective projection, picker, and request paths. It adds only the GUI source gate, one-time import, selection rewrite, and GUI authoring adapter.

## Explicit non-goals

- No GUI storage migration in v1; the shared architecture intentionally prepares a separate follow-up.
- No GUI-to-TUI endpoint import.
- No credential migration through `tui-migrate-setup`.
- No team-managed or enterprise BYOE.
- No server or protocol change.
- No multi-field endpoint editor in `/api-keys`.
