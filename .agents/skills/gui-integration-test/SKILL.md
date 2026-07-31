---
name: gui-integration-test
description: GUI desktop app only. Writes, runs, and debugs Warp integration tests using the custom Builder/TestStep framework in `crates/integration`. Use when adding a new integration test, fixing a failing integration test, wiring a test into the manual runner or nextest suite, or verifying end-to-end UI and terminal behavior in Warp.
---

# Warp Integration Tests

**Scope — GUI desktop app only.** This skill applies to Warp's **GUI** desktop front-end (the `app/` crate on the WarpUI pixel/GPU framework). It does **not** apply to the headless **TUI** front-end (`crates/warp_tui`; cell-grid `TuiElement` library under `crates/warpui_core/src/elements/tui`), which has its own components, tests, and change-verification workflow. For TUI work, see the `tui-ui-guidelines`, `tui-testing`, and `tui-verify-change` skills instead.

Use this skill for Rust integration tests in Warp's custom framework under `crates/integration/`.

These are not ordinary unit tests. They boot a real Warp app instance, give it an isolated test home directory, drive it with synthetic UI and terminal events, and poll assertions until success or timeout.

## When an integration test is the right tool

Integration tests are the most expensive tests in the repo. They boot the app, they are orders of magnitude slower than a unit test, and they are the most likely to go flaky. Write one when the risk you are covering genuinely lives *between* components:

- Real terminal/PTY and shell-integration behavior, especially across bash, zsh, and fish.
- Wiring and configuration: settings, keybindings, and user preferences actually taking effect end to end.
- Cross-component flows a unit test cannot see — command palette into editor into terminal, focus and pane management, tab/window lifecycle.
- Input and rendering paths that only exist once a real app (and sometimes a real display) is running.
- Regressions for user-visible bugs that escaped unit tests.

Do **not** reach for an integration test when:

- The logic is deterministic and reachable in-process. Push it into a unit test (`rust-unit-tests`); it will run in milliseconds and point straight at the failure.
- You are re-covering branch logic a unit test already covers. This harness is for the seams between components, not for re-testing conditionals at full app-boot cost.
- You only need TUI rendering. That is `tui-testing` — this harness does not drive the TUI at all.
- What you actually want is a screenshot or a manual look. Use `computer_use` or the `gui-integration-test-video` skill instead of attaching weak assertions to a full app boot.

If a behavior is hard to reach from a unit test *only* because of how the code is structured, prefer fixing the structure over writing a slow test around it.

## Framework map

The core pieces are:

- `crates/integration/src/bin/integration.rs`
  - Manual integration test runner binary.
  - Registers test names to `Builder` factories.
  - Runs exactly one named test per invocation.
- `crates/integration/tests/common/mod.rs`
  - The outer Rust test harness used by `cargo test` and `cargo nextest`.
  - Shells out to the integration binary.
  - Forwards a limited set of env vars (`PATH`, `RUST_*`, `WARP_*`, `WARPUI_*`, `WGPU_*`, display-related vars).
  - Re-runs tests up to 10 times when the integration binary exits with the special rerun code.
- `crates/integration/src/test.rs`
  - Module hub for integration tests.
  - Add new test modules here and `pub use` their functions so the runner can see them.
- `crates/integration/tests/integration/ui_tests.rs`
  - List of UI-oriented integration tests that nextest should run.
- `crates/integration/tests/integration/shell_integration_tests.rs`
  - List of tests that must run against every shell or a specific shell matrix.
- `crates/integration/src/builder.rs`
  - Warp-specific wrapper around the lower-level WarpUI integration builder.
  - Sets default timeout, hermetic home directory, shell rc files, user prefs, and real-display mode when requested.
- `crates/warpui_core/src/integration/driver.rs`
  - Executes steps, handles retries, precondition reruns, screenshots, video capture, artifact export, and `on_finish`.
- `crates/warpui_core/src/integration/step.rs`
  - Defines `TestStep`, input/event APIs, assertion polling, step-to-step data passing, and screenshot/recording hooks.
- `app/src/integration_testing/`
  - High-level helpers and assertions for common Warp behaviors.
  - Prefer these helpers over raw low-level event plumbing whenever they fit.

## How the framework actually runs a test

1. A Rust test from `crates/integration/tests/integration/*.rs` calls `run_integration_test("test_name")`.
2. That harness launches the `integration` binary with the test name.
3. The binary in `crates/integration/src/bin/integration.rs` looks up the name in `register_tests()`, builds the `Builder`, and turns it into a `TestDriver`.
4. `Builder::build(...)` creates an isolated temp directory, points `HOME` at it, writes minimal rc files, and initializes file-backed user preferences.
5. The driver runs each `TestStep` in order:
   - setup callbacks
   - synthetic events
   - actions
   - assertion polling until success or timeout
6. If an assertion returns `PreconditionFailed`, the binary exits with the rerun code and the outer harness retries the whole test.
7. On success, failure, or cancellation, the driver can run `on_finish` and export artifacts/runtime tags.

This means integration tests should be written for a hermetic environment. Do not rely on the developer's real shell dotfiles, home directory contents, or persisted Warp settings.

## Where to put a new test

Add the actual test function in a module under `crates/integration/src/test/`.

Use these heuristics:

- Put the test in an existing module when it matches that feature area.
- Create a new module when the feature does not fit an existing one cleanly.
- Add the test to `crates/integration/tests/integration/ui_tests.rs` if it is primarily a UI/app behavior test.
- Add the test to `crates/integration/tests/integration/shell_integration_tests.rs` if it needs to run against every shell, or depends on a specific shell/set of shells.

Being present in `crates/integration/src/test/*.rs` is not enough. For a test to run under `cargo nextest`, it also needs to be listed in one of the macro files in `crates/integration/tests/integration/`.

## Authoring checklist for a new test

When adding a new integration test, do all of the following:

1. Implement `pub fn test_name() -> Builder` in a module under `crates/integration/src/test/`.
2. Add the module to `crates/integration/src/test.rs`.
3. `pub use` the new module's exports from `crates/integration/src/test.rs`.
4. Add `register_test!(test_name);` in `crates/integration/src/bin/integration.rs`.
5. Add `test_name` to either:
   - `crates/integration/tests/integration/ui_tests.rs`, or
   - `crates/integration/tests/integration/shell_integration_tests.rs`
6. Default to making the test run in CI once it is added to one of those macro lists. Only mark it `#[ignore]` when the task explicitly calls for manual-only coverage or there is a concrete, documented reason it cannot run reliably in CI.
7. Run the test manually first, then through nextest once it is stable enough for the suite you chose.

## Writing the test body

The normal shape is:

```rust
use crate::Builder;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks,
    execute_command_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
    util::ExpectedExitStatus,
};

pub fn test_example() -> Builder {
    Builder::new()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo hello".to_string(),
            ExpectedExitStatus::Success,
            "hello".to_string(),
        ))
        .with_step(
            new_step_with_default_assertions("Assert some UI state")
                .add_named_assertion("specific assertion name", |app, window_id| {
                    // inspect app state and return AssertionOutcome
                    warpui::integration::AssertionOutcome::Success
                }),
        )
}
```

Prefer a small number of focused steps with descriptive names over a huge monolithic test.

## Builder guidance

### `Builder::new()`

Start here almost every time.

Warp's wrapper automatically gives you:

- a per-test root directory
- isolated `HOME`
- generated rc files for Bash, Zsh, and Fish
- file-backed user preferences
- a default 2-minute hard timeout
- real-display support if `WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS` is present

### `with_setup(...)`

Use this for filesystem or environment setup before the app runs.

Common patterns:

- `utils.set_env("NAME", Some(value))`
- creating files under `utils.test_dir()`
- writing fixture config files

Prefer this over reaching into the real filesystem.

### `with_user_defaults(...)`

Use this to set persisted Warp preferences before the test starts.

This is the right tool for settings backed by user preferences rather than environment variables.

### `set_should_run_test(...)`

Use this to gate tests on shell/platform/runtime capabilities when the test genuinely cannot run everywhere.

### `with_on_finish(...)`

Use this for final verification or artifact inspection that should happen after all steps complete, such as checking that screenshots or recordings were written.

### `with_real_display()`

Use this explicitly when the test needs a real display for frame capture or visual workflows. Video/screenshot tests should normally be manual or ignored in CI unless there is a stable real-display path.

## `TestStep` guidance

`TestStep` is the unit of execution. Each step can have:

- setup callbacks
- input events
- actions
- assertions
- a timeout
- retry count
- failure handling

### Start from helper constructors

Prefer:

- `wait_until_bootstrapped_single_pane_for_tab(0)`
- `new_step_with_default_assertions("...")`
- `new_step_with_default_assertions_for_pane("...", tab, pane)`

The default step helpers already assert:

- no pending model events
- no block executing

These are good baseline invariants for most UI interactions.

### Prefer helper APIs over raw event plumbing

Use high-level helpers from `app/src/integration_testing/` whenever possible:

- terminal command execution helpers
- block list helpers
- command palette helpers
- navigation helpers
- settings helpers
- workflow/file tree/notebook helpers

Drop to raw `with_event(...)`, `with_event_fn(...)`, or saved-position mouse events only when there is no suitable helper.

### Use named assertions

Prefer `add_named_assertion(...)` over unnamed assertions. Named assertions make failure output and runtime tags much easier to interpret.

### Use polling assertions instead of sleeps

Assertions are polled until success or timeout. Lean on that model instead of hardcoding sleeps.

Good pattern:

- trigger an event or action
- assert on the eventual UI/model state

Avoid brittle timing assumptions.

### Use step data when one step computes something for the next

If a later step needs data from an earlier one, use:

- `add_named_assertion_with_data_from_prior_step(...)`
- `StepDataMap`

This is useful for saving measured positions, counts, IDs, or other values from prior frames.

### Use retries sparingly

`set_retries(...)` can help for a legitimately retryable step, but do not use it to hide deterministic failures. Prefer making the step more robust first.

### Use `PreconditionFailed` for genuinely environmental flakes

If the environment reaches a state where the rest of the test is invalid, return `AssertionOutcome::PreconditionFailed(...)` instead of failing hard. The outer harness can rerun the entire test up to 10 times. The existing bootstrap helper is a good model for this.

Use this deliberately. The rerun mechanism exists for conditions the test genuinely cannot control, such as bootstrap racing or shell startup timing. It is not a way to turn an intermittently failing test green. A real bug that reproduces one run in five will pass under rerun and ship to users.

Before reaching for `PreconditionFailed`, confirm the failure is actually environmental by looking at the failure rate and the failure mode:

```bash
for i in {0..50}; do
  RUST_BACKTRACE=full cargo run -p integration --bin integration -- test_name || break
done
```

If the same assertion fails in different ways, or fails while the environment looks fine, it is a product or test bug. Fix it rather than absorbing it into a rerun.

## Designing a good integration test

### Keep the scope as small as the behavior allows

The scope of a test follows the scope of what you boot and drive, so cover one user journey per test and let separate tests cover separate journeys. When a flow is long, prefer several shorter tests that each verify one hop over a single test that walks the whole path. A long test is slower, harder to diagnose, and fails for many unrelated reasons.

### Assert on behavior a user could observe

The framework can see internal state, which makes it tempting to assert on it. Anchor each test on the user-observable outcome:

- output visible in the terminal
- focus moved where expected
- UI element opened or closed
- selection changed
- settings applied

Internal state assertions are still useful, but they should support the visible behavior rather than replace it. Assertions that mirror internal call sequences become change detectors: they break on every refactor and catch no bugs.

### Make the failure diagnosable by someone else

An integration test spans processes, so a stack trace tells the next engineer almost nothing. The failure output has to carry the context instead:

- Name every assertion with what it expects, not what it touches: `"terminal shows command output"`, not `"check blocks"`.
- Give steps descriptive names; they are the breadcrumb trail through the run.
- Put expected vs. actual into the assertion output rather than returning a bare failure.

Assume the person reading the failure has never seen this test and does not own the code it covers.

### Keep the test hermetic

`Builder::new()` already gives you an isolated `HOME`, generated rc files, and file-backed preferences. Preserve that. Set up everything you depend on inside the test via `with_setup(...)` and `with_user_defaults(...)`, and never rely on the developer's dotfiles, real settings, network access, or state left behind by another test. A test that assumes a resource is already in the right state will fail for the wrong reason on someone else's machine or in CI.

### Own the test

Larger tests span components, so ownership is ambiguous by default and unowned tests rot. If you add one, you are the person who fixes it when it breaks. Put it in the module matching the feature area it covers so the next person can find the right owner.

## Common test-writing patterns

### 1. Wait for bootstrap first

For most terminal-facing tests, the first real step should be:

- `wait_until_bootstrapped_single_pane_for_tab(0)`

Do not start asserting on terminal UI before bootstrap completes.

### 2. Clear the bootstrapped blocks if block indices matter

If the test relies on saved positions like `block_index:0`, clear the block list after bootstrap:

- `clear_blocklist_to_remove_bootstrapped_blocks()`

Otherwise the first user-generated block index depends on bootstrap output and the active shell.

### 3. Use helper command runners

Prefer helpers like:

- `execute_command_for_single_terminal_in_tab(...)`
- `execute_echo(...)`
- `execute_echo_str(...)`
- `execute_long_running_command(...)`

These helpers already handle a lot of correctness and output validation.

## Running tests

### Run one test directly through the integration binary

Use this first while authoring:

```bash
cargo run -p integration --bin integration -- test_name
```

This is the fastest way to iterate on a specific test because it bypasses the outer Rust test wrapper and runs the named test directly.

### Run one test through nextest

Once it is wired into one of the `tests/integration/*.rs` macro lists, run it with nextest:

```bash
cargo nextest run --no-fail-fast --workspace test_name
```

### Run with a real display when needed

For screenshot/video or other real-display flows:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 cargo run -p integration --bin integration -- test_name
```

Or with nextest:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 cargo nextest run --no-fail-fast --workspace test_name
```

## Debugging and investigation

### Get a backtrace on failures

```bash
RUST_BACKTRACE=1 cargo run -p integration --bin integration -- test_name
```

### Pause on failure

This is useful when running locally and you want to inspect the failed UI state:

```bash
WARPUI_PAUSE_INTEGRATION_TEST_ON_FAILURE=1 cargo run -p integration --bin integration -- test_name
```

### Pause after every step

Useful for understanding exactly what the test is doing:

```bash
WARPUI_PAUSE_INTEGRATION_TEST_AT_EVERY_STEP=1 cargo run -p integration --bin integration -- test_name
```

### Video and screenshots

If the task is specifically about recording a test, collecting screenshots, or validating overlay/video artifacts, also use the `gui-integration-test-video` skill (located at `.warp/skills/gui-integration-test-video/SKILL.md`).

### Environment variable gotcha

`utils.set_env(...)` affects runtime environment lookups such as `std::env::var(...)`.

It does not affect compile-time lookups like `option_env!(...)`. If the product code uses `option_env!`, changing the env var inside the test will not change that behavior without rebuilding.

## Verification checklist

Before considering a new integration test done, verify all of the following:

- The test function lives under `crates/integration/src/test/`.
- The module is added and re-exported in `crates/integration/src/test.rs`.
- The test is registered in `crates/integration/src/bin/integration.rs`.
- The test is listed in the correct nextest macro file and will run in CI by default, unless it was explicitly made manual-only with a documented reason.
- The test passes when run directly through the integration binary.
- The test passes through nextest if it is meant to be part of the automated suite.
- An integration test is the right level for this behavior; it is not re-covering logic a unit test could reach.
- The assertions check the intended user-visible behavior.
- Every assertion is named, and the failure output would be actionable to someone who has never seen the test.
- The test does not depend on the developer's real home directory, shell config, or machine state.
- If the test uses screenshots/video, the produced artifacts were actually inspected rather than only assuming they exist.

## Anti-patterns to avoid

- Writing a test only in `src/test/*.rs` and forgetting the nextest macro list.
- Asserting on bootstrap-sensitive block indices without clearing the bootstrapped blocks first.
- Using raw events everywhere when a helper already exists.
- Adding sleeps instead of assertion polling.
- Making the test depend on personal dotfiles, real settings, or non-hermetic filesystem state.
- Using retries or `PreconditionFailed` to paper over a deterministic bug.
- Re-testing branch logic that a unit test already covers, at full app-boot cost.
- Asserting on internal call sequences instead of user-visible behavior.
- Unnamed assertions that produce failure output nobody can act on.
- One long test that walks an entire user journey instead of several focused ones.
- Leaving a real-display/manual test enabled in CI without a stable path.

## Good workflow for agents

When asked to add or fix an integration test:

1. Find the closest existing integration test module for the feature.
2. Reuse helper assertions and step constructors before inventing new low-level plumbing.
3. Register the test in all required places, not just the implementation file.
4. Run the test manually first.
5. If it belongs in automation, run it with nextest too.
6. If the test exercises visual behavior, verify the resulting UI behavior or artifacts directly.
