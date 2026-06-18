# Product Spec: `--jq` filter flag for `oz run get` and `oz run list`

Linear: [REMOTE-1393](https://linear.app/warpdotdev/issue/REMOTE-1393)
Figma: none (CLI-only change)

## Summary

Add a `--jq "<filter>"` flag to `oz run get` and `oz run list`. When set, the command runs the server's JSON response through the given jq filter (using the `jaq` Rust implementation) and writes the filter's result to stdout instead of printing the raw response.

## Problem

[REMOTE-1374](https://linear.app/warpdotdev/issue/REMOTE-1374) landed `--output-format json` on both commands and deliberately deferred first-class JQ-style filtering as a follow-up. In practice, the scripting pattern it unlocks still requires an external `jq` binary on the user's `PATH`:

```
oz run list --state failed --output-format json | jq -r '.runs[].task_id'
```

That's fine for interactive use but awkward in CI, sandboxes, or remote environments where `jq` is not guaranteed to be installed, and it adds a process boundary that complicates error handling and quoting. Bringing filtering into the CLI itself makes one-off inspection commands easier and keeps scripts portable.

## Behavior

1. Both `oz run list` and `oz run get` accept an optional `--jq <FILTER>` flag. `<FILTER>` is a jq filter expression in the dialect implemented by [`jaq`](https://github.com/01mf02/jaq) with the full standard library enabled (via `jaq-all`). When not set, both commands behave exactly as they do today.

2. When `--jq` is set, the input to the filter is the same JSON document the command would emit under `--output-format json`:
   - For `oz run list`, the full list response envelope, e.g. `{ "runs": [...], "page_info": { ... } }`.
   - For `oz run get <run_id>`, the single run object returned by `GET /api/v1/agent/runs/:runId`.

3. `--jq` and `--output-format` interact as follows:
   - `--jq` implies JSON mode: whatever `--output-format` is set to (including the default `pretty`), the JSON fetch path is used to obtain the response, and the filter's output is what is printed. The `pretty`/`text` table renderers are skipped entirely.
   - `--output-format json` together with `--jq` is accepted and equivalent.
   - `--output-format text` together with `--jq` is accepted and equivalent (table output is skipped; only the filter output is printed).

4. Output format of the filter's result:
   - Each filter output value is printed on its own line, terminated by `\n`.
   - **Scalar values are printed as raw text**, not JSON-encoded. Specifically: strings are printed without surrounding double quotes and without JSON escapes; numbers are printed in their natural form; `true`, `false`, and `null` are printed as the literal words `true`, `false`, and `null`. This matches `gh --jq`'s behavior ([cli/cli#3012](https://github.com/cli/cli/pull/3012)) and means `oz run list --jq '.runs[].task_id'` prints one bare task ID per line, not one JSON-quoted string per line.
   - Non-scalar values (arrays and objects) are pretty-printed as JSON across multiple lines, consistent with the non-filtered `--output-format json` behavior.
   - `--jq .` on either command therefore produces byte-identical output to the same command with `--output-format json` (the top-level response is always an object; objects go through the JSON path).
   - If the filter produces zero output values (for example, `empty`), the command prints nothing and exits `0`.

5. Filter errors are surfaced clearly:
   - A filter that fails to parse (syntax error, unknown function, etc.) causes the command to exit with a non-zero status and prints a human-readable error to stderr that includes the filter text and, where `jaq` provides it, a location within the filter. Parse errors fire _before_ any HTTP request is issued, so a typo in `--jq` never triggers a network call or consumes credits.
   - A runtime error during filter execution (for example, `.foo | .bar` on a number) causes the command to exit with a non-zero status and prints the `jaq` runtime error message to stderr. Any output values already produced before the error are still written to stdout.

6. `--jq` does not change which HTTP requests the CLI makes: on success, exactly one `GET /api/v1/agent/runs` (or `.../runs/:runId`) request, identical to the request issued by `--output-format json` without `--jq`. On a filter parse error, zero requests are made (see invariant 5). The filter is applied entirely client-side on the parsed response.

7. `--jq` does not change server-side filtering behavior. Existing `oz run list` filter flags (`--state`, `--source`, etc.) compose with `--jq`: the server narrows the response first, and the jq filter runs on the narrowed response.

8. `--jq` is documented in `oz run list --help` and `oz run get --help`. The help text:
   - Names the flag (`--jq <FILTER>`).
   - Explains that it runs the command's JSON response through a jq filter.
   - Notes that `--jq` implies JSON output and that the filter dialect is jq-compatible via `jaq`.
   - Gives at least one example invocation.

9. Invariants that must not regress:
   - `oz run list` and `oz run get` with no `--jq` produce byte-identical output to today for each `--output-format`.
   - The `--output-format json` payload shape is unchanged — the filter reads the existing response, not a new one.
   - `--jq ""` (empty string) is rejected as an invalid filter via the same error path as any other parse error; it does not silently pass input through. Users wanting pass-through should omit `--jq` or use `--jq .`.
   - Scalar unwrapping applies only to the _top-level_ filter output. Scalars that appear _inside_ an array or object are JSON-encoded as usual (so `--jq '.runs'` quotes string fields like `title` inside the returned array, but `--jq '.runs[].title'` does not).

## Non-goals

- No explicit `-r`/`--raw-output` flag. Scalar unwrapping (invariant 4) covers the common `jq -r` use case; users who want string values to be JSON-quoted at the top level can wrap the filter, e.g. `--jq '. | tojson'`.
- No `--jq-from-file`, variables (`--arg`), or multi-input streaming. The scope is a single inline filter applied to the single JSON response.
- No `--template` / Go-template alternative ([gh formatting docs](https://cli.github.com/manual/gh_help_formatting)). Separate ask if we ever want it.
- No TTY-aware colorization or compact-when-piped output ([cli/cli#7236](https://github.com/cli/cli/pull/7236)). Output is always pretty-printed for non-scalars, same as `--output-format json` today.
- No `--jq` on other commands (e.g. `oz run conversation get`, `oz environment list`). The implementation is structured so that adding `--jq` to any command that already uses `print_raw_json` is a small, mechanical follow-up, but those rollouts are out of scope here.
- No first-class `jq`-style error codes (jq reserves specific exit codes for "no output" / "false/null output"). The CLI uses its normal success/failure exit codes.

## Example invocations

```
# Pull just the task IDs of failed runs from the last day.
oz run list --state failed --updated-after 2026-04-20T00:00:00Z \
  --jq '.runs[].task_id'

# Get a single field from a run.
oz run get 01HX... --jq '.state'

# Compose with other flags; --jq runs after server-side filters.
oz run list --source CLI --limit 100 \
  --jq '[.runs[] | select(.harness == "claude")] | length'
```
