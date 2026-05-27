# gh-11514 — `/v1/models` auto-discovery for custom inference endpoints

## Context

[Issue #11514](https://github.com/warpdotdev/warp/issues/11514) asks for a way
to populate a custom inference endpoint's model list from the provider's
OpenAI-compatible `GET /models` catalog instead of typing each model ID by
hand. [#11586](https://github.com/warpdotdev/warp/issues/11586) is an
independently-filed request for the same feature.

This builds directly on [#10781](https://github.com/warpdotdev/warp/pull/10781),
which shipped custom inference endpoints (BYOK + arbitrary OpenAI-compatible
URLs) behind the `CustomInferenceEndpoints` feature flag. That PR established
the data model and the configuration modal this feature extends:

- `crates/ai/src/api_keys.rs` — `CustomEndpoint { name, url, api_key, models }`
  and `CustomEndpointModel { name, alias, config_key }`. **Unchanged** by this
  work; discovery only fills in the same `models` rows the user would type.
- `app/src/settings_view/custom_inference_modal.rs` — the
  `CustomEndpointModal` view, its `ModelRow`s, `AddModel`/`RemoveModel`
  actions, URL validation, and `Save`.

The persisted shape and the request wire format are untouched. This is a
client-only, additive convenience layer over the existing modal.

## Design

Two pieces:

1. A small, self-contained network/parse helper (`discover_models`) plus a pure
   merge helper (`new_model_ids`), both unit-testable without UI.
2. Modal wiring: a "Fetch from endpoint" button that runs the helper as a
   background task and merges the result into the existing model rows.

### `app/src/ai/discover_models.rs` (new)

```rust
pub async fn discover_models(
    client: &http_client::Client,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<String>>
```

- Trims `base_url`, strips a trailing `/`, and requests `<base>/models`.
- Rejects empty URL / empty key up front with a clear error.
- `GET` with `bearer_auth(api_key)` + `Accept: application/json` via the
  existing `http_client::Client` wrapper (same client used elsewhere in the
  app, e.g. `load_ai_conversation.rs`).
- Non-2xx → error carrying the status and a 200-char body snippet.
- Caps the buffered body at `MAX_RESPONSE_BYTES` (1 MiB) before parsing.
- Parses the OpenAI-compatible shape with serde:
  ```rust
  struct ModelsResponse { data: Vec<ModelEntry> }
  struct ModelEntry { id: String }
  ```
  Unknown fields are ignored, so provider-specific extras don't break parsing.
- Filters blank IDs; empty result → error ("endpoint returned no models").

```rust
pub fn new_model_ids<'a>(discovered: &'a [String], existing: &[String]) -> Vec<&'a str>
```

- Returns discovered IDs not already present, preserving discovery order.
- Case-insensitive match against existing row names; dedups within the
  discovered list; ignores blank existing rows (so a fresh modal with one empty
  row counts as "no existing models").

### Modal wiring — `custom_inference_modal.rs`

- New `CustomEndpointModalAction::FetchModels`.
- New private `FetchStatus` enum: `Idle | InProgress | Success { added, skipped } | Error(String)`.
- New fields on `CustomEndpointModal`: `fetch_models_button_mouse_state`,
  `fetch_status: FetchStatus`, `fetch_handle: Option<SpawnedFutureHandle>`.
- `fetch_models(&mut self, ctx)`:
  - Reads URL + key from the editors; guards on empty and on `validate_url`
    (the modal's existing HTTPS/host/non-private check), surfacing an inline
    error instead of firing a request.
  - Aborts any prior in-flight `fetch_handle`, sets `InProgress`, and spawns the
    request via `ctx.spawn(async { discover_models(&Client::new(), &url, &key).await }, on_done)`.
- `apply_fetch_result(result, ctx)` (the spawn continuation):
  - On `Ok`, collects existing row names, computes `new_model_ids`, and appends
    a `ModelRow` per new ID (subscribing each new editor to the existing model
    editor event handler). If the only row was the default blank row, it's
    cleared first so no stray empty input remains. Sets
    `Success { added, skipped }`.
  - On `Err`, sets `Error(format!("{err:#}"))` (anyhow chain).
- Render: "Fetch from endpoint" button beside "+ Add model", disabled unless URL
  + key are present, the URL is valid, and no fetch is in flight; label flips to
  "Fetching…" during the request. A status line renders below, in the error
  color for `Error`.
- Lifecycle: `prefill` and `on_close` abort any in-flight handle and reset
  `fetch_status` to `Idle`, so a stale fetch can't land on a different endpoint
  or a closed modal.

## Cancellation & threading

`ctx.spawn` runs the future on a background executor and invokes the
continuation back on the UI thread, where it mutates `model_rows` and calls
`ctx.notify()`. The returned `SpawnedFutureHandle` is stored so the modal can
`abort()` it on close / re-prefill / a superseding fetch. Dropping/aborting is
safe: the continuation simply never runs.

## Security / resource considerations

- **No new reachable surface.** Discovery reuses `validate_url`, so it inherits
  the endpoint allow-list (HTTPS only, must have a host, no localhost / private
  / loopback). It can't be pointed anywhere `Save` couldn't already point.
- **Bounded body.** 1 MiB cap before JSON parsing guards against a hostile or
  misconfigured endpoint streaming an oversized response.
- **Bearer key handling.** The key is read from the existing password editor and
  sent only to the user-entered endpoint over the bearer header; it is not
  logged. Error snippets are capped at 200 chars and come from the response
  body, not the request.

## Testing

- `app/src/ai/discover_models_tests.rs` (14 tests) using `mockito::Server`
  against `http_client::Client::new_for_test()`:
  happy path (asserts the `Authorization: Bearer` header), trailing-slash,
  empty-URL, empty-key, 401, 404, non-JSON, missing `data`, empty `data`,
  blank-ID filtering, network-unreachable; plus three `new_model_ids` cases
  (case-insensitive exclude, intra-response dedup, blank-existing-row).
- These cover every branch in the helper without standing up the UI. The modal
  wiring is thin glue over the tested helper; behavior there is validated
  manually (configure a real OpenRouter endpoint, Fetch, confirm rows populate
  and re-fetch is idempotent).

## Rollout

Lives behind the same `CustomInferenceEndpoints` feature flag as the modal it
extends — no separate flag. No migration: persisted `CustomEndpoint` data and
the request wire format are unchanged.

## Out of scope (see PRODUCT.md)

Capability introspection (context window / vision), headless config (#8937),
reaching `http://localhost` providers (blocked by endpoint validation), and
team-level endpoint sharing (#11726).
