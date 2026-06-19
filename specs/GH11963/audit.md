# GH11963: Context & Capability Parity Audit — Hosted vs. Custom-Endpoint Models

> Evidence-backed audit answering the question: *"Is the context sent to Warp's
> hosted agent the same context sent to custom (BYO) endpoint models, and where
> do open models get a penalty?"* All references are `file:line` against the
> tree this audit was written from. Read this before the product/tech specs.

## TL;DR

The hypothesis is **half true, and the wrong half matters**:

- ✅ **The context *body* is already unified.** Files, git, project rules,
  codebase index, MCP, skills, attachments, environment — every `InputContext`
  element is assembled and serialized identically regardless of model type. The
  request goes to **one** server endpoint chosen by *request type*, never by
  model. There is no client-side context-trimming branch for custom models.
- ⚠️ **The penalty is real but lives in two other layers:**
  1. **Model metadata is stubbed** for custom models (zeroed context window,
     no reasoning level, `provider: Unknown`, empty host configs).
  2. **Capability flags** in the request are partly hardcoded and are being
     *forked* per-model-type by in-flight PRs — the opposite of consolidation.

So "consolidate so there's no penalty for open models" is best framed not as
"send the same context" (already done) but as **"give custom models real
capability metadata and derive request capabilities from that metadata uniformly,
instead of stubbing the model and hardcoding/branching the flags."**

## 1. The request is built once and routed by type, not model

`app/src/ai/agent/api/impl.rs:61-136` constructs a single `api::Request`. The
`InputContext` (`input`), `mcp_context`, `task_context`, rules toggle, Warp Drive
toggle, and web-context toggle are all set unconditionally:

- `impl.rs:65` — `input: Some(convert_input(params.input)?)`
- `impl.rs:74-76` — `rules_enabled`, `warp_drive_context_enabled`,
  `web_context_retrieval_enabled` — no model branch
- `impl.rs:135` — `mcp_context: params.mcp_context.map(Into::into)`

`app/src/ai/agent/api/convert_to.rs` → `convert_context()` walks every
`AIAgentContext` variant (shell commands, dir, selection, env, time, images,
codebases, project rules, files, git, repos, PRs, skills) with **no** conditional
on model, provider, or capability. Likewise `convert_input()`.

Dispatch: `app/src/server/server_api.rs` → `generate_multi_agent_output()`
selects the URL purely from `is_evals` and `is_passive`
(`.../ai/multi-agent` vs `.../ai/passive-suggestions`). **Model type never enters
URL selection.** Custom-endpoint requests traverse the identical client path and
hit the same Warp backend, which then forwards to the user's endpoint (confirmed
by the data-flow spec in PR #12003 / issue #11681: keys are stored locally, the
request is routed in-flight through Warp's backend).

**Conclusion for layer 1:** the context payload is already at parity. Nothing to
"consolidate" here beyond documenting it.

## 2. Custom-model metadata is stubbed — the real penalty source

`app/src/ai/llms.rs:1338` `custom_llm_info_from()` builds the `LLMInfo` for every
custom-endpoint model:

```rust
reasoning_level: None,                      // :1344
provider: LLMProvider::Unknown,             // :1353
host_configs: HashMap::new(),               // :1354
context_window: LLMContextWindow::default() // :1356  -> all zeros
```

`LLMContextWindow` (`app/src/ai/llms.rs:150-159`) defaults every field to `0`
(`is_configurable=false, min=0, max=0, default_max=0`).

This zeroed window flows straight into the request:

- `app/src/ai/execution_profiles/mod.rs:145-156`
  `context_window_limit_for_request()` returns **`None`** for any model whose
  `LLMContextWindow` is not configurable (`has_configurable_context_window` is
  false because the window is all zeros).
- `app/src/ai/agent/api.rs:343-355` reads that `None` into
  `context_window_limit`.
- `app/src/ai/agent/api/impl.rs:71`
  `base_model_context_window_limit: params.context_window_limit.unwrap_or(0)`
  → the server receives **`0`** ("unknown") for every custom model.

Downstream consequences (all driven by the stub, not by intent):

- Warp's context-window management / truncation / summarization heuristics have
  no real bound for custom models (this is exactly what issue **#11963**
  describes).
- Reasoning features key off `reasoning_level` / model spec — `None` disables
  them for custom models even when the underlying model supports thinking
  (issues **#11810**, **#11587**).
- The long-context pricing warning keys off `provider` + threshold; `Unknown` +
  `None` implicitly excludes custom models (made explicit by PR #12808).

## 3. Capability flags are hardcoded and being forked per model type

In `app/src/ai/agent/api/impl.rs:66-106`, the `Settings.supports_*` flags are a
mix of hardcoded `true`, feature-flag reads, and request params — but they are
**not** derived from the selected model's `LLMInfo`:

- `impl.rs:89` — `supports_reasoning_message: true` (hardcoded)
- `impl.rs:77` — `supports_parallel_tool_calls: true` (hardcoded)
- `impl.rs:80,82,84` — `supports_create_files`, `supports_long_running_commands`,
  `supports_todos_ui` (hardcoded)
- `impl.rs:104` — `custom_model_providers: params.custom_model_providers`
  (the only model-aware field, gated upstream)

Custom-model providers are gated separately in
`crates/ai/src/api_keys.rs:382-420` `custom_model_providers_for_request()` —
returns `None` when `!include_custom_models` (`:386`), where the flag derives
from workspace policy (`is_custom_inference_enabled`) + `FeatureFlag::CustomInferenceEndpoints`.

**The divergence trend:** rather than driving these flags from model capabilities,
in-flight PRs add ad-hoc per-model-type branches:

- **PR #11812** (open) — flips `supports_reasoning_message` *off* for custom
  endpoints (adds a `supports_reasoning_messages` capability gate). Closes #11810
  by *removing* a capability rather than modeling it.
- **PR #12808** (open) — adds `is_custom_endpoint` to `LongContextWarningState`
  to short-circuit the warning for custom models.

Each is locally reasonable but each adds another scattered `is_custom_endpoint`
branch. That's the pattern the parity work should replace with a single
capability source.

## 4. Where consolidation actually belongs

| Layer | State today | Parity action |
|-------|-------------|---------------|
| Context body (`InputContext`) | Already unified | Document & regression-test it |
| Model metadata (`LLMInfo` for custom) | Stubbed/zeroed (`llms.rs:1338`) | Carry real context window + reasoning (issue #11963) |
| Request capability flags (`impl.rs:66-106`) | Hardcoded + per-type branches | Derive from effective model's `LLMInfo` capabilities |
| Generation params (temperature, max tokens) | No wire field at all | Needs `warp-proto-apis` change first (deferred) |
| Server routing/translation | Out of this repo | N/A (Warp backend) |

**Wire check** (`warp-proto-apis` rev `97d1b36`, `request.proto`):
`ModelConfig.base_model_context_window_limit` (field 6) and
`Settings.supports_reasoning_message` (field 17) both exist → context-window and
reasoning parity are wire-supported today. `CustomModel` carries only `slug` +
`config_key`, and no `temperature`/`max_output_tokens` exists anywhere → generation
params require a proto change before any client work.

## 5. Related PRs and issues

**PRs — consolidating / metadata:**
- #12003 — spec: Custom Inference data flow disclosure (local key vs. in-flight routing) — read first
- #9620 — Spec OpenAI-compatible BYOK endpoints (argues for real custom metadata, not server-approved-only)
- #12783 — Replace `CustomInferenceEndpoints` flag with `BYO_ENDPOINT` billing policy (gating consolidation)
- #11731 — auto-discover models from `/v1/models` (would populate real metadata; issue #11514)
- #12227 (closed) — make long-context pricing warnings *server-driven* (preferred "derive, don't hardcode" pattern)
- #12775 — Custom Auto Models client request building (touches `api.rs` + `api/impl.rs` — conflict watch)

**PRs — diverging (counter to parity):**
- #11812 — disable reasoning messages for custom endpoints (closes #11810 by removal)
- #12808 — gate long-context warning to non-custom endpoints

**Issues:**
- #11963 — model configuration for custom endpoints (context window, temperature, params) — **primary anchor**
- #11810 / #11587 — custom/DeepSeek thinking-mode errors / reasoning_content round-trip
- #11947 — configure max input/output tokens on custom model
- #11514 — auto-discover models from `/v1/models`
- #11805 — Anthropic-compatible BYOK endpoints
- #9303 (closed) / #4339 — original BYO endpoint / local-LLM (Ollama) support
