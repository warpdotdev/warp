# Fetch models for a custom inference endpoint

Inspected at [`3959ea72141fc0ecd007665029a8058e0e6db0f8`](https://github.com/warpdotdev/warp/commit/3959ea72141fc0ecd007665029a8058e0e6db0f8).

## Context

[#11514](https://github.com/warpdotdev/warp/issues/11514) (`ready-to-spec`) asks for a **Fetch models** control in the custom-endpoint modal that calls OpenAI-compatible `GET /models` and merges `data[].id` into the existing model rows. User-facing behavior is in [PRODUCT.md](PRODUCT.md).

This is not Codex-style live catalog refresh. APP-5380 listed discovery as a non-goal for the shared definition migration; this follow-up is GUI-only, user-triggered, and writes the same rows the user would type.

Current flow:

1. [`CustomEndpointModal`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/settings_view/custom_inference_modal.rs#L89-L103) edits name, URL, API key, schema, and `ModelRow`s. Actions today are Cancel / Save / AddModel / RemoveModel / RemoveEndpoint / SetSchema ([`CustomEndpointModalAction`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/settings_view/custom_inference_modal.rs#L73-L80)).
2. Save emits `AddEndpoint` / `SaveEndpoint` with `models: Vec<(name, alias, config_key)>`. Empty names are dropped ([`save`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/settings_view/custom_inference_modal.rs#L457-L499)).
3. [`warp_agent_page.rs` `handle_custom_endpoint_modal_event`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/settings_view/warp_agent_page.rs#L1562-L1636) calls [`custom_endpoints::add` / `save`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/ai/custom_endpoints.rs#L126-L148).
4. Definitions live in `agents.custom_endpoints`; keys in `AiCustomEndpointKeys`. [`CustomEndpointModel`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/crates/ai/src/api_keys.rs#L379-L386) is `{ name, alias, config_key }`. A definition is invalid with zero models ([`is_valid`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/crates/ai/src/api_keys.rs#L204-L216)).
5. [`build_custom_llm_infos`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/ai/llms.rs#L2204-L2216) synthesizes picker entries from those stored rows. The desktop app never HTTP-calls the user URL.

URL policy already used by Save: [`validate_custom_endpoint_url`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/crates/ai/src/api_keys.rs#L336-L347) (HTTPS, public host). Fetch must reuse it.

There is a stale draft, [PR #11731](https://github.com/warpdotdev/warp/pull/11731) (May 2026, `david/local-models-discovery`). It implemented this shape (`discover_models` + modal button) against pre–APP-5380 storage. Do not land that branch as-is; rebase the idea onto the current modal and settings coordinator.

Async precedent: [`CreateApiKeyModal::fetch_agents`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/settings_view/platform/create_api_key_modal.rs#L282-L309) uses `ViewContext::spawn`. `spawn` returns [`SpawnedFutureHandle`](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/crates/warpui_core/src/core/view/context.rs#L566) for abort on close/prefill.

## Proposed changes

### 1. Pure discovery helper — new `app/src/ai/discover_models.rs`

Keep HTTP and merge out of the view.

```rust
pub struct DiscoveredModel { pub id: String, pub alias: Option<String> }

pub async fn discover_models(
    client: &http_client::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<DiscoveredModel>, DiscoverModelsError>
```

- Trim `base_url`, strip one trailing `/`, request `{base}/models`.
- Reject empty URL or key before I/O.
- `GET` with bearer auth + `Accept: application/json` via `http_client::Client` (same wrapper as other app HTTP). Set a 15s timeout on this request (PRODUCT invariant 7).
- Non-2xx → typed error (`Unauthorized` for 401/403, `NotFound` for 404, otherwise `UnexpectedResponse`). Do not include the response body in `Display`.
- Buffer at most 1 MiB; oversize → `UnexpectedResponse`.
- Deserialize `{ "data": [ { "id": String } ] }` with ignored unknown fields. Missing `data` or invalid JSON → `UnexpectedResponse`.
- Drop blank/whitespace IDs; if none remain → `NoModels`.
- Dedup within the response case-insensitively, preserve first-seen order.

```rust
pub fn new_model_ids<'a>(discovered: &'a [String], existing: &[String]) -> Vec<&'a str>
```

Case-insensitive match against trimmed existing names; ignore blank existing names so a fresh modal’s empty row does not block appends.

Do not put this in `crates/ai`: it is GUI modal glue, not the shared definition type. TUI does not call it.

### 2. Modal wiring — `custom_inference_modal.rs`

- Add `CustomEndpointModalAction::FetchModels`.
- Add `FetchStatus { Idle, InProgress, Success { added: usize }, NoNew, Failed(&'static str /* or small owned reason matching PRODUCT (10) */) }`.
- Fields: `fetch_models_button_mouse_state`, `fetch_status`, `fetch_handle: Option<SpawnedFutureHandle>`.
- Enable the button with the PRODUCT (3) predicate. Label **Fetch models**; while `InProgress`, **Fetching…** and disabled.
- `fetch_models`: abort prior handle; read URL + key from editors; if `validate_url` fails, set `Failed` without I/O; else spawn `discover_models`. Copy URL and key into the future; do not log them.
- Continuation: if the modal was closed/prefilled (generation counter or aborted handle), drop the result. On `Ok`, compute `new_model_ids` from current editor names; if the sole row is blank, `remove_model(0)` first; append `create_model_row(Some(id), None, None, …)` and subscribe editors the same way `add_model` does. New rows get a new `config_key` from `create_model_row` (today `None` mints on save).
- `prefill` and `on_close`: abort `fetch_handle`, set `Idle`. `on_open` does not fetch.

Place the button in a row with **+ Add model** ([current Add model block](https://github.com/warpdotdev/warp/blob/3959ea72141fc0ecd007665029a8058e0e6db0f8/app/src/settings_view/custom_inference_modal.rs#L959-L981)). Status line immediately below that row.

Save path unchanged: still emits whatever rows are in the form. Fetch never calls `custom_endpoints::add/save`.

### 3. Persistence / picker / requests

No schema or wire changes. Once the user saves, APP-5380 already:

- writes `CustomEndpointModel { name, alias, config_key }` into `agents.custom_endpoints`,
- rebuilds `custom_llms` on `ApiKeyManagerEvent::KeysUpdated`.

Preserve `config_key` for names that already existed (merge skips those rows). New fetched names mint new keys on first save, matching typed rows.

### 4. Schema

Do not special-case Anthropic in the helper. A 404 becomes `Fetch failed: not found` (PRODUCT 4, 10). Users can still type rows.

### 5. Feature gating

No new flag. The modal is already gated by BYO endpoint entitlement (`CustomInferenceVisibility` / `can_use_custom_inference_controls`). Fetch is only reachable when the modal is.

### 6. Telemetry

Optional later. If added, emit only `{ outcome: success|unauthorized|not_found|network|unexpected|no_models|no_new, added_count }` — never URL, host, or key.

## Testing and validation

Map to PRODUCT invariants. Prefer unit tests for the helper; modal tests where they already exist (`custom_inference_modal_tests.rs`).

| PRODUCT | Proof |
|---|---|
| 3, 6, 7 | Helper rejects empty URL/key; mock server: 200 + timeout. Button disabled when URL invalid or fetch in flight (modal unit or render snapshot if the file already tests button disable). |
| 6 URL join | `https://host/v1` and `https://host/v1/` both hit `/v1/models`. |
| 8–9 | Parse extra fields; drop blank ids; case-insensitive skip; intra-response dedup; blank existing row ignored. |
| 10–11, 14 | Each HTTP/parse failure maps to the specified status; `Display` of errors contains no body/key. |
| 12–13 | Abort handle on `prefill` / `on_close`; stale continuation does not append rows. |
| 15–16 | Save still requires a named model; fetched names round-trip through `CustomEndpointParams` like typed names. Existing `custom_endpoints` / `llms` tests remain green. |
| 17–18 | No TUI or picker-open fetch code. |

Manual (PRODUCT 6, 9, 10): against a public HTTPS OpenAI-compatible catalog (OpenRouter or the AI Router example from #11514 comments). Fetch once, confirm rows; fetch again, confirm `No new models` and aliases unchanged; Save; picker shows alias-or-name.

`cargo nextest run -p warp --lib discover_models custom_inference_modal`; `./script/format`; clippy on touched files.

Do not require localhost Ollama for acceptance; URL policy still forbids it.

## Parallelization

Not useful. Helper + modal + tests share one form and one merge function. A second agent would collide on `custom_inference_modal.rs`. Implement on one branch, e.g. `gh-11514-fetch-models`, in this checkout.

## Risks and mitigations

- **Stale PR #11731.** Pre–APP-5380 (`ai_page.rs`, `ApiKeys.custom_endpoints` blob). Reimplement on current files; cherry-pick only `discover_models` tests if they still compile.
- **Huge catalogs.** 1 MiB cap bounds memory; OpenRouter can still add hundreds of rows. Acceptable for v1; user can delete rows. Do not write the catalog until Save.
- **Stale spawn.** Abort on close/prefill so a slow 200 cannot append into a different endpoint.
- **Secret leakage.** Typed errors only; no `{:#}` of anyhow chains that include URL+key.
- **Anthropic 404.** Expected; do not hide the button.

## Follow-ups

- Capability fields (context / vision / tools) — #11514 out of scope.
- Relaxing HTTPS/localhost for Ollama — separate URL-policy change, not this button.
- Codex-style background catalog — explicitly rejected here.
