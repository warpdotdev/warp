# gh-11963: Custom Endpoint Model Configuration

## Context

Product behavior is defined in `PRODUCT.md`. The current code has two separate
concerns that must not be collapsed:

- `crates/ai/src/api_keys.rs:15` stores AI API keys under secure-storage key
  `AiApiKeys`.
- `crates/ai/src/api_keys.rs:34-40` includes `custom_endpoints` in that secure
  blob.
- `crates/ai/src/api_keys.rs:42-60` defines `CustomEndpoint` and
  `CustomEndpointModel`. A custom model currently has only `name`, `alias`, and
  stable `config_key`.
- `crates/ai/src/api_keys.rs:257-312` adds and saves custom endpoints, then calls
  `write_keys_to_secure_storage`.
- `crates/ai/src/api_keys.rs:522-566` reads and writes the whole API-key payload
  through secure storage.
- `app/src/ai/llms.rs:149-177` defines `LLMContextWindow` and `LLMInfo`.
- `app/src/ai/llms.rs:1324-1358` builds synthetic `LLMInfo` values for custom
  models, but stubs `reasoning_level` and `context_window`.
- `app/src/ai/agent/api.rs:343-346` computes `context_window_limit` from the
  active Agent profile.
- `app/src/ai/agent/api/impl.rs:67-72` sends
  `base_model_context_window_limit`, falling back to `0`.
- `app/src/ai/agent/api/impl.rs:89` currently hardcodes
  `supports_reasoning_message: true`.
- `app/src/settings_view/custom_inference_modal.rs:33-48` defines the modal
  events that currently pass endpoint name, URL, API key, and model tuples.
- `app/src/settings_view/custom_inference_modal.rs:63-80` stores modal row state
  for model name, alias, remove button state, and `config_key`.
- `app/src/settings_view/ai_page.rs:1973-2045` opens the add/edit custom endpoint
  modal and handles its events.
- `crates/cloud_object_models/src/ai_execution_profile.rs:380-385` stores the
  selected model ids and optional profile context-window override.

One important implication: simply adding `context_window` to `LLMInfo` is not
enough. `context_window_limit_for_request` only returns a value when the active
profile has an explicit context-window override. A custom model's configured
default window must be threaded into request construction, or the request will
continue to send `0`.

The existing PR #12834 correctly identifies the model-metadata gap, but it treats
the secure `ApiKeys` blob as if it satisfied the `settings.toml` requirement.
That is not accurate and would miss a core part of issue #11963.

## Proposed Changes

### 1. Split public model configuration from secrets

Introduce a non-secret custom endpoint configuration surface in the settings
system, for example under an `agents.custom_endpoints` TOML path. The exact type
can live near the existing AI settings in `app/src/settings/ai.rs`, or in a small
module owned by AI settings if the shape is too large for that file.

The public config should contain:

- endpoint id
- endpoint display name
- base URL if product/security accepts it as non-secret
- model variant rows
- variant display name
- provider model slug
- stable variant id/config key
- optional context window
- optional reasoning metadata

The public config must not contain API keys. API keys stay in secure storage.
If base URLs are considered sensitive, store only endpoint identity and model
metadata in TOML and keep URLs with the secret payload.

### 2. Preserve and migrate existing endpoints safely

Existing secure-storage endpoints must continue to load. During the transition,
the runtime model list can be assembled from:

1. public settings metadata when present, plus matching secure secrets
2. legacy secure-storage endpoint rows when no public metadata exists

When a legacy endpoint is edited, write non-secret metadata to the new settings
surface and keep the API key in secure storage. Preserve the existing
`config_key` so Agent profiles do not lose their selected model.

Invalid public config should fail closed for the affected endpoint or variant:
report the settings error and do not include the malformed variant in request
settings. Do not silently reinterpret an invalid context window as a different
valid model configuration.

### 3. Model custom variants explicitly

Extend the custom model representation with optional metadata. The exact type
may be a new public settings type that converts into `CustomEndpointModel`, or
additional fields on `CustomEndpointModel` if the secure/public split keeps that
struct as the merged runtime representation.

Required runtime fields:

- `name` or `slug`: provider-facing model id
- `alias` or `display_name`: picker label
- `config_key`: stable Warp-facing id
- `context_window: Option<u32>`
- `reasoning: Option<CustomModelReasoningConfig>`

Use multiple variant rows for the same provider slug when users want different
reasoning settings or profile-specific behavior. This matches the existing model
picker pattern where different reasoning levels are represented as selectable
model choices.

### 4. Populate custom `LLMInfo` from metadata

Update `custom_llm_info_from` in `app/src/ai/llms.rs` so configured custom
variants no longer get all-zero metadata.

For context windows:

- If a custom variant has a configured context window, populate
  `LLMContextWindow` with that max/default value.
- Keep unknown values as `LLMContextWindow::default()` for backwards
  compatibility.

For reasoning:

- Map configured custom reasoning metadata into the same `reasoning_level`
  semantics used by hosted model variants when possible.
- Keep reasoning unset for custom variants that do not declare it.

Do not infer provider-specific capabilities from model name alone.

### 5. Send effective custom context and capabilities

Add an explicit request-time capability calculation instead of relying only on
the Agent profile context-window override.

Recommended shape:

- Add a small `ModelCapabilities` or `EffectiveModelMetadata` value to
  `RequestParams`.
- Compute it in `RequestParams::new` from the active model's `LLMInfo`.
- Include both hosted-model behavior and custom-model metadata in the helper.
- Use it in `app/src/ai/agent/api/impl.rs` for
  `base_model_context_window_limit` and `supports_reasoning_message`.

For custom models with a configured context window, the request should send that
value even when the profile has no user override. For hosted models, preserve
the existing override behavior.

### 6. Update settings UI and settings search

Update the custom endpoint add/edit flow to edit public metadata and secure
secrets through the appropriate storage path. The UI should support multiple
variants per endpoint, including duplicate provider slugs with different labels
or reasoning metadata.

Add search terms for custom endpoints, custom models, context window, reasoning,
thinking, temperature, and max tokens. Temperature and max-token controls should
remain disabled or absent until the request wire supports them.

### 7. Defer generation parameters until the wire supports them

Issue #11963 asks for temperature, max output tokens, and provider-specific
parameters. The current client request path has no confirmed field for these in
the custom model provider payload. Do not add runtime UI that appears to send
these values until the proto/API contract exists.

Follow-up sequence:

1. Add or confirm request fields in `warp-proto-apis`.
2. Bump the generated API dependency in this repo.
3. Extend the public settings schema and request conversion.
4. Add UI and validation for active generation-parameter controls.

## Testing and Validation

- Serde/settings tests for valid and invalid public custom endpoint config.
- Secure-storage tests proving API keys are not written to `settings.toml`.
- Migration tests for legacy secure-storage-only endpoints.
- Unit tests for custom model variant identity preservation across edits.
- Unit tests for `custom_llm_info_from` mapping context window and reasoning
  metadata.
- Request-construction tests proving configured custom context windows send a
  non-zero `base_model_context_window_limit` without requiring a profile
  override.
- Request-construction tests proving reasoning support is derived per selected
  variant, and hosted models remain unchanged.
- UI tests or focused view tests for adding/editing multiple variants with the
  same provider slug.

## Risks and Coordination

- Settings schema design must keep secrets out of TOML while still making
  non-secret metadata shareable.
- Existing secure-storage data should not be destructively migrated without a
  rollback path.
- Several in-flight PRs touch nearby custom-model and capability code. Recheck
  conflicts against #12775, #12808, and #11812 before implementation.
- The settings UI can become crowded. A design pass or mock should happen before
  implementation if product wants more than context window and reasoning in the
  first UI slice.

## Follow-ups

- Add active temperature, max-output-token, and provider-parameter support after
  the request wire can carry those values.
- Consider endpoint model discovery from `/v1/models` as an input source for the
  same public metadata shape.
- Decide whether endpoint base URLs are acceptable in `settings.toml` or should
  remain in secure storage with the API key.
