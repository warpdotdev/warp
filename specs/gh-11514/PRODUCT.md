# gh-11514: Auto-discover models from `/v1/models` for custom inference endpoints

## Overview

When a user configures a custom inference endpoint (Settings → AI → Custom
inference), they currently have to type every model name by hand. Most
OpenAI-compatible providers expose a `GET /models` catalog, so Warp can fetch
that list and populate the model rows for the user.

This adds a **"Fetch from endpoint"** button to the custom endpoint modal that
calls the endpoint's `/models` route with the entered credentials, parses the
OpenAI-compatible response, and merges the returned model IDs into the modal's
model rows.

- Issue: [#11514](https://github.com/warpdotdev/warp/issues/11514)
- Related / duplicate request: [#11586](https://github.com/warpdotdev/warp/issues/11586)
- Builds on: [#10781](https://github.com/warpdotdev/warp/pull/10781) (custom inference endpoints / BYOK)

This is a user-triggered, additive convenience. It changes nothing about how
endpoints are persisted or sent on the wire — it only fills in the same model
rows the user would otherwise type.

## Problem

Manual entry is slow and error-prone, especially for providers that expose
many model IDs (OpenRouter) or rotate them often. A mistyped model name fails
silently at request time rather than at configuration time.

## Desired behavior

### The button
- A **"Fetch from endpoint"** button sits next to **"+ Add model"** in the
  custom endpoint modal.
- It is **disabled** unless all of the following hold:
  - the Endpoint URL field is non-empty,
  - the API key field is non-empty,
  - the URL passes the modal's existing validation (HTTPS, has host, not a
    local/private host — the same rules that gate the Save button).
- While a fetch is in flight the button shows **"Fetching…"** and is disabled
  so a second fetch can't be launched on top of the first.

### On click
1. Warp issues `GET <endpoint-url>/models` with `Authorization: Bearer <api key>`
   and `Accept: application/json`.
2. On success, the returned model IDs are merged into the model rows:
   - IDs not already present (case-insensitive match against existing row
     names) are appended as new rows.
   - If the modal currently has a single blank row (the default on open), it is
     replaced rather than left dangling above the fetched rows.
   - Aliases the user already typed on existing rows are preserved.
3. An inline status line below the buttons reports the outcome:
   - `Added N models from endpoint.`
   - `Added N models (M already configured).`
   - `No new models found (M already configured).`
   - `Fetch failed: <reason>` (rendered in the theme's error color).

### Result states the user can see
| Situation | Status line |
|---|---|
| Fetch returned new IDs | "Added N models from endpoint." |
| Some new, some already present | "Added N models (M already configured)." |
| All returned IDs already present | "No new models found (M already configured)." |
| Network error / unreachable | "Fetch failed: network error fetching …" |
| 401 / 403 (bad key) | "Fetch failed: endpoint returned HTTP 401 …" |
| 404 (no `/models` route) | "Fetch failed: endpoint returned HTTP 404 …" |
| Non-JSON / HTML body | "Fetch failed: response was not OpenAI-compatible JSON" |
| Valid JSON, empty `data` | "Fetch failed: endpoint returned no models" |

## Invariants and edge cases

- **Discovery never overwrites user input.** It only appends model rows that
  aren't already present and never edits or removes existing rows or aliases.
- **Trailing slash tolerance.** `https://host/v1` and `https://host/v1/` both
  resolve to `https://host/v1/models`.
- **Empty model IDs are dropped.** Entries in `data[]` with a blank `id` are
  ignored; if that leaves nothing, the fetch reports "no models" rather than
  adding blank rows.
- **In-flight fetch is cancellable.** Closing the modal, or re-prefilling it
  for a different endpoint, aborts any pending fetch and clears the status.
- **Same security envelope as Save.** Discovery reuses the modal's URL
  validation, so it can't be pointed at `http://`, `localhost`, or
  private/loopback IPs even though Save would reject those too. (This means
  local providers like Ollama on `http://localhost:11434/v1` are not reachable
  via this button — that's an intentional consequence of the existing endpoint
  validation, not a discovery-specific rule, and is called out as a known
  limitation below.)
- **Response size is bounded.** Bodies larger than 1 MiB are rejected before
  JSON parsing.

## Out of scope

- Capability introspection (context window, vision/tool-call support). The
  `/models` response shape for these fields is non-standard across providers;
  deferred until there's enough demand.
- Headless / `oz-cli` configuration of endpoints
  ([#8937](https://github.com/warpdotdev/warp/issues/8937)).
- Reaching local (`http://localhost`) providers — blocked by the existing
  custom-endpoint URL validation, not by this feature. Relaxing that is a
  separate decision.
- Team-level sharing of endpoints/aliases
  ([#11726](https://github.com/warpdotdev/warp/issues/11726)).

## Success criteria

- A user can configure an OpenRouter (or other OpenAI-compatible) endpoint by
  entering URL + key and clicking "Fetch from endpoint" instead of typing
  model names.
- Every failure mode above produces a clear inline message and never a crash,
  hang, or partially-populated/blank row.
- Discovery is purely additive: existing rows and aliases are untouched.

## Validation

- Unit tests for `discover_models` cover the happy path plus every failure mode
  (empty URL, empty key, 401, 404, network error, non-JSON, missing `data`,
  empty model list, blank-ID filtering, trailing-slash) against a mock HTTP
  server.
- Unit tests for the merge helper cover case-insensitive dedup, dedup within a
  single response, and ignoring blank existing rows.
- Manual: configure a real OpenRouter endpoint, click Fetch, confirm rows
  populate; repeat to confirm idempotence ("No new models found").
