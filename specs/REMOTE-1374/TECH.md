# Tech Spec: Filtering and JSON output for `oz run list` and `oz run get`

See `specs/REMOTE-1374/PRODUCT.md` for the product spec.

## 1. Problem

`oz run list` and `oz run get` are wired as `TaskCommand::List` / `TaskCommand::Get` subcommands in `crates/warp_cli/src/task.rs`, and are implemented by `AmbientAgentRunner::{list_tasks,get_task_status}` in `app/src/ai/agent_sdk/ambient.rs`. Today:

- `ListTasksArgs` exposes only `--limit`.
- `TaskGetArgs` takes only `task_id` (and `--conversation`, which is out of scope).
- Both commands ignore `--output-format`; they always call `print_tasks_table()` which renders a pretty ASCII card-style table.
- The client-side filter struct `TaskListFilter` (in `app/src/server/server_api/ai.rs`) only covers a subset of what the REST endpoint accepts: `creator`, `updated_after`, `states`, `source`, `created_after`, `environment_id`. The server additionally supports `created_before`, `q`, `name`, `model_id`, `skill` / `skill_spec`, `schedule_id`, `ancestor_run_id`, `artifact_type`, `execution_location`, `sort_by`, `sort_order`, and `cursor`.
- `AIClient::list_ambient_agent_tasks` deserializes straight into `Vec<AmbientAgentTask>` and discards the surrounding response envelope (`runs` / `tasks`, `page_info`), so there's no way to forward `next_cursor` or future fields to the CLI.

We need to (a) extend `TaskListFilter` to cover every server filter, (b) expand `ListTasksArgs` with CLI-level flags and wire them into the filter, (c) thread the global `--output-format` into `run_task`, and (d) give the CLI access to the raw server JSON so `--output-format json` can mirror the public API schema 1:1.

## 2. Relevant Code

- `crates/warp_cli/src/task.rs (1-43)` — `TaskCommand`, `ListTasksArgs`, `TaskGetArgs`.
- `crates/warp_cli/src/lib.rs:444-500` — `CliCommand::Run(TaskCommand)` wiring and `Args` plumbing.
- `crates/warp_cli/src/agent.rs:10-30` — the existing `OutputFormat` enum used by the `--output-format` global flag.
- `app/src/ai/agent_sdk/mod.rs:125-139` — `dispatch_command` routes `CliCommand::Run` to `run_task`.
- `app/src/ai/agent_sdk/mod.rs:464-492` — `run_task` translates the subcommand into calls on `ambient::*`.
- `app/src/ai/agent_sdk/ambient.rs:43-61` — `list_ambient_agent_tasks` / `get_ambient_agent_task_status` entry points from the CLI dispatcher.
- `app/src/ai/agent_sdk/ambient.rs (457-488)` — `AmbientAgentRunner::list_tasks` / `get_task_status` bodies (call `ai_client.list_ambient_agent_tasks` / `get_ambient_agent_task`, then `print_tasks_table`).
- `app/src/ai/agent_sdk/ambient.rs (594-664)` — `print_tasks_table` pretty renderer and its helpers.
- `app/src/ai/agent_sdk/output.rs` — shared `print_list`/`write_list` helpers that handle `OutputFormat::{Pretty,Text,Json}` for tabular output.
- `app/src/server/server_api/ai.rs:472-513` — `TaskListFilter` and the internal `ListTasksResponse` deserializer (drops `page_info`).
- `app/src/server/server_api/ai.rs:1167-1214` — `AIClient::list_ambient_agent_tasks` / `get_ambient_agent_task` HTTP plumbing that builds the query string and calls `get_public_api`.
- `app/src/server/server_api.rs:571-619` — `get_public_api` / `get_public_api_response`. The latter returns the raw HTTP response so we can parse into `serde_json::Value` directly.
- `app/src/ai/ambient_agents/task.rs (100-285)` — `AmbientAgentTaskState::as_query_param` and `AgentSource::as_str`; reusable for new filters.
- `public_api/openapi.yaml (209-428)` on the server side — documents the exact query parameter names, allowed values, and response envelope we mirror. (Authoritative reference for any ambiguity in the product spec.)

## 3. Proposed Changes

The work splits into four layers: CLI args, client-side filter, API client, and output. Keep each layer narrow and testable on its own.

### 3a. CLI args (`crates/warp_cli/src/task.rs`)

Expand `ListTasksArgs` with one field per product-spec flag. Use clap `ValueEnum` for the enum-typed flags so invalid values fail parsing with a useful error. Keep all new fields `Option<_>` except `--state`, which is `Vec<RunStateArg>` (empty means "no filter").

New value-enum types (defined in `task.rs`, kept `pub`):

```rust path=null start=null
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunStateArg {
    #[value(name = "queued")] Queued,
    #[value(name = "pending")] Pending,
    #[value(name = "claimed")] Claimed,
    #[value(name = "in-progress")] InProgress,
    #[value(name = "succeeded")] Succeeded,
    #[value(name = "failed")] Failed,
    #[value(name = "error")] Error,
    #[value(name = "blocked")] Blocked,
    #[value(name = "cancelled")] Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunSourceArg { /* API, CLI, LOCAL, SLACK, LINEAR, SCHEDULED_AGENT, WEB_APP, CLOUD_MODE, GITHUB_ACTION, INTERACTIVE */ }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExecutionLocationArg { Local, Remote }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ArtifactTypeArg { Plan, PullRequest, Screenshot, File }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunSortByArg { #[value(name="updated-at")] UpdatedAt, #[value(name="created-at")] CreatedAt, Title, Agent }

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunSortOrderArg { Asc, Desc }
```

Timestamp flags are parsed with `clap::value_parser!` using a small wrapper that calls `DateTime::parse_from_rfc3339` and converts to `DateTime<Utc>`; this keeps validation at the clap layer rather than surfacing server errors.

Updated `ListTasksArgs` keeps `limit` as-is (default 10) and adds the fields listed in the product spec. Field naming: Rust fields use snake_case, the clap attribute sets the long name (e.g. `#[arg(long = "created-after")]`). `-q` / `--query` maps to `query: Option<String>`.

`TaskGetArgs` is unchanged. (No new flags; only the output path changes.)

### 3b. Client-side filter (`app/src/server/server_api/ai.rs`)

Extend `TaskListFilter` with the missing fields so every server parameter is reachable:

```rust path=null start=null
#[derive(Clone, Debug, Default)]
pub struct TaskListFilter {
    pub creator_uid: Option<String>,
    pub updated_after: Option<DateTime<Utc>>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub states: Option<Vec<AmbientAgentTaskState>>,
    pub source: Option<AgentSource>,
    pub execution_location: Option<ExecutionLocation>, // new enum, API-only
    pub environment_id: Option<String>,
    pub skill_spec: Option<String>,
    pub schedule_id: Option<String>,
    pub ancestor_run_id: Option<String>,
    pub config_name: Option<String>,
    pub model_id: Option<String>,
    pub artifact_type: Option<ArtifactType>, // new enum, mirrors server values
    pub search_query: Option<String>,
    pub sort_by: Option<RunSortBy>,
    pub sort_order: Option<RunSortOrder>,
    pub cursor: Option<String>,
}
```

Add matching enums (`ExecutionLocation`, `ArtifactType`, `RunSortBy`, `RunSortOrder`) next to `TaskListFilter` with an `as_query_param()` that returns the exact server string (e.g. `"updated_at"`, `"PLAN"`). These mirror the clap enums but live in the server-api layer so we don't couple clap-specific value names to the HTTP layer.

Update `list_ambient_agent_tasks` to append every new field to the URL. The existing code uses manual `push_str` + `urlencoding::encode`; rewrite the URL construction using `url::Url::parse` + `url_builder.query_pairs_mut()` for readability and to avoid double-encoding bugs. Each `Option<T>` contributes one `&key=value` pair when present; `states` contributes one pair per entry.

The `ListTasksResponse` deserializer currently drops `page_info`. Keep the existing typed return (`Vec<AmbientAgentTask>`) for the UI callers who already rely on it, and introduce a new method for the CLI. Name the new methods with `agent_runs` (not `ambient_agent_tasks`) to match the preferred `/agent/runs` REST naming — these are CLI-only, so there's no back-compat pressure on the existing typed names:

```rust path=null start=null
async fn list_agent_runs_raw(
    &self,
    limit: i32,
    filter: TaskListFilter,
) -> anyhow::Result<serde_json::Value>;

async fn get_agent_run_raw(
    &self,
    task_id: &AmbientAgentTaskId,
) -> anyhow::Result<serde_json::Value>;
```

Both implementations build the same URL as the typed variants but call `get_public_api_response` (already exists at `server_api.rs:587`) and then `response.json::<serde_json::Value>().await`. Route the list call to `/api/v1/agent/runs` (not `/agent/tasks`) so the envelope uses the preferred `runs` key; the server treats both paths the same and the web app uses `/runs`.

Add a shared private helper `fn build_list_tasks_url(limit, filter) -> String` used by both the typed and raw methods, and unit-test it against a table of `(filter, expected_query_string)` cases.

Note on the mock: because `AIClient` uses `#[cfg_attr(test, automock)]`, the new trait methods automatically get mockall-generated counterparts. No test harness changes needed beyond that.

### 3c. Plumbing in the CLI dispatcher (`app/src/ai/agent_sdk/mod.rs`, `ambient.rs`)

- Change `run_task(ctx, command)` to `run_task(ctx, global_options: GlobalOptions, command)` and pass the full `GlobalOptions` through from `dispatch_command` (`mod.rs:139`). This follows the convention used by the other CLI subcommand runners (`environment::run`, `schedule::run`, `secret::run`, etc.), so future global flags don't require changing each subcommand signature.
- `run_task` passes `global_options` to `ambient::list_ambient_agent_tasks` and `ambient::get_ambient_agent_task_status`. The public `ambient::*` entry points take `GlobalOptions` as well; `AmbientAgentRunner::{list_tasks, get_task_status}` read `global_options.output_format` internally.
- `AmbientAgentRunner::list_tasks` builds a `TaskListFilter` from `ListTasksArgs` via a new helper `fn filter_from_args(args: &ListTasksArgs) -> anyhow::Result<TaskListFilter>` (local to `ambient.rs`). The mapping is mechanical: each clap enum converts to its server-api counterpart, strings pass through unchanged.
- The runner then branches on `global_options.output_format`:
  - `Json` → call `list_agent_runs_raw` and `println!("{}", serde_json::to_string_pretty(&value)?)`.
  - `Pretty` / `Text` → call `list_ambient_agent_tasks` (typed) and render the existing table.
- `get_task_status` applies the same branching against the raw/typed `get_agent_run_raw` / `get_ambient_agent_task` variants.

Rationale for picking raw-vs-typed up front (rather than always fetching raw and re-deserializing): one HTTP request per invocation, no duplicated work, and the typed struct stays the single source of truth for pretty/text rendering.

### 3d. Output helpers

The existing `print_tasks_table` stays unchanged and continues to own pretty/text rendering; it is behavior-preserving for users who don't pass `--output-format json`. Text (`OutputFormat::Text`) is already served by falling through to the same card renderer (matching today's behavior). If follow-up work later wants a true tab-separated run list, it can add a `TableFormat` impl for `AmbientAgentTask` and call `print_list`, but that is explicitly out of scope for this ticket.

For JSON, the raw `serde_json::Value` is written with `serde_json::to_string_pretty(&value)?`. No per-command formatter trait is needed.

## 4. Feature Flagging and Back-Compat

- No new feature flag. These commands are already GA behind `AmbientAgentsCommandLine`, which controls the subcommand entirely; within the enabled surface, adding flags and honoring `--output-format` is safe.
- Existing scripts that call `oz run list` or `oz run get` without `--output-format` keep the same pretty output.
- The URL moves from `/api/v1/agent/tasks` to `/api/v1/agent/runs` for the CLI. Server-side both routes are supported and return the same data, but the `/runs` variant is the preferred spelling going forward (it's what the web app and openapi.yaml use) and uses the `"runs"` envelope key the product spec assumes. The typed `list_ambient_agent_tasks` should also migrate in the same change so the CLI and UI don't diverge.

## 5. Testing Plan

Unit tests (added as `*_tests.rs` alongside the edited files, per the repo convention):

- `crates/warp_cli/src/task_tests.rs` (new):
  - Parse a full `oz run list` command with every new flag and assert the resulting `ListTasksArgs` fields.
  - Confirm invalid enum values exit with a non-zero clap error.
  - Confirm RFC 3339 parsing rejects malformed timestamps.
- `app/src/server/server_api/ai_tests.rs` (augment existing file):
  - Table-driven test of `build_list_tasks_url` covering every field, repeated `states`, and the empty filter. Compare the resulting query string exactly, including percent-encoding of timestamps.
- `app/src/ai/agent_sdk/ambient_tests.rs` (new):
  - `filter_from_args` produces the expected `TaskListFilter` for each combination of clap inputs.
  - With a mocked `AIClient`, exercise `list_tasks` / `get_task_status` with `OutputFormat::{Json, Pretty}` and assert that the correct trait method was called (raw vs typed) and nothing was fetched twice.

Manual validation against staging:

- Run `oz run list` with every new flag against a known fixture of runs and compare the JSON to `curl` hitting `GET /agent/runs` directly.
- Confirm `--cursor` paginates correctly for each `--sort-by` value.
- Confirm human-readable error messages and non-zero exit codes when the server rejects the request (invalid cursor, too-old `created_after`, unknown environment ID, etc.).

Presubmit: run `./script/presubmit` (fmt, clippy `-D warnings`, test suite). The touched crates are `warp_cli` and `warp` (app); the test suite should run quickly for these.

## 6. Rollout

- Single PR, no server-side changes, no schema migrations.
- No feature flag. CHANGELOG entry: `CHANGELOG-IMPROVEMENT: oz run list and oz run get now support --output-format json and a full set of filter/sort flags`.

## 7. Risks and Mitigations

- **Flag name drift vs the API**: We use terminal-ergonomic flag names (`--environment`, `--skill`, `-q`) while the API uses `environment_id`, `skill_spec`, `q`. Mitigation: document the mapping in `--help` and in openapi/product docs.
- **JSON shape drift vs the Rust struct**: Because JSON output bypasses `AmbientAgentTask` entirely and goes through `serde_json::Value`, new server fields appear in CLI output automatically. This is the desired behavior but means a malformed server response is surfaced verbatim. The existing tolerant deserializer (which drops bad task entries) only applies to the pretty path; for JSON the raw bytes go through as-is, which is the point.
- **Cursor bugs are silently wrong**: If a caller reuses a cursor with a different `--sort-by`, the server returns `400`. Mitigation: product spec documents this; we surface the server error unchanged. Not worth client-side validation.
- **Mockall surface growth**: Adding two new trait methods to `AIClient` grows the mock surface. Mitigation: the methods are small and only used in two new callsites; test coverage is additive.

## 8. Follow-Ups (not in this ticket)

- First-class JQ-style `--filter` support on the CLI output.
- A terse tabular JSON-to-table renderer (`TableFormat for AmbientAgentTask`) so `--output-format text` produces one row per run instead of the card layout.
- Mirror this work for `oz run conversation get` / `oz run get --conversation`.

## 9. Files Changed

- **Modified**: `crates/warp_cli/src/task.rs` — expand `ListTasksArgs`; add `RunStateArg` / `RunSourceArg` / `ExecutionLocationArg` / `ArtifactTypeArg` / `RunSortByArg` / `RunSortOrderArg` value enums; add `value_parser` wrappers for RFC 3339 timestamps.
- **New**: `crates/warp_cli/src/task_tests.rs` — clap parsing tests for the new flags.
- **Modified**: `app/src/server/server_api/ai.rs` — extend `TaskListFilter`; add `ExecutionLocation`, `ArtifactType`, `RunSortBy`, `RunSortOrder` enums with `as_query_param`; add `list_agent_runs_raw` / `get_agent_run_raw`; move URL construction to a shared `build_list_tasks_url` helper; route to `/agent/runs`.
- **Modified**: `app/src/server/server_api/ai_tests.rs` — URL-builder tests.
- **Modified**: `app/src/ai/agent_sdk/mod.rs` — thread `GlobalOptions` from `dispatch_command` into `run_task`.
- **Modified**: `app/src/ai/agent_sdk/ambient.rs` — add `filter_from_args`; branch `list_tasks` / `get_task_status` on `OutputFormat`; call raw methods for JSON, typed methods for pretty/text.
- **New**: `app/src/ai/agent_sdk/ambient_tests.rs` (or augment if one exists) — tests for `filter_from_args` and the `list_tasks` / `get_task_status` branching against a mocked `AIClient`.
