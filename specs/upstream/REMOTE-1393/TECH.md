# Tech Spec: `--jq` filter flag for `oz run get` and `oz run list`

See `specs/REMOTE-1393/PRODUCT.md` for the product spec.

This builds on top of REMOTE-1374. The `--output-format json` wiring and the `print_raw_json` helper already exist; this change adds a filter-processing layer in front of `print_raw_json` and a pair of clap flags.

Two guiding constraints beyond the product spec:

- `--jq` is a pattern we want on any command whose default JSON path goes through `print_raw_json`. The clap wiring lives in a single reusable struct (`JsonFilter`) embedded via `#[command(flatten)]`, so adding `--jq` to a new command is a one-line change.
- Filter compilation happens at clap parse time, not after the API response lands. Typos fail fast, before auth or HTTP activity. This is achieved with a custom `value_parser` that runs `jaq_core::Compiler::compile` during argument parsing.

## Context

The JSON output path for `oz run list` and `oz run get` is already in place. The relevant code after REMOTE-1374:

- `crates/warp_cli/src/task.rs:101` — `ListTasksArgs` (all existing filter flags).
- `crates/warp_cli/src/task.rs:281` — `TaskGetArgs`.
- `app/src/ai/agent_sdk/ambient.rs:566` — `AmbientAgentRunner::list_tasks`, which branches on `OutputFormat` and calls `print_raw_json` for `Json`.
- `app/src/ai/agent_sdk/ambient.rs:593` — `AmbientAgentRunner::get_task_status`, same pattern for the single-run response.
- `app/src/ai/agent_sdk/output.rs:80` — `print_raw_json(value: &serde_json::Value) -> anyhow::Result<()>`. The only shared JSON emitter; extending it is the user's stated preference and the lowest-footprint place to plug in a filter.
- `app/Cargo.toml:171` — `serde_json.workspace = true`. The workspace already enables `serde_json` with `features = ["raw_value"]`; we'll continue to use `serde_json::Value` for the filter input.

The `jaq` crates (MIT licensed, pure-Rust) provide a jq-compatible implementation. We depend on the three relevant crates directly and skip `jaq-all` / `jaq-fmts`, since `jaq-core` 3.0 provides everything we need:

- `jaq-core` 3.0 — parser, compiler, and interpreter. Exposes `Filter`, `Compiler`, `Ctx`, `defs()`, `funs()`, and the `data::JustLut<V>` `DataT` helper that lets us skip writing a custom `Data` / `DataKind`. The crate-level example in the `jaq-core` docs is exactly the compile+run path we use.
- `jaq-std` 3.0 — the jq standard library (`length`, `map`, `select`, etc.) as additional `defs()` and `funs()` to chain into the compiler.
- `jaq-json` 2.0 — the `Val` type used across the jaq ecosystem. With `features = ["serde"]` enabled, `Val` implements `serde::Deserialize`/`Serialize`, which is the path the user called out for `serde_json` compatibility (see `https://github.com/01mf02/jaq/blob/main/jaq-json/tests/common/mod.rs`, where `from_value::<Val>` converts a `serde_json::Value` to `Val`).

All three are `no_std`-friendly so they compile cleanly in the same configurations `output.rs` already compiles in.

## Proposed changes

Five small edits: add the dependency, introduce a reusable `JsonFilter` clap component that compiles the filter at parse time, embed it in both `Args` structs, extend `print_raw_json`, and thread the filter through the two call sites.

### 1. Dependencies

In `Cargo.toml` (workspace), add under `[workspace.dependencies]`:

```toml path=null start=null
jaq-core = "3.0"
jaq-std = "3.0"
jaq-json = { version = "2.0", features = ["serde"] }
```

In both `app/Cargo.toml` and `crates/warp_cli/Cargo.toml`, add:

```toml path=null start=null
jaq-core.workspace = true
jaq-std.workspace = true
jaq-json.workspace = true
```

`warp_cli` compiles the filter at clap parse time; `app` runs it. `warp_cli` technically only uses `jaq-core` + `jaq-std` items in its own code, but depends on `jaq-json` too because the stored `Filter` type is parameterized by `jaq_json::Val` — without `Val` in scope, the `Filter` field would not resolve. The alternative (store `Option<String>` in `warp_cli` and recompile in `app`) doubles the compile work and prevents sharing a single compile source of truth; keep the direct `jaq-json` dep and forgo that split unless the binary-size hit shows up as a problem.

### 2. Reusable `JsonFilter` clap component (`crates/warp_cli/src/json_filter.rs`, new)

Introduce a small module with a `JsonFilter` struct that can be flattened into any command's `Args`. The stored value is the compiled `Filter` itself — no wrapper struct, no `Arc`, no `source()` accessor. `jaq_core::Filter` is already `Clone`, so cloning `JsonFilter` is cheap and lets it cross async boundaries.

```rust path=null start=null
use clap::Args;
use jaq_core::{
    data::JustLut,
    load::{Arena, File, Loader},
    Compiler, Filter, Native,
};
use jaq_json::Val;

/// A compiled jaq filter parameterized by `jaq_json::Val`. This is the type
/// produced by `parse_jq_filter` and stored on `JsonFilter`.
pub type JqFilter = Filter<Native<JustLut<Val>>>;

/// CLI argument bundle that parses a `--jq <FILTER>` flag into a pre-compiled
/// jaq filter. Embed with `#[command(flatten)]` on any command whose default
/// JSON output path goes through `print_raw_json`.
#[derive(Debug, Clone, Default, Args)]
pub struct JsonFilter {
    /// jq filter applied to the command's JSON response. When set, implies
    /// JSON output and prints the filter result instead of the raw response.
    /// Uses the jaq implementation of jq (see https://github.com/01mf02/jaq).
    /// Top-level scalar outputs are printed as raw text (no JSON quoting);
    /// arrays and objects are pretty-printed as JSON.
    #[arg(long = "jq", value_parser = parse_jq_filter, value_name = "FILTER")]
    pub filter: Option<JqFilter>,
}

fn parse_jq_filter(src: &str) -> Result<JqFilter, String> {
    let arena = Arena::default();
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs::<Val>()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let loader = Loader::new(defs);
    let modules = loader
        .load(&arena, File { path: (), code: src })
        .map_err(|errs| format!("invalid jq filter {src:?}: {errs:?}"))?;
    Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errs| format!("invalid jq filter {src:?}: {errs:?}"))
}
```

Expose it from `crates/warp_cli/src/lib.rs`:

```rust path=null start=null
pub mod json_filter;
```

The exact spellings of the type alias and `funs::<Val>()` call may need small adjustments once we compile against `jaq-core` 3.0 locally (the crate-level example in the `jaq-core` docs is the canonical reference); the overall shape is stable.

### 3. Flatten `JsonFilter` into both commands (`crates/warp_cli/src/task.rs`)

Replace the per-command flag with a single flattened field:

```rust path=null start=null
use crate::json_filter::JsonFilter;

#[derive(Debug, Clone, Args)]
pub struct ListTasksArgs {
    // ... existing fields ...

    #[command(flatten)]
    pub json_filter: JsonFilter,
}

#[derive(Debug, Clone, Args)]
pub struct TaskGetArgs {
    // ... existing fields ...

    #[command(flatten)]
    pub json_filter: JsonFilter,
}
```

Because `JsonFilter` is a single-field `Args` struct with `long = "jq"`, both commands expose `--jq <FILTER>` in help output without needing per-command duplication of the doc comment. Adding `--jq` to a third command later (e.g. `oz run conversation get`) is a single `#[command(flatten)] pub json_filter: JsonFilter,` line.

### 4. Filter plumbing in `print_raw_json` (`app/src/ai/agent_sdk/output.rs`)

Extend `print_raw_json` to accept an optional pre-compiled filter. When `None`, behavior is identical to today. When `Some(filter)`, run the filter against the input value and print each output on its own line.

```rust path=null start=null
use warp_cli::json_filter::JqFilter;

pub fn print_raw_json(
    value: &serde_json::Value,
    jq_filter: Option<&JqFilter>,
) -> anyhow::Result<()> {
    ensure_stdout_blocking();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match jq_filter {
        None => write_json_pretty(value, &mut out)?,
        Some(filter) => run_jq_filter(value, filter, &mut out)?,
    }

    out.flush()?;
    Ok(())
}
```

The `run_jq_filter` helper lives next to `print_raw_json` in the same file. Because the filter is already compiled, it:

1. Converts the input `serde_json::Value` to `jaq_json::Val` with `serde_json::from_value::<Val>(value.clone())`. The `.clone()` is acceptable — responses are bounded by the server's page size and this keeps the API ergonomic.
2. Constructs a `Ctx` over the filter's `lut` (`Ctx::<JustLut<Val>>::new(&filter.lut, Vars::new([]))`, matching the `jaq_core` crate-level example) and calls `filter.id.run((ctx, input)).map(unwrap_valr)`. For each yielded value:
   - Round-trip the `jaq_json::Val` to `serde_json::Value` via `serde_json::to_value(val)?`. The `serde` feature on `jaq-json` provides `Val: Serialize`, so this is a direct conversion.
   - If the resulting `serde_json::Value` is a scalar (`Null`, `Bool`, `Number`, or `String`), write its raw text form followed by `\n`:
     - `Null` → `null`
     - `Bool` → `true` or `false`
     - `Number` → the number's `to_string()`
     - `String` → the unescaped string content (matches `gh --jq` and `jq -r` semantics for scalars)
   - Otherwise (`Array` or `Object`), write via `serde_json::to_writer_pretty(&mut out, &value)?` followed by `\n`. This keeps the existing pretty-printed, `serde_json`-driven formatting for structured output so that `--jq .` on either command's response stays byte-identical to `--output-format json` without `--jq`.
3. On runtime errors, flushes already-emitted output, writes the error to stderr via `eprintln!` (so the CLI's standard log-hint suffix isn't appended to user-authored filter errors), and returns a non-zero `anyhow::Error`.

Note that `run_jq_filter` never recompiles the filter — compilation only happens inside `warp_cli::json_filter::parse_jq_filter` at clap parse time. This is what makes the CLI fail fast on bad filters: an invalid `--jq` exits during `Args::from_env()` and never reaches the spawn path, auth refresh, or HTTP client.

### 5. Call-site wiring (`app/src/ai/agent_sdk/ambient.rs`)

Both call sites currently match on `OutputFormat` and only call `print_raw_json` in the `Json` arm. With `--jq`, the JSON path should be taken regardless of `--output-format`. Treat "JSON-or-jq" as a single branch:

```rust path=null start=null
let jq = args.json_filter.filter.as_ref();

if matches!(output_format, OutputFormat::Json) || jq.is_some() {
    let response = ai_client.list_agent_runs_raw(limit, filter).await?;
    super::output::print_raw_json(&response, jq)?;
} else {
    let tasks = ai_client.list_ambient_agent_tasks(limit, filter).await?;
    Self::print_tasks_table(&tasks);
}
```

The same shape applies to `get_task_status`. `args` must be threaded into both runner methods so `args.json_filter` is reachable — today `list_tasks` only takes `limit` and `filter`. Pass the full `ListTasksArgs`/`TaskGetArgs` (or a small borrowed struct) to the runner.

### 6. Error mapping

Parse failures surface via clap's standard error-reporting path: `parse_jq_filter` returns `Err(String)`, and clap prints `error: invalid value 'xxx' for '--jq <FILTER>': <our message>` to stderr and exits with clap's usage-error code. This happens inside `Args::from_env()` before the command is even dispatched — no auth, no HTTP, no spawn.

Runtime failures propagate as `anyhow::Error` through the existing `spawn_command` error-reporting path; the CLI prints them to stderr and exits non-zero. Their `Display` is the jaq runtime error message.

## Testing and validation

Unit tests on `JsonFilter` and `parse_jq_filter` in `crates/warp_cli/src/json_filter_tests.rs` (new):

- **Invariants 1, 5, 6 (fail-fast):** `parse_jq_filter(".foo")` returns `Ok`; `parse_jq_filter("@")` and `parse_jq_filter("")` return `Err` whose `Display` contains the filter source. This is the regression guard for fail-fast: if this test passes at the `parse_jq_filter` layer, clap's own invocation guarantees failure happens in `Args::from_env()`.
- **Invariants 1, 5 (end-to-end clap):** calling `Args::try_parse_from(["oz", "run", "list", "--jq", "@"])` returns `Err` with `clap::error::ErrorKind::ValueValidation`. Repeat for `oz run get ID --jq @`.

Unit tests on `print_raw_json` in `app/src/ai/agent_sdk/output_tests.rs` (existing file). These use `parse_jq_filter` to produce a `JqFilter` and then call a crate-private variant of `run_jq_filter` that writes to a `Vec<u8>` instead of stdout. Each test maps back to a numbered invariant in `PRODUCT.md`:

- **Invariants 1 and 9 (no regression):** `print_raw_json(value, None)` produces byte-identical output to the pre-change implementation for a representative response payload.
- **Invariants 2, 6, and 9 (identity filter):** `run_jq_filter(value, parse_jq_filter(".").unwrap(), out)` on a top-level object (the shape both endpoints return) produces the same bytes as `print_raw_json(value, None)`.
- **Invariant 4 (scalar unwrapping):** a filter that emits a string (`.runs[0].task_id`) prints the bare string (no surrounding quotes, no JSON escapes); a filter that emits a number (`.runs | length`) prints the bare number; `true`/`false`/`null` print as the literal words.
- **Invariant 4 (non-scalar output):** a filter that emits an object or array (`.runs[0]`, `.page_info`) prints pretty-printed JSON.
- **Invariant 4 (multiple outputs):** a filter that yields multiple values (`.runs[] | .task_id`) produces one raw line per value.
- **Invariant 9 (inner scalars stay JSON-encoded):** `.runs` on a payload with a string-valued `title` produces JSON output where `title` remains quoted. Scalar unwrapping applies only at the top level.
- **Invariant 4 (empty output):** `empty` produces zero bytes of stdout and returns `Ok(())`.
- **Invariant 5 (runtime error, partial output):** a filter that emits a valid value and then errors writes the valid value to `out` before returning `Err`.

Clap parsing tests in `crates/warp_cli/src/task_tests.rs`:

- Parsing `oz run list --jq ".foo"` populates `ListTasksArgs.json_filter.filter` with `Some(_)` (we don't assert deep structural equality on the compiled filter; the `parse_jq_filter`-level tests above already cover the compile happy path).
- Parsing `oz run get ID --jq ".foo"` populates `TaskGetArgs.json_filter.filter` with `Some(_)`.

Runner-level tests in `app/src/ai/agent_sdk/ambient_tests.rs` (existing file):

- With `--jq` set and `OutputFormat::Pretty`, the runner still calls `list_agent_runs_raw`/`get_agent_run_raw` (not the typed variants) and routes through `print_raw_json`. Assert with the existing mockall-generated `AIClient` mock — no new trait methods needed.

Manual validation against staging covering invariants 2, 3, 4, 5, 6, 7, and 8:

- `oz run list --jq '.runs | length'` and `oz run get <id> --jq '.state'` on a run that exists.
- `oz run list --source CLI --jq '.runs[].task_id'` (composition with existing filters, invariant 7).
- `oz run list --jq '.runs[].bogus | .x'` (runtime error, invariant 5).
- `oz run list --jq '@'` (parse error, invariants 5 and 6 — confirm via `-v`/traffic inspection that no HTTP request is made).
- `oz run list --jq empty` (empty output, invariant 4).
- `oz run list --help` and `oz run get --help` to confirm flag documentation per invariant 8.

Presubmit: `./script/presubmit` (fmt, clippy `-D warnings`, test suite). The touched crates are `warp_cli` (minor) and `warp` (app).

## Risks and mitigations

- **Dependency weight:** `jaq-core`, `jaq-std`, and `jaq-json` together pull in `num-bigint`, `indexmap`, `hifijson`, and a few smaller transitives. The binary-size delta is expected to be modest and similar to other CLI features. `warp_cli` gains all three deps since it compiles filters; `app` already transitively carries serde_json and adds the same set. We deliberately skip `jaq-all` and `jaq-fmts` — the only thing `jaq-all` adds on top of these three is multi-format conveniences we don't use.
- **Filter dialect drift:** `jaq` aims to be jq-compatible but is not byte-identical to BSD `jq` in every edge case. Mitigation: the product spec explicitly names `jaq` as the dialect, and the help text says so.
- **Scalar-unwrapping divergence from pure `jq`:** our top-level scalar unwrapping differs from `jq` (which always emits JSON-encoded values) but matches `gh --jq`. Users coming from `jq` may be surprised that top-level strings are unquoted. Mitigation: document this in the help text, and point to `| tojson` as the opt-out.
- **Partial output on runtime error:** we intentionally flush already-produced outputs before surfacing the error (invariant 5). This matches jq's behavior but means a failing filter can still leave valid JSON on stdout. Downstream scripts that parse stdout should continue to check the CLI exit code, which is unchanged. Documented in the help text is not necessary; the product spec captures it.

## Follow-ups

- Apply `--jq` to `oz run conversation get` (and `oz run get --conversation`). Mechanical once this lands — the conversation emitter also goes through JSON.
- Add `--jq` to any other command that already uses `print_raw_json` (currently only the two in this PR).
- TTY-aware colorization of non-scalar output ([cli/cli#7236](https://github.com/cli/cli/pull/7236)), ideally shared with the existing `--output-format json` path.
- `--template` as a Go-template (or handlebars) alternative to `--jq` ([gh formatting docs](https://cli.github.com/manual/gh_help_formatting)), if there's user demand.
