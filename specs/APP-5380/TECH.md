# Shared custom inference endpoints
## Context
The accepted behavior is defined in [PRODUCT.md](PRODUCT.md). At [`afd2aecd`](https://github.com/warpdotdev/warp/tree/afd2aecd461f73699318f0d87b576fda3c2a1511), [`crates/ai/src/api_keys.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/crates/ai/src/api_keys.rs) stores provider credentials and complete `CustomEndpoint` values in the `AiApiKeys` secure blob. [`app/src/settings_view/warp_agent_page.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/app/src/settings_view/warp_agent_page.rs) performs GUI endpoint CRUD by secure-blob vector index.

[`app/src/ai/llms.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/app/src/ai/llms.rs) synthesizes selectable custom models from those secure values, and [`app/src/ai/agent/api.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/app/src/ai/agent/api.rs) serializes them on agent requests. The TUI consumes the shared picker and controller but [`crates/warp_tui/src/api_keys_menu.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/crates/warp_tui/src/api_keys_menu.rs) exposes credentials only for fixed providers.

The implementation follows the file-backed execution-profile precedent in [`app/src/ai/execution_profiles/config.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/app/src/ai/execution_profiles/config.rs) and [`app/src/ai/execution_profiles/profiles.rs`](https://github.com/warpdotdev/warp/blob/afd2aecd461f73699318f0d87b576fda3c2a1511/app/src/ai/execution_profiles/profiles.rs): a validated stable-keyed settings collection, an explicit source/authority state, and one-time import that waits for settings reconciliation.
## Proposed changes
### File-backed definition model
Add `CustomEndpointDefinitions` as an ordered map from `CustomEndpointId` to `CustomEndpointDefinition`. Endpoint IDs accept non-empty ASCII alphanumeric, underscore, and hyphen characters. Definitions contain `name`, `base_url`, `schema`, and models containing `name`, optional `alias`, and explicit `config_key`.

Implement `SettingsValue` and JSON schema generation through a file-safe representation. Reject the complete collection if endpoint IDs or config keys are duplicated, required strings are empty after trimming, no models are present, aliases are explicitly empty, schemas are unsupported, or URLs fail the shared public-HTTPS validator.

Register the value on `AISettings` at `agents.custom_endpoints` with `SettingSurfaces::ALL`, global sync respecting the user setting, no privacy flag, and an empty default. GUI settings mode synchronizes definitions; TUI settings mode retains its existing local-file behavior.
### Credentials and effective endpoint coordinator
Keep the legacy `custom_endpoints` field temporarily for migration and rollback. Add a separate secure-storage map from stable endpoint IDs to API keys. Reads join the current definition setting with the current surface's secure map to produce existing runtime `CustomEndpoint` values.

Introduce a shared coordinator under `app/src/ai` that owns source authority, settings/error subscriptions, migration, secret reconciliation, and effective endpoint events. `ApiKeyManager` remains the secure-storage owner and exposes keyed-secret CRUD plus request serialization from supplied effective definitions.

Move the GUI URL validator into the shared endpoint domain module so file input and the existing modal apply one policy.
### GUI source migration
TUI uses the settings source immediately and never reads the GUI namespace. GUI switches directly to the settings source after this one-time import:

1. If the new setting is explicit, it is authoritative.
2. Otherwise wait for initial cloud-preference reconciliation.
3. If the setting remains absent, import each legacy secure endpoint.
4. Preserve every model `config_key` exactly. Reject an invalid legacy collection rather than partially importing it.
5. Derive deterministic legacy endpoint IDs from the endpoint's legacy index, name, URL, and model config keys so retries produce the same mapping.
6. Persist the keyed secret map before the definition collection.
7. Retain the legacy secure objects for the rollback window. A later cleanup removes them after the new source is stable.

Convert GUI add/edit/remove to stable endpoint IDs. Adds generate endpoint and model IDs, edits preserve them, and removes make the definition unavailable before best-effort secret cleanup.
### Picker and request plumbing
Replace direct reads of `ApiKeys.custom_endpoints` in `LLMPreferences`, GUI settings, and request construction with `ApiKeyManager`'s effective joined endpoint snapshot. Settings and credential changes emit `ApiKeyManagerEvent`; existing workspace events re-evaluate entitlement and team policy.

Only synthesize picker entries from valid definitions with non-empty local keys. Attach custom providers only when both `is_custom_inference_enabled` and `are_member_byo_endpoints_allowed` are true.

Subscribe to settings-file error state. If `agents.custom_endpoints` is invalid, publish an empty effective registry immediately and retain credentials. On error clearance, rebuild even if the typed setting value equals the value held before the parse failure.
### TUI surfaces
Extend `/api-keys` row identity with stable custom endpoint IDs. Build rows dynamically from the coordinator snapshot, sort by display name, and reuse the current masked editor, error header, and clear action. Omit custom-endpoint rows when the definition collection is absent or empty, and add an aggregate invalid-setting row.

Add a zero-state Custom endpoints section next to the existing MCP status section only when definitions exist or the explicit setting is invalid. Subscribe to coordinator changes and render compact status copy through semantic TUI styles. The text remains informational; it does not introduce click or focus behavior.
## Testing and validation
- Shared-domain tests cover secret-free serialization, URL and collection validation, duplicate config-key rejection, deterministic legacy IDs, config-key preservation, keyed-secret joins, invalidation, recovery, and credential clearing.
- Existing app model tests cover picker synthesis, key requirements, config-key resolution, removal, and cloud-agent exclusion. Request policy remains covered by the existing scoped BYOK/BYOE workspace-policy tests.
- `/api-keys` tests cover omission when unconfigured, the invalid aggregate row, dynamic user-named rows, sorting, key replacement, and key clearing.
- Zero-state tests cover omission when unconfigured plus missing-key, mixed, connected, and invalid-setting status.
- A generic OpenAI-compatible fixture named OpenRouter verifies that no provider-specific code path is required.

Run focused `cargo nextest` suites for `ai`, `warp`, and `warp_tui`; `./script/format`; targeted clippy for changed crates and targets; and a live `./script/run-tui` verification that unconfigured endpoints are omitted while needs-key, connected, and invalid-setting states update without a restart.
## Risks and mitigations
- Settings and secure-storage writes are not transactional. Write secrets first, make definitions authoritative second, retain legacy data during rollout, and make every step retryable.
- Stale valid settings may remain in the generic settings model after parse errors. The coordinator independently tracks settings-file errors and empties the effective registry.
- Existing selections depend on model config keys. Migration and GUI edits preserve them, with dedicated regression tests.
- GUI and TUI settings and secrets are intentionally isolated despite using one schema. Documentation and zero-state copy must not imply automatic credential sharing.
## Parallelization
Implementation remains single-threaded. The setting types, migration state, secure-key join, GUI CRUD, picker, and TUI statuses depend on the same evolving APIs, so parallel worktrees would create high-conflict integration and validation work. Independent validation can run concurrently after the implementation stabilizes.
