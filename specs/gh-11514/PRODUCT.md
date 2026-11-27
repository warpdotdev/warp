# Fetch models for a custom inference endpoint

## Summary

In the GUI custom-inference endpoint editor, the user can click **Fetch models** to load OpenAI-compatible model IDs from `GET {base URL}/models` into the same model rows they would otherwise type. Discovery is one-shot and user-triggered. It does not run on open, on save, or in the background, and it does not change how endpoints are stored or sent on requests.

Issue: [#11514](https://github.com/warpdotdev/warp/issues/11514). Same request: [#11586](https://github.com/warpdotdev/warp/issues/11586).

## Problem

Configuring a custom endpoint still requires typing every model slug. That is slow on large catalogs (OpenRouter) and easy to mistype. Warp already has a modal for URL, API key, schema, and model rows; it does not query the endpoint.

APP-5380 made those definitions file-backed and shared, and listed model discovery as a non-goal. This spec is the follow-up that issue #11514 asked for: an explicit fetch in the GUI editor only.

## Goals / Non-goals

**Goals**

- Let the user populate model rows from `GET /models` without typing each ID.
- Keep fetch additive: existing names and aliases stay unless the user edits them.
- Keep discovery off the request path, off the TUI, and off any automatic refresh.

**Non-goals**

- Codex-style auto-refresh on picker open, session start, or a timer.
- Inferring context window, vision, tools, pricing, or reasoning from the catalog.
- Changing URL policy (HTTPS, no localhost/private hosts).
- TUI or `settings.toml` discovery.
- Connectivity tests, extra HTTP headers, or a new persisted catalog format.
- Cloud-agent use of custom endpoints.

## Figma

Figma: none provided. Layout follows the existing custom-endpoint modal and the control described in #11514.

## Behavior

1. The custom-endpoint modal still lets the user add, edit, and remove model rows by hand, including aliases. Fetch is optional.

2. A **Fetch models** control sits next to **+ Add model**. It is not a substitute for Add model.

3. Fetch models is enabled only when all of the following hold:
   - the Endpoint URL field is non-empty,
   - the API key field is non-empty,
   - the URL passes the same validation as Save (HTTPS, has a host, not localhost or a private/loopback address),
   - no fetch is already in flight.

4. Fetch models stays available for every API schema in the modal (OpenAI Chat Completions, OpenAI Responses, Anthropic Messages). The request is always OpenAI-compatible `GET {base URL}/models`. If a schema’s upstream has no such route, the user sees a failure status and can still type rows.

5. Clicking Fetch models does not save the endpoint. Rows appear in the open modal only. Persist still happens when the user clicks **Add endpoint** / **Save**.

6. On click, Warp:
   - shows an in-flight state on the control (label **Fetching…**, control disabled),
   - issues `GET {normalized base URL}/models` with `Authorization: Bearer {API key}` and `Accept: application/json`,
   - treats `{base}` and `{base}/` as the same origin (`…/v1` and `…/v1/` both become `…/v1/models`).

7. The request uses a bounded wait (15 seconds). If it does not finish in time, the user sees a fetch-failed status and existing rows are unchanged.

8. On success, Warp parses `{ "data": [ { "id": "..." }, ... ] }`. Unknown fields on the object or on each entry are ignored. Blank or whitespace-only `id` values are dropped. If an entry has a non-empty `display_name` or `name` that differs from `id` (case-insensitive), that value is used as the row alias. `display_name` wins when both are present. Official OpenAI catalogs typically have only `id`; OpenRouter sends `name`; some Codex-shaped catalogs send `display_name`.

9. Surviving IDs merge into the modal’s model rows:
   - An ID that already matches an existing row name (case-insensitive, trimmed) is skipped. That row’s alias and identity stay as they are.
   - New IDs append in catalog order as new rows with the ID as the model name, the catalog alias when one was present, otherwise an empty alias, and a new stable model identity (same kind of identity a typed row gets).
   - Duplicate IDs inside one response are added at most once (first occurrence wins).
   - If the only row is the default empty row (blank name and blank alias), that empty row is removed before appending so it does not sit above fetched rows.
   - Fetch never deletes, reorders, or rewrites a row the user already filled.

10. After a completed fetch, an inline status line under the model actions reports exactly one of:
    - `Found N models` when N ≥ 1 new rows were appended (N is the number added, not the catalog size).
    - `No new models` when the catalog was valid but every ID was already present.
    - `Fetch failed: unauthorized` for HTTP 401 or 403.
    - `Fetch failed: not found` for HTTP 404.
    - `Fetch failed: network error` for DNS, TLS, connection, or timeout failures.
    - `Fetch failed: unexpected response` for non-JSON, HTML, missing `data`, or a body larger than 1 MiB.
    - `Fetch failed: no models` when JSON is valid but `data` is empty after dropping blank IDs.
    Failure copy uses the theme error color. Success and idle copy use the same secondary text color as the modal description.

11. Failure never adds blank rows, never clears existing rows, and never crashes or hangs the modal.

12. Only one fetch runs at a time. A second click while Fetching… is ignored. Closing the modal, prefilling it for a different endpoint, or starting a new fetch after the previous one finished cancels or discards any in-flight result so it cannot apply to the wrong form.

13. Status returns to idle (no leftover Fetching…, no stale success/error from a previous endpoint) when the modal is closed or prefilled for another endpoint.

14. The API key, Authorization header, and response body never appear in the status line, logs, toasts, or telemetry. Error copy uses the generic phrases in (10), not upstream body text.

15. Fetch does not relax Save rules. Save still requires a non-empty name, valid URL, non-empty API key, and at least one model row with a non-empty name. A failed fetch that leaves only a blank row keeps Save disabled.

16. After a successful fetch, Save / Add endpoint behaves as today: the in-modal rows (typed + fetched) become the endpoint’s model list, each with a stable identity. Picker labels still prefer alias, then name. Existing saved selections keep working for rows whose names already existed.

17. TUI `/api-keys`, TUI `settings.toml` editing, and cloud agents do not gain a fetch action. GUI-fetched rows become ordinary definition rows once saved, so the TUI can attach a key to that endpoint like any other.

18. Warp does not poll `/models`, does not fetch when the modal opens, and does not fetch when the model picker opens.
