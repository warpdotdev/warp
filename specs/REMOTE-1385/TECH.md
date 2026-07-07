# Harness Auth Preflight + Runtime Failure Detection — Tech Spec

## Context
See `PRODUCT.md` for user-facing behavior. This document covers the implementation.

### Current state
Third-party harnesses implement the `ThirdPartyHarness` trait (`app/src/ai/agent_sdk/driver/harness/mod.rs:122`). The lifecycle for a third-party harness run is:

1. `ThirdPartyHarness::validate()` — checks CLI is on PATH.
2. `ThirdPartyHarness::build_runner()` — writes config files (auth, trust, system prompt, MCP) and returns a `HarnessRunner`.
3. `HarnessRunner::start()` — creates the external conversation on the server and launches the main CLI command.

These steps are orchestrated in `AgentDriver::run_internal` (`driver.rs:1608`), which calls `setup_harness` → `prepare_harness` → `run_preflight_checks` → `run_harness`.

Driver errors are classified in `error_classification.rs` into `(AgentTaskState, TaskStatusUpdate)` pairs and reported to the server via `report_driver_error`. The `PlatformErrorCode` enum already has `AUTHENTICATION_REQUIRED`.

### Relevant files
- `app/src/ai/agent_sdk/driver/harness/mod.rs` — `ThirdPartyHarness` trait, `HarnessRunner` trait, `preflight_commands_for`
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs`, `harness/codex.rs`, `harness/gemini.rs` — per-harness implementations
- `app/src/ai/agent_sdk/driver.rs` — `AgentDriver`, `AgentDriverError` enum, `run_internal`, `prepare_harness`, `run_harness`, `run_preflight_checks`
- `app/src/ai/agent_sdk/driver/harness_output_monitor.rs` — runtime scanner (new)
- `app/src/ai/agent_sdk/driver/error_classification.rs` — maps `AgentDriverError` → `(AgentTaskState, TaskStatusUpdate)`
- `app/src/ai/agent_sdk/driver/terminal.rs` — `TerminalDriver::execute_command`, `block_snapshot`, `find_first_match_in_block_output` (new)
- `app/src/terminal/model/find.rs` — `RegexDFAs` machinery reused by the scanner
- `app/src/terminal/model/blockgrid.rs` — `BlockGrid::find(&RegexDFAs)` returns matches over cell storage directly

## Proposed changes

### 1. Trait surface changes on `ThirdPartyHarness`

Drop the billing preflight method and add a runtime-error-patterns method:

```rust path=null start=null
// Removed:
// fn billing_check_command(&self) -> Option<String> { None }

// Added:
/// Substrings to scan for in the running harness block's output. A hit
/// indicates the harness can't make a successful API request.
fn runtime_error_patterns(&self) -> &'static [&'static str] {
    &[]
}
```

Per-harness:
- **Claude Code** and **Codex**: drop `billing_check_command`, add `runtime_error_patterns` returning an empty slice with a `TODO(REMOTE-1385)` to fill in validated needles.
- **Gemini**: inherits default `None` / empty.

`preflight_commands_for(harness)` in `harness/mod.rs` now only pushes `auth_check_command()`. The viewer's "setup commands" grouping continues to recognize preflight blocks by exact string equality.

### 2. New error variant + classification

In `driver.rs`'s `AgentDriverError`:

```rust path=null start=null
// Removed: HarnessAuthFailureKind enum and the `kind` field on
// HarnessAuthCheckFailed.

#[error("Harness '{harness}' auth preflight failed")]
HarnessAuthCheckFailed {
    harness: String,
    detail: String,
},

// Added:
#[error("Harness '{harness}' reported a runtime failure matching '{pattern}'")]
HarnessRuntimeFailureDetected {
    harness: String,
    pattern: String,
    excerpt: String,
},
```

In `error_classification.rs`, both variants map to `(AgentTaskState::Failed, PlatformErrorCode::AuthenticationRequired)` with distinct user-visible messages. The runtime failure message surfaces both the matched needle and the excerpt from the harness output.

### 3. New `find_first_match_in_block_output` helper on `TerminalDriver`

Add a sibling to `block_snapshot` in `driver/terminal.rs` that runs the existing DFA machinery against the block's output grid directly:

```rust path=null start=null
pub struct BlockOutputMatch {
    pub matched_text: String,
    pub excerpt: String,
}

pub fn find_first_match_in_block_output(
    &self,
    block_id: &BlockId,
    dfas: &RegexDFAs,
    ctx: &AppContext,
) -> Option<BlockOutputMatch>;
```

Internally:
- Acquire the terminal model lock (same scope as `block_snapshot`).
- Look up the block by ID; return `None` if missing.
- Run `block.output_grid().find(dfas).next()` to get the first `Match = RangeInclusive<Point>`.
- Use the grid handler's `bounds_to_string` to extract the matched substring (used to identify the originating needle) and the full row(s) the match touches (used as the user-visible excerpt). Both with `include_esc_sequences=false` and `RespectObfuscatedSecrets::Yes` so the failure message never leaks credentials.

No new grid-side code is required — `BlockGrid::find` is the same path the find feature uses on every keystroke.

### 4. New `harness_output_monitor` module

File: `app/src/ai/agent_sdk/driver/harness_output_monitor.rs`.

```rust path=null start=null
pub(crate) struct DetectedHarnessError {
    pub pattern: String,
    pub excerpt: String,
}

const SCAN_SCHEDULE: &[(Duration, Duration)] = &[
    (Duration::from_secs(5), Duration::from_secs(30)),
    (Duration::from_secs(15), Duration::from_secs(60)),
];

pub(crate) fn build_dfas(patterns: &[&'static str]) -> Option<RegexDFAs>;
pub(crate) fn pattern_for_match(matched_text: &str, patterns: &[&'static str]) -> Option<&'static str>;
pub(crate) async fn watch_block_for_errors(
    block_id: BlockId,
    patterns: &'static [&'static str],
    foreground: &ModelSpawner<AgentDriver>,
) -> Option<DetectedHarnessError>;
```

Implementation notes:
- Resolve immediately with `None` when `patterns.is_empty()` so Gemini/Oz pay zero runtime cost.
- Build the combined DFA once at scanner entry via `RegexDFAs::new_many` (with regex-escaped needles + case-insensitive matching). Wrap in `Arc` so each tick can clone cheaply.
- Use `warpui::r#async::Timer::after` for ticks, matching the existing `run_harness` periodic-save pattern.
- Each tick spawns onto `foreground` to call `TerminalDriver::find_first_match_in_block_output(&dfas)`. On `Some`, the scanner runs the **stall confirmation loop** (see below) before mapping `matched_text` → originating needle via `pattern_for_match`, capping the excerpt to ~240 chars, and resolving the future with `DetectedHarnessError`.
- The DFA scans cell storage from the start each tick, so no `last_scanned_len` cursor is needed — the regex_automata cache memoizes transitions.

#### Stall confirmation loop
After a pattern hit, the scanner runs `confirm_stall` before reporting a failure. This guards against false positives when a harness prints a transient API error and then automatically retries.

```rust path=null start=null
/// Gap between consecutive plaintext snapshots while confirming a hit.
const STALL_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Total budget for the stall-confirmation loop.
const STALL_CONFIRMATION_BUDGET: Duration = Duration::from_secs(60);

/// Pure equality check kept out of the async body for easy unit tests.
fn outputs_stalled(before: Option<&str>, after: Option<&str>) -> bool {
    matches!((before, after), (Some(a), Some(b)) if a == b)
}

/// Polls the block plaintext every `STALL_POLL_INTERVAL` for up to
/// `STALL_CONFIRMATION_BUDGET`. Returns `(Some(hit), elapsed)` once two
/// consecutive snapshots are byte-identical AND a pattern is still
/// present after the second snapshot; otherwise `(None, elapsed)`.
async fn confirm_stall(
    block_id: &BlockId,
    dfas: &Arc<RegexDFAs>,
    foreground: &ModelSpawner<AgentDriver>,
) -> (Option<BlockOutputMatch>, Duration);
```

Design notes:
- **Why plaintext equality over `dirty_cells_range`?** `GridHandler::dirty_cells_range` is per-pty-pass-scoped — it resets to the cursor point on every `on_finish_byte_processing` call, so it's not a usable "has anything happened lately" oracle. Sampling the visible plaintext twice and comparing them gives us the same information directly, and works for spinner-only retries (the spinner cell changes between samples → the snapshots differ → the loop keeps running).
- **Why count confirmation time against the outer scan-schedule budget?** A flaky harness that keeps retripping the same pattern would otherwise extend the watch window indefinitely — each candidate hit would buy another 60 seconds. Adding `confirmation_elapsed` back to the outer `elapsed` counter caps the total observation period at the outer 90-second budget plus whatever final confirmation was in flight when the schedule exhausted.
- A new helper `TerminalDriver::block_output_plaintext(block_id, ctx) -> Option<String>` is added alongside `find_first_match_in_block_output`. It reads `block.output_grid().contents_to_string(false, None)` under the same model lock and obfuscates secrets identically.

### 5. Wire the scanner into `run_harness`

`run_harness` gains two new parameters and a fourth select arm:

```rust path=null start=null
async fn run_harness(
    runner: Arc<dyn HarnessRunner>,
    harness_name: String,
    runtime_error_patterns: &'static [&'static str],
    foreground: &ModelSpawner<Self>,
    harness_exit_rx: oneshot::Receiver<()>,
) -> Result<(), AgentDriverError> {
    let command_handle = runner.start(foreground).await?;
    let block_id = command_handle.block_id().clone();
    let mut command_handle = command_handle.fuse();
    let mut harness_exit_rx = harness_exit_rx.fuse();

    let scanner_fut = harness_output_monitor::watch_block_for_errors(
        block_id, runtime_error_patterns, foreground,
    )
    .fuse();
    futures::pin_mut!(scanner_fut);

    let mut detected_runtime_failure: Option<harness_output_monitor::DetectedHarnessError> = None;

    let command_result = loop {
        futures::select! {
            exit_code = command_handle => break exit_code,
            _ = warpui::r#async::Timer::after(HARNESS_SAVE_INTERVAL).fuse() => {
                /* existing periodic save */
            }
            _ = harness_exit_rx => {
                /* existing graceful-exit branch */
            }
            detected = scanner_fut => {
                if let Some(error) = detected {
                    // Ask the harness to exit gracefully so cleanup runs;
                    // record the failure and let the command future complete.
                    let _ = runner.exit(foreground).await;
                    detected_runtime_failure = Some(error);
                }
                // When the schedule exhausts without a hit, the `Fuse`
                // wrapper makes this branch stay Pending forever.
            }
        }
    };

    /* existing final save + cleanup */

    if let Some(error) = detected_runtime_failure {
        return Err(AgentDriverError::HarnessRuntimeFailureDetected {
            harness: harness_name,
            pattern: error.pattern,
            excerpt: error.excerpt,
        });
    }

    /* existing exit-code mapping */
}
```

Notes:
- After `scanner_fut` resolves to `None`, the `Fuse` wrapper makes the branch stay `Pending` forever, so we don't busy-loop.
- A detected runtime failure takes precedence over the harness's own exit code: surface the actionable detail rather than a generic "exit code N".
- The cleanup-disposition computation also factors `detected_runtime_failure` so we always drop resumption state on a runtime failure (a failed run shouldn't be silently resumable).

### 6. Caller update in `run_internal`

The third-party branch in `run_internal` already has `harness.as_ref()` in hand. Plumb the harness name and runtime patterns through both the credentials-refresh branch and the no-refresh branch:

```rust path=null start=null
let harness_name = harness.cli_agent().command_prefix().to_owned();
let runtime_error_patterns = harness.runtime_error_patterns();
Self::run_harness(runner, harness_name, runtime_error_patterns, &foreground, harness_exit_rx).await
```

### 7. Server-side changes

None. Both failure modes map to the existing `AUTHENTICATION_REQUIRED` `PlatformErrorCode`, differentiated only by the human-readable `statusMessage`. The server already handles `AUTHENTICATION_REQUIRED` in task state transitions.

## Testing and validation

### Unit tests
- **`harness_output_monitor_tests.rs`** (new):
  - `build_dfas` returns `None` for empty patterns and `Some` otherwise; regex metacharacters in needles are escaped.
  - `pattern_for_match` maps a case-different matched text back to the originating `'static` needle; returns `None` when no needle matches; picks the first matching needle when multiple lowercase-equal candidates exist.
- **`error_classification_tests.rs`**:
  - `HarnessAuthCheckFailed` maps to `(Failed, AuthenticationRequired)` with the auth-failure message. (Existing `harness_auth_test_request_failed_*` test removed since the `TestRequestFailed` variant is gone.)
  - `HarnessRuntimeFailureDetected` maps to `(Failed, AuthenticationRequired)`; the user-visible message contains both the matched `pattern` and the `excerpt`.
- **`harness/mod_tests.rs`**:
  - Billing tests removed (`*_billing_check_command`, billing assertions in `preflight_commands_for_*`).
  - `preflight_commands_for_claude_returns_auth_only` / `preflight_commands_for_codex_returns_auth_only` pin the auth check string to the trait impl.
  - `runtime_error_patterns()` is callable on Claude/Codex (slice may be empty); Gemini returns an empty slice.

### Manual validation
- Claude Code with a valid API key → auth check passes, harness runs, scanner finds no match in 90 seconds → run completes normally.
- Claude Code with an invalid API key → auth check fails, task is marked FAILED with the auth-failure message.
- Claude Code with a valid key but exhausted credits → auth check passes, harness emits a credit-balance error to its block, scanner detects it, task is marked FAILED with the runtime-failure message including the matched pattern + excerpt.
- Same matrix for Codex.
- Gemini → auth check skipped, scanner is a no-op, harness proceeds and behaves as before.

## Parallelization
Single-agent task. The change is tightly coupled (trait method, classifier, driver loop, scanner module, new helper, tests) and lives in one crate; parallel agents would step on each other.
