# QUALITY-1449: Custom endpoint reasoning level in the Warp client

## PRODUCT

**Summary:** Let a user configure an optional reasoning level for each model on a personal
custom endpoint in the desktop Settings modal. The choice is stored with that model in the
existing secure-storage payload and sent with the model on every eligible multi-agent request.
This is the `warpdotdev/warp` slice only; protocol definition and server-side request mapping are
owned by the companion `warp-proto-apis` and `warp-server` specs.

**Working shared assumptions:** The cross-repo product contract supplied for this spec uses
per-custom-model ownership, the explicit values `none`, `low`, `medium`, `high`, and `xhigh`, and
an unset state that maps to protobuf `REASONING_LEVEL_UNSPECIFIED = 0` and preserves today's
behavior. Proto3 omits that default-valued scalar from encoded bytes; explicit `none` is the
distinct `REASONING_LEVEL_NONE = 1`. The client must keep the option registry and
schema-compatibility table centralized so a value-set decision from the lead `warp-server` spec
is a small edit rather than a UI/data-flow rewrite.

**Key design choices:** Store a typed optional level on `CustomEndpointModel`; expose a
`Reasoning` dropdown inside each model row group with `Provider default` as the unset choice;
retain an incompatible value visibly and block Save rather than silently clearing it; and keep
runtime tool compatibility on the server because the settings modal cannot know the final tool
payload. Do not populate `LLMInfo.reasoning_level`, whose existing meaning is a server-provided
model variant rather than configuration attached to one custom-model ID.

### Behavior

1. Each model row in Add/Edit custom endpoint has its own `Reasoning` control. The choices are
   `Provider default`, `None`, `Low`, `Medium`, `High`, and `Extra high`; the last five serialize
   as `none`, `low`, `medium`, `high`, and `xhigh`. `Provider default` is selected for a new row.
2. Reasoning is per model, not per endpoint. Two models under one endpoint may be saved with
   different levels, and adding, removing, or reordering rows cannot transfer a level or stable
   `config_key` to another model.
3. `Provider default` means unspecified, not `none`. Saving it stores no explicit local level and
   sets the generated protobuf scalar to `REASONING_LEVEL_UNSPECIFIED`; proto3 omits that zero
   value from encoded bytes. `None` sets `REASONING_LEVEL_NONE` and must round-trip distinctly.
4. A legacy secure-storage payload with no model reasoning field loads every endpoint, key,
   schema, model name, alias, and `config_key` unchanged, with `Provider default` shown for each
   model. An unknown future stored value must degrade to the unset behavior without causing the
   entire `AiApiKeys` payload to fail deserialization.
5. Reopening an endpoint restores each model's explicit choice. Editing unrelated endpoint or
   model fields preserves the choice; changing the choice preserves the existing `config_key`.
6. Compatibility is driven by one client-side option table keyed by `CustomEndpointSchema`.
   Under the current working contract, `xhigh` is unavailable for OpenAI Chat Completions and is
   available for OpenAI Responses and Anthropic Messages; the remaining values and unset are
   available for all three schemas.
7. If changing the endpoint schema makes a saved selection incompatible, the client does not
   discard or coerce it. The affected row keeps the value visible, shows an inline explanation,
   and disables Save until the user selects a compatible level or `Provider default`. Disabled
   options explain which schema is required.
8. For OpenAI Chat Completions, any explicit non-`none` choice shows non-blocking helper copy
   that tool-bearing requests may run with reasoning disabled. The client still sends the user's
   configured value; `warp-server` owns the final tools-plus-reasoning policy because tool
   presence is request-specific and not knowable in this settings form.
9. The existing endpoint-level API schema control remains the source of the compatibility state.
   Switching it updates all model rows immediately, including rows currently below the scroll
   fold, without changing their names, aliases, keys, or selected levels.
10. The modal remains usable at its current 560 px width and small-window height cap. Keep model
    name and alias on the first line of a row group and place the reasoning control and helper or
    error copy on a second line; do not compress three controls into narrow columns. The remove
    action removes the whole row group, and all dropdown popups render above the scroll clip.
11. Keyboard and pointer interaction can open and select every reasoning dropdown. Focus,
    scrolling-to-row, Add model, row removal, Save, Cancel, and the fixed action row continue to
    work with one or many model rows.
12. When custom inference is enabled and the endpoint is otherwise eligible for request
    serialization, every model is sent with its configured enum or the unspecified zero value.
    Existing endpoint filtering, BYO policy gates, schema serialization, API-key handling, and
    model selection by `config_key` remain unchanged.
13. The custom model picker continues to show one atomic custom model per `config_key`; this
    change does not create hosted-style reasoning variants or change the selected model ID.
14. QUALITY-1210 may land first as a server-side safety mitigation. This dial is the durable
    user-facing configuration and does not duplicate that mitigation in the client; a configured
    value is sent as-is and server-side precedence is defined by the `warp-server` spec.

## TECH

**Current context:** All references are pinned to Warp commit
`8b3327f0ec97165c7852052d2665d75e507011f3`.

- `crates/ai/src/api_keys.rs:49-122` defines the serde-defaulted `ApiKeys`,
  endpoint-level `CustomEndpointSchema`, and `CustomEndpointModel`, which currently contains only
  name, alias, and stable `config_key`.
- `crates/ai/src/api_keys.rs:278-284,471-532` passes model edits through a positional
  three-tuple in `CustomEndpointParams` and reconstructs stored models on add/save.
- `crates/ai/src/api_keys.rs:566-625` builds `CustomModelProviders`; it maps the endpoint schema
  but sends only `slug` and `config_key` per model.
- `crates/ai/src/api_keys.rs:713-756` reads and asynchronously rewrites the complete `AiApiKeys`
  JSON payload through platform secure storage.
- `app/src/settings_view/custom_inference_modal.rs:53-105` defines modal events/actions and
  `ModelRow`; model rows currently own two editors, a remove-button mouse state, and a
  `config_key`.
- `app/src/settings_view/custom_inference_modal.rs:108-390,456-500` builds/prefills rows and
  collects their values on Save. The endpoint schema dropdown already uses mirrored dropdown
  selection because its popup is rendered externally.
- `app/src/settings_view/custom_inference_modal.rs:761-1077` renders the 480 px form content,
  two-column model rows, clipped scrolling, fixed actions, and the schema popup at the outer
  stack.
- `app/src/settings_view/ai_page.rs:2433-2528` converts modal events to
  `ApiKeyManager::{add,save}_custom_endpoint`.
- `app/src/ai/agent/api.rs:306-325` attaches the manager's `CustomModelProviders` registry to
  each request when custom inference is enabled.
- `app/src/ai/llms.rs:1943-1982` builds one synthetic `LLMInfo` per stored custom model and
  deliberately sets its hosted-variant `reasoning_level` to `None`.
- `Cargo.toml:348` pins the generated `warp_multi_agent_api` Rust crate to a
  `warp-proto-apis` commit; `Cargo.lock:15810` records the same source revision.

### Proposed changes

1. **Add a typed persisted model level.**
   - Define `CustomEndpointReasoningLevel` next to `CustomEndpointSchema`, with the five shared
     wire values, display labels, a centralized ordered option list, schema compatibility, and
     an exhaustive conversion to the generated proto enum.
   - Add `reasoning_level: Option<CustomEndpointReasoningLevel>` to
     `CustomEndpointModel`. Missing storage data defaults to `None`; omit `None` when serializing
     the secure-storage JSON so it stays distinct from explicit `none`.
   - Make deserialization of an unrecognized future level conservative: treat it as unset and
     continue loading the rest of `ApiKeys` rather than returning `ApiKeys::default()` and hiding
     all credentials.
   - Replace `CustomEndpointParams.models`' positional tuple with a named
     `CustomEndpointModelParams` carrying `name`, `alias`, optional `config_key`, and optional
     reasoning level. Use one conversion helper for both add and save so generated/preserved keys
     and reasoning values cannot diverge.

2. **Extend the model-row UI without widening the modal.**
   - Give each `ModelRow` a reasoning dropdown initialized from its model or unset for a new row.
     Build its items from the central option registry; use rich disabled menu items and tooltips
     for schema-incompatible choices.
   - Subscribe each row dropdown to re-render on open/close, and forward every open popup to the
     same outer `Stack` used by the schema popup so menus escape the clipped scroll area.
   - Render each model as one row group: the existing name/alias line, then a labeled reasoning
     dropdown and contextual helper/error text. Extend the row position marker to cover the whole
     group so focus navigation scrolls the complete control into view.
   - Read mirrored dropdown selections in `save`, emit named model params, and include all row
     reasoning dropdowns in prefill, Add model, removal, close/reset, theme refresh where
     applicable, keyboard traversal, and compatibility validation.
   - Recompute compatibility from `selected_schema()` rather than relying on a potentially stale
     cached schema action. Keep the user's incompatible value selected, surface the reason, and
     include compatibility in `is_valid` so Save is blocked without data loss.

3. **Serialize the wire enum with an explicit unspecified sentinel.**
   - After the companion proto change is available, update the `warp_multi_agent_api` revision in
     `Cargo.toml` and `Cargo.lock`.
   - In `custom_model_providers_for_request`, map `Some(level)` onto the matching generated
     `ReasoningLevel` discriminant and map unset/unknown onto the generated unspecified variant
     with numeric value `0`. Do not map unset to the generated `NONE` variant; the zero scalar
     will be absent from proto3 encoded bytes.
   - Preserve the existing provider/model eligibility filters, endpoint schema mapping, API-key
     transport, and stable `config_key` mapping. No second request path is added:
     `Request::new` continues attaching the manager-produced registry.

4. **Keep picker semantics separate.**
   - Leave `custom_llm_info_from(...).reasoning_level` as `None`. That field groups multiple
     server-provided IDs into hosted reasoning variants; setting it on a custom model would make
     the main picker search for `SelectReasoningModel` while custom rows use
     `SelectModel(config_key)`, breaking selected-row matching.
   - The configured level is edited only in Custom endpoint Settings and travels only in the
     custom provider registry. No new picker variant, execution-profile field, or custom-model ID
     is introduced.

### Cross-repo contract

- **Consumes from `warpdotdev/warp-proto-apis`:** `CustomModel.reasoning_level = 3` and its
  generated scalar `ReasoningLevel` contract:
  `REASONING_LEVEL_UNSPECIFIED = 0`, `NONE = 1`, `LOW = 2`, `MEDIUM = 3`, `HIGH = 4`, and
  `XHIGH = 5`, as specified in
  [`warp-proto-apis` PR #352](https://github.com/warpdotdev/warp-proto-apis/pull/352).
  The Warp dependency pin moves to the proto commit that supplies this contract.
- **Provides to `warpdotdev/warp-server`:** for each client-configured custom model, the existing
  `config_key` and slug plus either `REASONING_LEVEL_UNSPECIFIED` (unset/legacy) or exactly one
  explicit nonzero enum value. The endpoint schema remains provider-level. The client never
  substitutes `NONE` for unset and never encodes request-time tool presence.
- **Consumes from `warpdotdev/warp-server` product policy:** the approved shared value set and
  static schema-compatibility matrix. The server remains authoritative for Chat Completions tool
  conflicts, OpenAI parameter shape, Anthropic budget mapping, reasoning summaries/encrypted
  content, and QUALITY-1210 precedence. The enterprise admin entry point neither reads nor writes
  this client's secure-storage state.

### Design alternatives

- **Stored representation:** A typed optional enum is selected over `Option<String>` because it
  centralizes display, schema compatibility, serde, and proto conversion and makes every new
  value an exhaustive compiler-guided edit. A raw string would tolerate future values but could
  send typos and scatter validation. Conservative unknown-value deserialization preserves
  downgrade safety without weakening the in-memory type.
- **Modal data transfer:** A named `CustomEndpointModelParams` is selected over extending the
  existing tuple to four elements. The named type prevents field-order mistakes across modal,
  settings-page, manager, and tests and leaves future per-model settings additive.
- **Control placement:** A two-line model row group is selected over widening the 560 px modal or
  compressing three columns. It preserves small-window behavior and readable model names while
  the existing scroll container absorbs the added height.
- **Incompatible schema changes:** Preserve-and-block is selected over silently resetting to
  unset or preserving and sending an invalid pair. It avoids data loss while guaranteeing a
  saved endpoint is internally valid.
- **Chat tools conflict:** A contextual warning plus server enforcement is selected over client
  coercion. The client cannot know the actual request's tool set, and coercion here would make the
  saved value disagree with what the user chose. If the approved server policy forbids a level
  statically for Chat Completions, that level moves into the same compatibility table and becomes
  blocking.
- **Picker integration:** Keeping custom reasoning out of `LLMInfo.reasoning_level` is selected
  over reusing the hosted variant UI because one custom model remains one selectable ID; creating
  variant IDs would destabilize execution profiles and duplicate the per-model setting.

### Open questions resolved

- Shared product questions are adopted from the foreman's alignment contract: per-model
  ownership; `none|low|medium|high|xhigh`; unset means protobuf `UNSPECIFIED` and preserves
  current behavior because the zero-valued scalar is omitted from encoded bytes.
- The UI label for unset is `Provider default`; local storage omits it and request construction
  maps it to proto `REASONING_LEVEL_UNSPECIFIED`, never explicit `NONE`.
- `xhigh` is treated as incompatible with Chat Completions under the working matrix and remains
  a single option-table edit if the lead server spec approves a different matrix.
- The client warns but does not decide the dynamic tools-plus-reasoning outcome. The server spec
  is the source of truth and can subsume QUALITY-1210 while remaining compatible with this wire.
- Unknown future local-storage values degrade to unset instead of invalidating all stored keys.
- No client telemetry is added: the level may describe a private model/provider configuration,
  and the ticket does not require product analytics.
- No feature flag is added. The control is inside the existing policy-gated custom-inference
  modal and is inert until the server/proto contract is deployed.

### Risks and mitigations

- A proto revision mismatch can make Warp fail to compile or accidentally collapse unset into
  explicit `NONE`; sequence the proto commit first and assert `UNSPECIFIED = 0` and `NONE = 1`
  separately, including encoded-byte behavior for the zero scalar.
- Positional row state can drift after add/remove operations; move values through named params,
  keep dropdown ownership inside `ModelRow`, and test multiple distinct rows through removal.
- Extra row height/popups can regress modal scrolling or fixed actions; retain the existing scene
  layout tests, add reasoning-popup coverage, and visually test small and full-height windows.
- An unknown stored value currently risks deserializing the whole `AiApiKeys` blob to defaults;
  use tolerant field-level deserialization and test preservation of unrelated credentials.
- Client compatibility can drift from server behavior; keep one explicit table, cover every
  schema/value pair in a table-driven test, and update it with any shared-spec decision before
  implementation begins.
- Setting the existing `LLMInfo.reasoning_level` would alter model picker semantics; add a
  regression assertion that configured custom models remain atomic `config_key` selections.

## Validation & verification criteria (all must pass before merge)

1. Add table-driven unit coverage for the complete client option registry. It must assert display
   labels, serde values, proto mappings, and the approved compatibility result for unset plus all
   five explicit levels across all three `CustomEndpointSchema` values.
2. Add `custom_endpoint_model_reasoning_serde_round_trip` in
   `crates/ai/src/api_keys_tests.rs`. It must prove explicit `none` and every other level survive
   the exact `ApiKeys` JSON shape written under `AiApiKeys`, while unset is omitted and reloads as
   `None`.
3. Add a legacy/forward-compatibility storage test: a payload with no reasoning fields must retain
   all endpoint/model/key/schema/config-key data, and a payload containing an unknown future model
   level must retain unrelated provider keys and endpoints while treating only that level as
   unset. No test may accept replacement of the full payload with `ApiKeys::default()`.
4. Add manager add/save tests proving named model params generate a key only for a new model,
   preserve existing keys on edit, preserve distinct reasoning values across at least two models,
   and do not transfer the second row's value after removing the first row.
5. Extend `custom_model_providers_for_request` tests to assert all three wire states separately:
   unset produces `REASONING_LEVEL_UNSPECIFIED` (and no field bytes after protobuf encoding),
   explicit `none` produces `REASONING_LEVEL_NONE`, and another explicit level produces its
   matching nonzero enum. Retain assertions for schema, slug, key, BYO/custom inference policy
   gating, invalid endpoint/model filtering, and multiple providers.
6. Add a request-construction regression test proving `Request::new` places those per-model fields
   under `settings.custom_model_providers` without changing model IDs, endpoint schema, API-key
   gating, or unrelated settings.
7. Add modal state tests for new, edit, and legacy rows: `Provider default` is the new-row default;
   prefill restores each explicit choice; Save emits named model params with the correct
   reasoning/config-key pair; and reopening after manager save restores the same values.
8. Add modal interaction tests with multiple model rows. Select distinct levels, add and remove a
   row, save, and assert every remaining row keeps its own value. Exercise pointer selection and
   keyboard focus/selection for the new dropdown.
9. Add exhaustive schema-change tests. A compatible selection remains saveable; changing to a
   schema that rejects a selected value leaves it visible, renders the inline reason, and disables
   Save; choosing a compatible value or `Provider default` clears the error and re-enables Save.
   The current matrix specifically covers `xhigh` when switching to/from Chat Completions.
10. Add Chat Completions helper-copy coverage proving explicit non-`none` renders the dynamic
    tools warning without changing the saved/wire value, while unset and explicit `none` do not
    render a misleading warning.
11. Extend scene/layout tests for one and many two-line model groups. At 560 px and a small window,
    inputs and controls remain within bounds, the form scrolls to the focused reasoning control,
    every open reasoning menu paints above the scroll clip, and the Save/Cancel action row remains
    fixed.
12. Add/retain a custom picker regression test proving a configured custom model with an explicit
    level is still represented by one `LLMInfo` with its original `config_key`, remains selectable
    under the `Custom models` section, and does not enter hosted reasoning-variant grouping.
13. Run focused deterministic tests from the repository root:
    `cargo test -p ai api_keys`,
    `cargo test -p warp custom_inference_modal`, and
    `cargo test -p warp custom_model_providers`. All new and adjacent tests must pass before the
    broader gate.
14. Run the repository's unconditional checks:
    `./script/format --check`,
    `cargo clippy -p ai --all-targets --tests -- -D warnings`,
    `cargo clippy -p warp --all-targets --tests -- -D warnings`, and
    `cargo check -p ai -p warp`. Each must pass with the pinned proto revision.
15. Because this L-sized change crosses shared AI storage, generated protocol types, request
    construction, and GUI state, run the full documented `./script/presubmit`; PR CI remains the
    final cross-platform/full-suite backstop.
16. Exercise the running desktop UI with computer use and attach visual proof to the Linear issue
    and PR. Capture a video showing: open Settings and Add custom model; configure two models with
    different values; switch schemas to show an incompatible state and recovery; save; reopen and
    confirm both values persisted; then use the custom model for an agent request. The recording
    must show the dropdown popup above the scroll area and the fixed actions in a small-window or
    many-model state. Media is not committed to the branch.
17. Validate the visual proof against behaviors 1-13 and record the focused tests, repository
    checks, full presubmit, proto revision, and visual artifact links in the implementation PR.
    Missing or stale proof is a merge blocker for this user-facing change.
