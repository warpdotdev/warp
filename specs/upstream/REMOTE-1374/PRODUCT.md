# Product Spec: Filtering and JSON output for `oz run list` and `oz run get`

Linear: [REMOTE-1374](https://linear.app/warpdotdev/issue/REMOTE-1374)
Related customer report: [CSAT-8397](https://linear.app/warpdotdev/issue/CSAT-8397)
Figma: none (CLI-only change)

## Summary

Bring the `oz run list` and `oz run get` CLI commands to parity with the public REST API and the Oz web app by (1) wiring up JSON output and (2) adding filter and sort flags to `oz run list`. JSON output mirrors the server's REST schema so that scripting against `oz run list`/`oz run get` is equivalent to scripting against `GET /api/v1/agent/runs` and `GET /api/v1/agent/runs/:runId`.

## Problem

Today the `oz run list` and `oz run get` commands are useful for a human glancing at recent runs, but they do not let users script against Oz from the shell:

- Neither command honors the global `--output-format=json` flag. Both always render a pretty ASCII table, which is the only output mode.
- `oz run list` only exposes `--limit`. None of the other server-supported filters are reachable from the CLI, so users cannot narrow to a specific state, source, creator, environment, time range, or search query.
- The REST API already supports all of this (see `openapi.yaml` for `GET /agent/runs`). Users who hit the limits of the CLI have told us they end up hand-crafting `curl` calls instead, which defeats the purpose of having a CLI.

This gap is called out by a GitHub customer report (CSAT-8397) and has been blocking scripting workflows.

## Goals

1. `oz run list` and `oz run get` honor the existing global `--output-format` flag (`pretty`, `text`, `json`). `pretty` stays the default and matches today's behavior.
2. JSON output from the CLI is the server's JSON response, passed through unmodified (parsed into `serde_json::Value`, then re-emitted as pretty-printed JSON). This keeps the CLI output in lockstep with the REST API / web app over time without requiring schema changes on the client.
3. `oz run list` exposes flags for the filter/sort query parameters that are useful from a terminal workflow (see the flag list below).
4. Help text is clear enough that `oz run list --help` is a reasonable scripting reference on its own.

## Non-Goals

- No changes to `oz run conversation get` or `oz run get --conversation`. Those already emit JSON and are out of scope for this ticket.
- No changes to `oz run get`'s filter surface. `oz run get <run_id>` takes a single ID and continues to do so.
- No new pretty-table columns. The pretty output stays functionally unchanged; the only new behavior is that `--output-format json` starts working.
- No JQ-like filter support on the CLI side in this change. The shape of the JSON output is chosen so that `oz run list --output-format json | jq ...` is the expected scripting pattern, and first-class JQ support is a follow-up.
- No changes to the server API. We only expose existing query parameters through the CLI.

## CLI Surface

### `oz run list`

Existing flag, unchanged:
- `-L, --limit <N>` — Maximum number of runs to return. Default `10`. Server caps at 500.

New flags (all optional; defaults preserve current behavior):

Filtering:
- `--state <STATE>` — Filter by run state. Accepts `queued`, `pending`, `claimed`, `in-progress`, `succeeded`, `failed`, `error`, `blocked`, `cancelled`. Repeatable; specifying multiple states matches any of them.
- `--source <SOURCE>` — Filter by run source. Accepts: `API`, `CLI`, `SLACK`, `LINEAR`, `SCHEDULED_AGENT`, `WEB_APP`, `CLOUD_MODE`, `GITHUB_ACTION`, `INTERACTIVE`. `INTERACTIVE` is the CLI-friendly name for the public API's `LOCAL` source (used for local interactive tasks — Agent Mode and local CLI runs); the client maps `--source INTERACTIVE` back to `source=LOCAL` in the request.
- `--execution-location <LOC>` — Filter by where the run executed. Accepts `local`, `remote`.
- `--creator <UID>` — Filter by creator UID (user or service account).
- `--environment <ENV_ID>` — Filter by environment ID.
- `--skill <SPEC>` — Filter by skill spec (e.g. `owner/repo:path/to/SKILL.md`).
- `--schedule <SCHEDULE_ID>` — Filter to runs created by a specific scheduled agent.
- `--ancestor-run <RUN_ID>` — Filter to descendants of a specific run.
- `--name <NAME>` — Filter by agent config name.
- `--model <MODEL_ID>` — Filter by model ID.
- `--artifact-type <TYPE>` — Filter by produced artifact type. Accepts `plan`, `pull-request`, `screenshot`, `file`.
- `--created-after <RFC3339>` — Only include runs created after the given RFC 3339 timestamp.
- `--created-before <RFC3339>` — Only include runs created before the given RFC 3339 timestamp.
- `--updated-after <RFC3339>` — Only include runs updated after the given RFC 3339 timestamp.
- `-q, --query <TEXT>` — Fuzzy search across run title, prompt, and skill spec.

Sorting and pagination:
- `--sort-by <FIELD>` — Sort field. Accepts `updated-at` (default), `created-at`, `title`, `agent`.
- `--sort-order <DIR>` — Sort direction. Accepts `asc`, `desc` (default).
- `--cursor <CURSOR>` — Opaque pagination cursor from a previous list response's `page_info.next_cursor`. When using `--cursor`, `--sort-by` and `--sort-order` must match the values used to obtain the cursor (server-enforced).

We deliberately use `--environment`, `--skill`, `--schedule`, `--ancestor-run`, `--created-after`, `--created-before`, `--updated-after`, and `-q/--query` as the flag names (instead of the API's `environment_id`, `skill_spec`, `schedule_id`, `ancestor_run_id`, etc.). The flag names are tuned for terminal ergonomics; under the hood they map to the existing API query parameters.

### `oz run get`

No flag changes. `oz run get <run_id>` continues to take a single run ID. Only the output layer changes (see below).

### Output

Behavior by `--output-format`:

- `pretty` (default): unchanged. Renders the same card-style ASCII table that exists today.
- `text`: unchanged. Same table rendered without box-drawing characters.
- `json`: new.
  - `oz run list --output-format json` prints one pretty-printed JSON object to stdout: exactly the body of `GET /api/v1/agent/runs`, i.e. `{ "runs": [...], "page_info": { "has_next_page": ..., "next_cursor": "..." } }`. (The key is `runs` when hit through `/agent/runs`; we use the `/agent/runs` path so the CLI matches the web app and the REST docs.)
  - `oz run get <run_id> --output-format json` prints one pretty-printed JSON object: exactly the body of `GET /api/v1/agent/runs/:runId`.
  - In both cases the client makes one request and passes the response body through a `serde_json::Value` parse, then re-serializes with `serde_json::to_string_pretty`. No fields are dropped, renamed, or reinterpreted on the client. Future server-side additions show up in the CLI automatically.
  - Exit code is `0` on a successful fetch and nonzero on any error. Errors are printed to stderr in human-readable form (same as today); they are not rendered as JSON.

## User Experience

### Example: scripting recent failed runs

```
oz run list \
  --state failed --state error \
  --updated-after 2026-04-01T00:00:00Z \
  --output-format json \
  | jq -r '.runs[] | "\(.task_id) \(.title)"'
```

### Example: paginating

```
# First page, sorted by creation time
oz run list --limit 50 --sort-by created-at --output-format json > page1.json

# Follow the cursor
CURSOR=$(jq -r .page_info.next_cursor page1.json)
oz run list --limit 50 --sort-by created-at --cursor "$CURSOR" --output-format json
```

### Example: fetching a single run as JSON

```
oz run get 01HX9Y... --output-format json | jq .state
```

### Example: existing pretty output (unchanged)

```
$ oz run list --limit 3
Agent Runs (3):
...  (existing table output)
```

## Invariants and Edge Cases

- `--state` is repeatable and matches any of the given states (OR semantics), consistent with the server.
- `--source` is single-valued. Unknown values produce a clap-level error with the list of accepted values.
- `--execution-location`, `--artifact-type`, `--sort-by`, `--sort-order` are single-valued and validated by clap against the allowed set. Values are accepted case-insensitively where reasonable; we standardize on lowercase in help text.
- Timestamp flags accept RFC 3339. Invalid timestamps are rejected with a clap parse error before the request is sent.
- When `--cursor` is provided with a `--sort-by` or `--sort-order` that disagrees with the cursor, the server returns a `400`. The CLI surfaces that error verbatim; no extra client-side validation.
- Filters that don't match any rows return an empty `runs` array (JSON) or `No runs found.` (pretty/text). The `page_info` block is still present in JSON.
- Permissions are unchanged: the server already scopes list responses to the authenticated principal's personal runs + team runs. The CLI does not add or remove any scoping on top.
- Pretty/text rendering continues to deserialize into the existing Rust `AmbientAgentTask` struct. Fields unknown to the client are ignored for rendering but retained in JSON output because JSON goes through `serde_json::Value` directly.
- Total JSON output size is bounded by the `--limit` cap. For a single run via `oz run get`, the payload is small (no full conversation transcript).

## Success Criteria

1. `oz run list --output-format json` prints the exact JSON body returned by `GET /api/v1/agent/runs`, pretty-printed.
2. `oz run get <run_id> --output-format json` prints the exact JSON body returned by `GET /api/v1/agent/runs/:runId`, pretty-printed.
3. Every filter listed above is reachable from the CLI, maps to the corresponding API query parameter, and is validated at the CLI layer for enum-typed flags.
4. Running `oz run list` with no new flags produces the same pretty output as before.
5. `oz run list --help` documents all filter, sort, and pagination flags, including accepted values for the enum flags.
6. Errors from the server (invalid cursor, invalid timestamp, etc.) are printed to stderr with a non-zero exit code, consistent with current behavior.

## Validation

- Rust unit tests for CLI argument parsing: each new flag parses into the expected filter value; mutually exclusive or enum-typed flags reject invalid input.
- Rust unit tests for the client-side filter-to-URL translation: every flag shows up as the correct query parameter in the constructed request URL. Repeated `--state` values each produce a `&state=...` pair.
- Rust unit tests for the output layer: JSON output equals the raw server JSON (byte-identical modulo pretty-printing); pretty/text output renders the same columns as before.
- Manual validation against staging:
  - Exercise each filter with a handful of real runs and compare CLI output to the web app.
  - Confirm `--cursor` paginates correctly for each `--sort-by`.
  - Confirm errors (invalid RFC 3339, invalid cursor) are surfaced with a non-zero exit code.
- Help-text snapshot: `oz run list --help` and `oz run get --help` are captured in a test to catch accidental regressions.
