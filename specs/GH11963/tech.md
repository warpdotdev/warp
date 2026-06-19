# GH11963: Tech Spec — Model configuration & capability parity for custom endpoints

## Context
Product behavior is in `specs/GH11963/product.md`; the evidence audit is in
`specs/GH11963/audit.md`. This spec adds optional per-model metadata to the
custom-endpoint config and threads it through the existing model-info and request
paths so custom and hosted models share one capability code path. **No change to
the context payload** (already unified — audit §1) and **no change to network
routing**.

Key existing paths (verified `file:line`):

- `crates/ai/src/api_keys.rs:42-60` — `CustomEndpoint { name, url, api_key, models }`
  and `CustomEndpointModel { name, alias, config_key }`. Both derive
  `Serialize/Deserialize` with `#[serde(default)]`, so adding optional fields is
  backward-compatible and round-trips through the persisted blob.
- `crates/ai/src/api_keys.rs:382-420` — `custom_model_providers_for_request()`
  builds the `CustomModelProviders` wire payload from these structs; gated by
  `include_custom_models` (`:386`).
- `app/src/ai/llms.rs:1338-1358` — `custom_llm_info_from()` constructs the
  `LLMInfo` for each custom model. This is where the stub lives:
  `reasoning_level: None` (`:1344`), `provider: LLMProvider::Unknown` (`:1353`),
  `host_configs: HashMap::new()` (`:1354`),
  `context_window: LLMContextWindow::default()` (`:1356`).
- `app/src/ai/llms.rs:1324-1336` — `build_custom_llm_infos()` iterates endpoints×
  models and calls `custom_llm_info_from`.
- `app/src/ai/llms.rs:150-159` — `LLMContextWindow { is_configurable, min, max,
  default_max }` (all `0` by default).
- `app/src/ai/execution_profiles/mod.rs:129-156` — `configurable_context_window()`
  and `context_window_limit_for_request()`; the latter returns `None` unless
  `has_configurable_context_window(llm, ...)` is true, then
  `self.context_window_limit.map(|l| l.clamp(min, max))`.
- `app/src/ai/agent/api.rs:343-355` — reads `context_window_limit_for_request(app)`
  into `RequestParams.context_window_limit`.
- `app/src/ai/agent/api/impl.rs:66-106` — the `Settings` block: `model_config`
  (`:67-73`, incl. `base_model_context_window_limit: ...unwrap_or(0)` at `:71`),
  hardcoded `supports_reasoning_message: true` (`:89`), and other `supports_*`
  flags; `custom_model_providers` (`:104`).
- `app/src/settings_view/ai_page.rs:8062` — `render_custom_endpoints_list` and the
  custom-endpoint modal (add/edit/save handlers around `:2119-2200`,
  `crates/ai/src/api_keys.rs:257-330`).

**Wire/proto verification** (`warp_multi_agent_api` = `warpdotdev/warp-proto-apis`
rev `97d1b367b955c562812e0a1315a6ec7ee6a5389e`, `apis/multi_agent/v1/request.proto`):
- `Settings.ModelConfig.base_model_context_window_limit` (uint32, field 6) exists —
  *"Zero or unset means use the model's default max."* Context-window parity is
  wire-supported; we populate this field instead of sending `0`.
- `Settings.supports_reasoning_message` (bool, field 17) exists at the request
  level — reasoning parity is wire-supported by driving this flag from the model.
- `Settings.ApiKeys.CustomModelProviders.CustomModel` carries only `slug` and
  `config_key`. There is **no** `temperature`, `top_p`, `max_output_tokens`, or
  `max_tokens` anywhere in the request. Per-model generation parameters therefore
  require a `warp-proto-apis` change first — out of scope here (see Follow-ups).

## Proposed changes

### 1. Add optional metadata to the custom-model config (`crates/ai/src/api_keys.rs`)
Extend `CustomEndpointModel` (`:51-60`) with optional, defaulted fields so existing
persisted blobs deserialize unchanged:

```rust
pub struct CustomEndpointModel {
    pub name: String,
    pub alias: Option<String>,
    pub config_key: String,
    #[serde(default)] pub context_window: Option<u32>,     // max input tokens
    #[serde(default)] pub supports_reasoning: bool,        // default false
}
```

Only fields that are expressible on the current wire are persisted. Temperature /
max output tokens are intentionally omitted until the proto carries them (see
Follow-ups) — persisting a field the client cannot send would be dead state.

Because the struct already round-trips through the `ApiKeys` blob, this is the
`settings.toml`-representable surface product goal 4 asks for. Keep validation
permissive (product behavior §7): drop invalid values to `None`/default at parse
or save time using the existing settings-validation path.

### 2. Populate real `LLMInfo` from the metadata (`app/src/ai/llms.rs:1338`)
In `custom_llm_info_from()`, map the new fields instead of stubbing:
- `context_window`: when `model.context_window` is `Some(n)`, build an
  `LLMContextWindow { is_configurable: false, min: n, max: n, default_max: n }`
  (or `is_configurable: true` with a sensible min if we want the configurable-window
  UI). When `None`, keep `LLMContextWindow::default()` (today's behavior →
  backward compatible).
- `reasoning_level`: derive a non-`None` level when `model.supports_reasoning` is
  true (reuse the same representation hosted reasoning models use; exact string/enum
  TBD against `LLMSpec`).
- Leave `provider: LLMProvider::Unknown` unless we add a declared provider later;
  parity does not require a real provider, only real capabilities.

This makes `has_configurable_context_window` / `context_window_limit_for_request`
(`execution_profiles/mod.rs:145-156`) return the configured window for custom
models, so `impl.rs:71` sends the real `base_model_context_window_limit` instead of
`0` — satisfying success criterion 1 with no change to those call sites.

### 3. Derive request capabilities from the effective model (`app/src/ai/agent/api/impl.rs`)
Replace the hardcoded `supports_reasoning_message: true` (`:89`) with a value
derived from the effective base model's `LLMInfo` (e.g. a
`model_supports_reasoning(&effective_model)` helper that reads `reasoning_level`/
spec). Thread the effective `LLMInfo` (or a small extracted `ModelCapabilities`
struct) into `RequestParams` so `generate_multi_agent_output` reads capabilities
from one source. This is the consolidation: one capability source for hosted and
custom models, replacing the per-type branches PR #11812 / #12808 introduce.

Generation params (temperature / max output tokens) are **not** built here: the
wire has no field for them (see Wire/proto verification). They are deferred to a
follow-up that first adds fields to `warp-proto-apis`.

### 4. Settings UI (`app/src/settings_view/ai_page.rs`)
Add the optional fields to the custom-endpoint add/edit modal next to the model
name/alias inputs: a numeric "Context window" and a "Supports reasoning" toggle.
Wire them through the existing `add_custom_endpoint` / `save_custom_endpoint`
handlers (`crates/ai/src/api_keys.rs:257-330`). Add settings-search terms per
product §6.

## Testing and validation
- Unit: `custom_llm_info_from` maps a configured window/reasoning into `LLMInfo`;
  `None`/false reproduces today's stub (backward-compat). Add to `llms_tests.rs`.
- Unit: `context_window_limit_for_request` returns the configured window for a
  custom model and `None` when unset (extend
  `execution_profiles/editor/mod_tests.rs`).
- Unit: capability helper drives `supports_reasoning_message` on/off per model;
  hosted models unchanged (extend `app/src/ai/agent/api/impl_tests.rs` — same file
  PR #11812 touches).
- Serde round-trip: `CustomEndpointModel` with and without new fields
  (`api_keys_tests.rs`).
- Manual: configure a custom model window, confirm request carries it (not `0`);
  toggle reasoning and confirm per-model effect.

## Risks & coordination
- **Merge conflicts**: PR #12775 (Custom Auto Models) edits `api.rs` + `api/impl.rs`;
  PR #11812 edits `impl.rs` + `impl_tests.rs`; PR #12808 edits the usage/footer
  paths. Coordinate ordering — ideally land the capability-source refactor (step 3)
  so #11812/#12808 simplify into it rather than conflict.
- **Reasoning representation**: needs to match what hosted reasoning models and the
  server expect; underspecified models (DeepSeek round-trip, #11587) may need
  server-side support beyond this client change — keep the toggle conservative.
- **Server contract**: sending a real `base_model_context_window_limit` for custom
  models assumes the backend honors it for forwarded endpoints; verify against the
  data-flow described in PR #12003 before shipping.

## Follow-ups
- **Generation parameters (issue #11963, #11947)**: temperature / max output tokens
  need new fields on `warp-proto-apis`
  (`Settings.ApiKeys.CustomModelProviders.CustomModel` and/or `ModelConfig`) before
  any client work. Sequence: proto PR → bump the `warp_multi_agent_api` rev in
  `Cargo.toml:338` → persist + send the values → modal UI.
- **`/v1/models` auto-discovery (issue #11514 / PR #11731)** can populate
  `context_window` / reasoning automatically once this metadata model exists.
- **Per-model reasoning protocol (issue #11587)**: DeepSeek-style
  `reasoning_content` round-trips may need server support beyond the client flag.
