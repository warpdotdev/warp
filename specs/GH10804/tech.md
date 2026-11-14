# Automatic Model Fallback Opt-Out — Tech Spec
Product spec: `specs/GH10804/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/10804

## Problem
Warp's client already lets users choose an Agent profile base model, but the server can still fall back to another model when that selected model is unavailable. The implementation needs a profile-level preference that is persisted and sent with every relevant agent request, plus server support that treats the preference as an instruction not to route to a fallback model.

This is not safely implementable as a client-only post-processing change. By the time the client receives `ModelUsed { is_fallback: true }` metadata, a fallback model may already have generated text or tool calls. The no-fallback decision must be available to the server before generation starts.

## Relevant code
- `app/src/ai/execution_profiles/mod.rs (215-264)` — `AIExecutionProfile` stores profile model and permission settings, derives serde with `#[serde(default)]`, and defines default values.
- `app/src/ai/execution_profiles/profiles.rs (495-604)` — setters for `base_model`, `coding_model`, `cli_agent_model`, `computer_use_model`, and `context_window_limit` show the existing profile edit + telemetry pattern.
- `app/src/ai/execution_profiles/profiles.rs (1151-1325)` — `edit_profile_internal` persists profile edits to Warp Drive cloud objects and handles unsynced default profiles.
- `app/src/ai/execution_profiles/editor/mod.rs (149-258)` — `ExecutionProfileEditorViewAction` and editor state handles for profile controls.
- `app/src/ai/execution_profiles/editor/mod.rs (1386-1585)` — profile editor action handling for model changes and permission toggles.
- `app/src/ai/execution_profiles/editor/ui_helpers.rs (216-282)` — `render_models_section` renders the Base model dropdown and related model controls.
- `app/src/ai/execution_profiles/editor/ui_helpers.rs (860-987)` — existing switch rows for Plan auto-sync and web search provide a pattern for a profile-scoped boolean switch.
- `app/src/settings_view/ai_page.rs (3959-4059)` — Settings > Agents > Profiles renders profile cards and opens the profile editor; legacy models UI remains nearby for feature-flagged layouts.
- `app/src/ai/llms.rs (527-630)` — `LLMPreferences` stores available models and resolves the active base model from terminal overrides or the active profile.
- `app/src/ai/llms.rs (782-840)` — `update_preferred_agent_mode_llm` stores a terminal-view model override while still deriving the profile default from `AIExecutionProfilesModel`.
- `app/src/ai/llms.rs (897-1049)` — model metadata refresh and `reconcile_disabled_model_preferences` preserve transient provider-outage selections but clear permanently unusable selections.
- `app/src/ai/blocklist/controller.rs (157-215)` — `RequestInput::new_with_common_fields` captures the selected base/coding/CLI/computer-use model IDs for each request.
- `app/src/ai/agent/api.rs (203-337)` — `AIAgentRequest::new` builds the `warp_multi_agent_api::Request` sent to `/ai/multi-agent`.
- `app/src/server/server_api.rs (1189-1265)` — `ServerApi::generate_multi_agent_output` posts the protobuf request to `/ai/multi-agent`.
- `app/src/ai/agent/mod.rs (383-404)` — `OutputModelInfo` records the model used and whether it was a fallback.
- `app/src/ai/agent/api/convert_conversation.rs (1868-1880)` — restored server `ModelUsed` messages are converted into `OutputModelInfo`.
- `app/src/ai/blocklist/block/status_bar.rs (803-842, 996-1105)` — current streaming status UI renders fallback-specific "Warping with ..." messaging when model-used metadata says `is_fallback`.
- `app/src/ai/agent_sdk/mod.rs (320-463, 828-881)` — local/cloud agent task config snapshots include model IDs and are used when launching ambient/CLI agent runs.
- `app/src/server/server_api/ai.rs (177-232)` — public/cloud `SpawnAgentRequest` accepts `AgentConfigSnapshot`; remote runs need an equivalent fallback preference in that config path.

## Current state
`AIExecutionProfile` stores explicit model preferences but has no fallback preference. `LLMPreferences::get_active_base_model` resolves the active base model from a terminal-view override first, then the active profile's `base_model`, and then the server-provided default. `RequestInput::new_with_common_fields` copies only resolved model IDs into a request. `AIAgentRequest::new` forwards those IDs to the multi-agent API.

The server can emit `ModelUsed` metadata with `is_fallback`, and the client uses that metadata to explain that a fallback model is being used. That path is useful for transparency, but it is too late to prevent fallback-generated output.

Profiles are stored as JSON cloud objects using serde defaults. This makes a new boolean field straightforward: missing fields on old profiles can default to the current behavior without a data migration.

Cloud and remote agent launch paths use `AgentConfigSnapshot` and server-side task creation. If only the local `/ai/multi-agent` request is updated, remote runs may still fall back silently, so this feature requires a server/API addition as indicated by the issue label.

## Proposed changes

### 1. Add a profile-level fallback preference
Add a boolean field to `AIExecutionProfile`, tentatively named:

- `pub automatic_model_fallback_enabled: bool`

Default it to `true` in every constructor:

- `Default::default`
- `create_agent_mode_eval_profile`
- `create_default_cli_profile`
- any test/helper constructors that build full profiles directly

Because `AIExecutionProfile` already uses `#[serde(default)]`, legacy profile JSON that lacks the field will deserialize with `true`, preserving current behavior. If any tests compare serialized profiles exactly, update the fixtures to include or tolerate the new field.

Add `AIExecutionProfilesModel::set_automatic_model_fallback_enabled(profile_id, enabled, ctx)` following the existing `set_web_search_enabled` and `set_autosync_plans_to_warp_drive` pattern:

- use `edit_profile_internal`
- emit `AIExecutionProfilesModelEvent::ProfileUpdated(profile_id)` via the existing edit path
- send `TelemetryEvent::AIExecutionProfileSettingUpdated { setting_type: "automatic_model_fallback_enabled", setting_value: format!("{enabled}") }`

### 2. Render the setting in the profile editor
Update `ExecutionProfileEditorView`:

- add `SetAutomaticModelFallback { enabled: bool }` to `ExecutionProfileEditorViewAction`
- add a `SwitchStateHandle` field, for example `automatic_model_fallback_switch`
- initialize it in `ExecutionProfileEditorView::new`
- handle the action by calling `AIExecutionProfilesModel::set_automatic_model_fallback_enabled`

Update `app/src/ai/execution_profiles/editor/ui_helpers.rs`:

- render a switch row in `render_models_section` immediately after the Base model dropdown and before the context window row
- label: "Automatic model fallback"
- description: "When your selected model is unavailable, Warp may use another model to keep the agent running."
- checked state: `profile_data.automatic_model_fallback_enabled`

Use the existing boolean-row style from `render_plan_auto_sync_toggle` / `render_web_search_toggle` so spacing and switch behavior match the profile editor. No new feature flag is required.

### 3. Expose the effective preference to request construction
Add a helper that resolves the active profile's fallback preference for a terminal view. The smallest implementation can read the active profile directly in `AIAgentRequest::new`, mirroring the existing `context_window_limit` block:

- `let profile_data = AIExecutionProfilesModel::as_ref(app).active_profile(terminal_view_id, app).data().clone();`
- `let automatic_model_fallback_enabled = profile_data.automatic_model_fallback_enabled;`

If multiple request builders need this value, add an accessor on `BlocklistAIPermissions` or `AIExecutionProfilesModel` rather than duplicating the active-profile lookup.

The effective value should be profile-scoped, not model-scoped. Terminal-view base model overrides from `LLMPreferences::base_llm_for_terminal_view` still use the active profile's fallback setting. This matches the product behavior where the setting lives on the profile and governs explicit model choices made while that profile is active.

### 4. Add a request/API field that the server enforces
Add a field to the multi-agent request schema, tentatively named:

- `automatic_model_fallback_enabled: bool`

or, if the server prefers positive "allow" semantics:

- `allow_model_fallback: bool`

The field must default to `true` on the server when omitted so older clients retain today's fallback behavior. The client should always send the field once supported.

Update the generated/request conversion path so `AIAgentRequest::new` sets the field from the active profile. The exact file depends on where `warp_multi_agent_api::Request` is generated, but the client-side set point is `app/src/ai/agent/api.rs (203-337)`.

Server behavior requirement:

- if the selected/requested concrete model is available, proceed normally
- if it is unavailable and fallback is allowed, preserve current fallback behavior
- if it is unavailable and fallback is disabled, return a typed error before generating output on any fallback model
- include enough error detail for the client to show the selected model display name when possible

The typed error should ideally use a stable code such as `MODEL_UNAVAILABLE_FALLBACK_DISABLED` rather than requiring string matching. If a transport-level error is used initially, update the client error classifier in the response-stream path to map it to a user-facing `RenderableAIError`.

### 5. Carry the preference through cloud/remote agent config
Remote/cloud agent runs are launched through task creation and `AgentConfigSnapshot`, not only through the local multi-agent request. Add the same preference to the task config path so cloud workers and remote child agents can enforce it:

- add an optional field to `AgentConfigSnapshot`, for example `automatic_model_fallback_enabled: Option<bool>` or `allow_model_fallback: Option<bool>`
- default omitted config to fallback enabled server-side
- when creating a task from the local client, populate the field from the same effective profile preference used for local requests
- when the user explicitly supplies an agent config file or public API config in the future, treat absence as enabled

Relevant launch code:

- `app/src/ai/agent_sdk/mod.rs (320-463)` — merged local/CLI config snapshot
- `app/src/ai/agent_sdk/mod.rs (828-881)` — `initialize_new_task` sends `AgentConfigSnapshot` to `create_agent_task`
- `app/src/server/server_api/ai.rs (177-232)` — `SpawnAgentRequest` carries public/cloud config
- `app/src/ai/blocklist/action_model/execute/start_agent.rs (198-428)` and `run_agents.rs (329-410)` — child-agent actions carry model IDs; if they create remote tasks, they need the fallback preference either in config or in the server-side request derived from parent context

If server ownership makes the cloud propagation larger than the initial client work, implement local request support first behind a server capability check but keep the profile UI disabled or hidden until the server can enforce the setting for all supported run modes. The product behavior requires server enforcement before shipping broadly.

### 6. Error handling and rendering
Extend the response-stream error path so the typed server error becomes a clear inline agent error:

- `app/src/ai/blocklist/controller/response_stream.rs (221-339)` currently retries retryable stream errors and emits final errors into the stream. The new model-unavailable/no-fallback error should not be retried automatically unless the server marks it retryable and no side effects occurred.
- Error copy should be generated from structured error data when possible: selected model display name, model ID, and reason.
- Child agent startup errors already flow through `start_agent_error_message_for_status` in `app/src/ai/blocklist/action_model/execute/start_agent.rs (522-557)`; ensure the selected-model-unavailable message is preserved there for failed child launches.

Do not attempt to cancel fallback after receiving `ModelUsed { is_fallback: true }`. That metadata should remain a transparency signal for the fallback-enabled path and a regression indicator in tests for the fallback-disabled path.

### 7. Telemetry and observability
Add or reuse telemetry so product and engineering can validate rollout:

- profile setting toggled: `AIExecutionProfileSettingUpdated` with `setting_type = "automatic_model_fallback_enabled"`
- request sent with fallback enabled/disabled, ideally as a field on existing agent request telemetry rather than a new high-cardinality event
- server-side event/count for fallback blocked by user preference, including model/provider identifiers that are already safe to log
- client-side error telemetry for selected-model-unavailable/no-fallback, separate from generic transport failures

Avoid logging prompt content, secrets, or full request payloads as part of this feature.

## End-to-end flow

### Fallback enabled
1. User selects a concrete Base model in an Agent profile.
2. `AIExecutionProfile.automatic_model_fallback_enabled` is `true`.
3. User submits a Warp Agent prompt.
4. `RequestInput::new_with_common_fields` resolves the active model IDs.
5. `AIAgentRequest::new` includes `automatic_model_fallback_enabled = true`.
6. Server tries the selected model.
7. If unavailable, server may route to fallback as it does today.
8. Client receives model-used metadata with `is_fallback = true` and continues to render existing fallback messaging.

### Fallback disabled
1. User turns "Automatic model fallback" off in the active profile.
2. Profile edit is persisted and synced like other profile settings.
3. User submits a Warp Agent prompt with a selected concrete model.
4. `AIAgentRequest::new` includes `automatic_model_fallback_enabled = false`.
5. Server tries the selected model.
6. If unavailable, server returns the typed selected-model-unavailable/no-fallback error before fallback generation.
7. Client renders the inline error and does not execute agent actions from a fallback model.
8. User retries, waits, enables fallback, changes model, or changes profile.

## Risks and mitigations

### Risk: client-only prevention is too late
If the client blocks only after seeing fallback metadata, fallback output and tool calls may already exist.

Mitigation: require a pre-generation request/config field and typed server enforcement. Treat fallback metadata under fallback-disabled requests as a regression.

### Risk: legacy profiles or old clients change behavior
New serde fields can accidentally default to `false`, disabling fallback for existing users.

Mitigation: default the field to `true` everywhere, rely on `#[serde(default)]`, and add deserialization tests for a legacy profile JSON without the field.

### Risk: local and cloud paths diverge
Local `/ai/multi-agent` requests and cloud/remote `AgentConfigSnapshot` launches have different plumbing. Updating one path would produce inconsistent behavior.

Mitigation: define both request and config fields up front, default omitted values to enabled, and include cloud/remote validation before broad rollout.

### Risk: "auto" model semantics are confused with fallback
Users may expect fallback off to make "auto" deterministic.

Mitigation: keep "auto" as an explicit automatic-routing choice, document that the setting controls fallback from concrete selected models, and make UI copy say "selected model" rather than "any model decision".

### Risk: transient outage reconciliation clears the selected model
If provider-outage metadata clears the profile's selected model, the next request may use the default model instead of surfacing the no-fallback error.

Mitigation: preserve the existing `DisableReason::ProviderOutage` behavior in `llms.rs`, where transient outages do not clear preferences, and add a regression test if this feature touches reconciliation logic.

## Testing and validation
- `AIExecutionProfile` unit tests:
  - default profile has `automatic_model_fallback_enabled = true`
  - legacy JSON without the field deserializes to `true`
  - setter persists `false` and then `true`
  - synced and unsynced default profile edit paths both emit profile-updated events
- Profile editor tests:
  - switch renders checked for default profiles
  - clicking switch dispatches `SetAutomaticModelFallback`
  - action handler updates the correct profile only
- Request construction tests:
  - active profile with fallback enabled sends/enqueues `true`
  - active profile with fallback disabled sends/enqueues `false`
  - terminal-view model override still uses the active profile setting
  - legacy/missing config values default to enabled
- Server/API tests:
  - unavailable selected model + fallback enabled uses existing fallback path
  - unavailable selected model + fallback disabled returns typed no-fallback error
  - available selected model + fallback disabled succeeds normally
  - omitted field behaves like enabled for backward compatibility
- Client error tests:
  - typed no-fallback error renders "{Model display name} is currently unavailable. Retry later or choose another model."
  - no model display name falls back to model ID
  - no-fallback errors do not trigger automatic retry after fallback generation
  - child-agent startup failure preserves the error message
- Manual validation:
  - toggle persists in Settings > Agents > Profiles
  - profile switching changes effective behavior
  - fallback-enabled outage still shows current fallback behavior
  - fallback-disabled outage shows inline error and no fallback output/tool calls
  - cloud/remote run receives the same preference once server support is deployed

## Follow-ups
- Update `docs.warp.dev` model-choice documentation to describe the opt-out and its default-on behavior when the feature ships.
- Consider adding CLI/config-file syntax for the same preference after the in-app profile setting and server enforcement are stable.
- Consider provider-specific fallback policies only if users ask for more granular control after this binary setting ships.
