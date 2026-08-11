---
name: rust-unit-tests
description: Write, improve, and run Rust unit tests in the warp Rust codebase.
---

# Rust Unit Tests in warp

## Scope
- This skill focuses on crate-level unit tests.
- Favor incremental, well-scoped tests that exercise a single function or behavior per case.
- For *how* to structure and run tests, read on. For *whether* a change needs a test and at what level, start with "What to unit test".

## What to unit test

Default to a unit test when the logic is deterministic and reachable without booting the app:

- Parsing, encoding, escaping, and any pure data transformation.
- Domain logic and state machines (block lifecycle, selection, ranges, diffing).
- Boundary and edge cases: empty input, single element, off-by-one at range ends, min/max, invalid UTF-8, zero-width and wide grapheme clusters.
- Error paths and fallbacks, not just the happy path.
- **Bug fixes, where the bug is reachable at this level.** A reproducible bug usually means a case was missing from the suite, so add the failing test first and then fix it. Not every bug is unit-testable — a rendering artifact, a PTY timing race, or a crash that needs a real window will not reduce to one. Cover those at the level where they actually reproduce, and don't reshape the code just to force a unit test.

Prefer unit tests for anything that fits: they are fast, deterministic, and point straight at the failure.

## When a unit test is the wrong level

Be honest about this codebase. Warp is a terminal emulator with a GPU renderer, PTY and shell integration, and IPC. The common "80% unit tests" heuristic assumes a business-logic-heavy system where units exchange messages and transform data. Large parts of this repo are not that, and forcing them under a unit test usually means mocking away the only thing that could actually break.

Escalate to a higher level when any of these hold:

- The behavior needs a real PTY, shell, window, or display to exist at all.
- What can break is the *wiring* between components — a setting taking effect, a keybinding dispatching, focus moving across panes — rather than logic inside any one of them.
- Reproducing it requires real IO, spawned processes, or cross-process timing.
- The only way to make it unit-testable is to add a trait and an indirection layer that exist solely for the test.
- You find yourself stubbing so much that the test no longer exercises anything real.

Where to go instead:

- `gui-integration-test` — GUI end-to-end behavior, terminal and shell integration, settings and keybinding wiring.
- `tui-testing` — TUI element and screen rendering.
- `gui-integration-test-video` or `computer_use` — when the real question is visual and someone needs to look at it.

Before escalating, try splitting the problem. Most "untestable" code is a thin shell of IO wrapped around logic that tests fine once separated: extract the decision-making into a pure function, unit test that, and let an integration test cover the thin shell that remains. That is usually cheaper than either a heavily stubbed unit test or a full app-boot test.

That said, weigh the indirection on its own merits. Code that is hard to test is *sometimes* genuinely badly designed — if extracting the logic would make the code clearer regardless of testing, do it. If the seam would exist only to satisfy a test, don't.

## When NOT to write a unit test

Tests cost real maintenance, and a bad test costs more than no test. Skip or delete these:

- **Change-detector tests.** A test that restates the implementation — inject two collaborators, assert they were called in order — fails on every refactor and catches no defects. It has negative value. Rewrite it as a state assertion or delete it.
- **Trivial code with no logic.** Getters, `From`/`Into` passthroughs, `Default` impls, plain struct construction. There is nothing that can break independently.
- **Code you don't own.** Don't test the standard library, `tokio`, or `wgpu`. Test *your usage* of them.
- **Redundant tests.** If a case is already covered, a near-identical test adds maintenance cost and no signal. Prune tests as ruthlessly as production code.

Don't chase a coverage number. Coverage says a line executed, not that anything was verified, and a target reliably turns into a ceiling.

## Where unit tests live
- Put unit tests in separate files named `${filename}_tests.rs` or `mod_test.rs`.
- Include the test module at the end of the corresponding source file:

```rust
#[cfg(test)]
#[path = "filename_tests.rs"] // or "mod_test.rs"
mod tests;
```

## Writing good tests

Aim for a test you never touch again unless the behavior changes. Refactors, new features, and bug fixes should not require editing existing tests; only a deliberate behavior change should. If a refactor breaks your tests, that is usually a defect in the tests.

### Test behavior through the public API

Exercise the unit the way its callers do. Reaching into private state makes the test fail on refactors no caller would notice. If a helper type exists only to serve one or two callers, test it through them rather than directly.

### Assert on state, not on interactions

Assert what the system *is* after the action, not which functions it called to get there.

```rust
// Brittle: still passes if the entry is dropped right after insertion, and
// fails on an equivalent refactor that calls a different internal method.
assert!(recorder.saw_call_to_insert(id));

// Better: asserts the outcome the caller actually cares about.
store.insert(id, entry.clone());
assert_eq!(store.get(id), Some(&entry));
```

### One behavior per test, named after that behavior

The test name is often the only thing visible in a failure report, so make it a sentence about behavior rather than about the method:

```rust
#[test]
fn parses_utf8_sequence_when_valid() { /* ... */ }

#[test]
fn returns_replacement_char_for_invalid_utf8() { /* ... */ }
```

If the name needs an "and", you are testing two behaviors — split it. Structure the body as arrange / act / assert, separated by blank lines.

### Keep the test complete and concise

Everything a reader needs to understand the result belongs in the test body; everything irrelevant belongs out of it. Prefer a builder or helper constructor that takes only the fields the test cares about over one large shared fixture. If a test asserts on a specific value, set that value in the test rather than inheriting it from shared setup.

### Prefer duplication over indirection

Test code has no tests of its own, so it has to be obviously correct on inspection. Some repetition is a fair price for a test that reads top to bottom. Extract a helper when it removes noise, not merely to remove repetition.

### No logic in tests

No conditionals, loops, arithmetic, or string concatenation to compute an expected value. Write expected values literally — computing them re-implements the code under test and can reproduce the same bug in the assertion.

### Make failures diagnosable

- Prefer `assert_eq!`/`assert_ne!` over `assert!` for readable diffs.
- Add a message when the values alone aren't self-explanatory: `assert_eq!(got, want, "cursor should clamp to line end for {input:?}")`.
- Use `#[should_panic]` only when panicking is intended API, and pin the message with `expected = "..."`.

### Repo-specific

- Minimize global state; inject dependencies via traits/constructors so logic is testable without heavy mocking.
- When adding an enum variant or expanding behavior, prefer exhaustive matches in the code under test and mirror the new cases in tests.
- Be mindful of terminal model locking: avoid patterns that acquire multiple `model.lock()` calls in the same call stack from tests, and prefer passing an already-locked reference down.

## Test doubles: prefer real code, then fakes

Work down this list and stop at the first option that is fast and deterministic:

1. **The real implementation.** The default for value types and pure logic. Running real collaborator code is what makes a test meaningful, and a failure caused by a real dependency's bug is a true positive worth having.
2. **A fake the repo already provides.** These are maintained alongside the real thing, so they don't drift the way hand-rolled doubles do: `warpui::App::test`, `VirtualFS`, `TerminalModel::mock(..)`, `TestBlockListBuilder`/`TestBlockBuilder`, `Appearance::mock()`, and `FeatureFlag::X.override_enabled(..)`. See "Common helpers to use" below for usage.
3. **A stubbed return value**, only to push the unit into a state you cannot otherwise reach, such as a rare error branch. Each stub should map to an assertion in the same test. Needing many stubs is a signal the unit does too much.
4. **Asserting that a call happened**, as a last resort, and only for state-changing effects (a write, a send, a spawn) whose result you cannot observe any other way. Never assert on calls to pure getters — the return value is already covered by whatever you assert next.

## Keeping tests deterministic

A flaky test is worse than no test: once people learn to re-run a red test, they stop trusting every other test too. Fix the cause instead of adding retries.

- **Time** — never read the system clock from logic under test. Inject a clock or timestamp so the test can pin it.
- **Async** — never sleep to wait for something. Await the future, use a callback, or poll for the state transition with a generous timeout.
- **Ordering** — tests must pass in any order and in parallel. Watch for statics, singletons, `OnceCell`, and environment variables.
- **Shared state** — when a test must touch global or external state, use `serial_test`'s `#[serial]` or scope the state locally.

If you can't make a test deterministic quickly, quarantine it (`#[ignore]` with a linked issue) rather than leaving an intermittently red test in the suite — and treat that as debt to pay down, not a place to leave it.

## Async and feature-gated code
- For async logic, use `#[tokio::test]` when the code requires a runtime.
- Prefer runtime feature checks (e.g., `FeatureFlag::X.is_enabled()`) over `#[cfg(...)]` so tests don’t require recompilation to toggle behavior.

## Quickstart harness (UI/model tests)
- Prefer `warpui::App::test` for deterministic unit tests around views/models.
- Initialize app models once, then mutate via `update` and assert via `read`.

```rust
use warpui::App;
// In app crate tests prefer `crate::test_util::...`; from other crates use `warp::test_util::...`.
use warp::test_util::{terminal::initialize_app_for_terminal_view, add_window_with_terminal};

#[test]
fn example() {
    App::test((), |mut app| async move {
        // One-time app setup for terminal/view tests
        initialize_app_for_terminal_view(&mut app); // includes settings init
        let term = add_window_with_terminal(&mut app, None);

        // Act
        term.update(&mut app, |view, _ctx| {
            view.model.lock().simulate_block("ls", "out");
        });

        // Assert
        term.read(&app, |view, _ctx| {
            assert!(view.model.lock().block_list().len() > 0);
        });
    })
}
```

## TUI element tests
Tests for the headless TUI render an element tree to text lines rather than drawing pixels. Use `warpui_core::elements::tui::test_support::render_to_lines` and `TuiBuffer::to_lines`, and keep them in `*_tests.rs` files next to the source in `crates/warp_tui` and `crates/warpui_core/src/elements/tui`. They are plain unit tests and do NOT use the GUI integration / real-display / `computer_use` framework. See the `tui-testing` skill for details. The `warpui::App::test` harness above still applies to shared model logic that both front-ends use.

## Common helpers to use
- Terminal model shortcuts: `TerminalModel::mock(..)`, `.simulate_block(..)`, `.finish_block()`, `.simulate_cmd(..)`.
- Builders for focused tests: `terminal::model::test_utils::{TestBlockListBuilder, TestBlockBuilder}`.
- Virtual filesystem for IO-heavy code:
```rust
use virtual_fs::{VirtualFS, Stub};
VirtualFS::test("case", |_dirs, mut fs| {
    fs.with_files(vec![Stub::FileWithContent("path/file.txt", "contents")]);
    // run logic and assert
});
```
- Feature flags (scoped):
```rust
use warp::features::FeatureFlag; // or `use crate::features::FeatureFlag;` inside the app crate
let _flag = FeatureFlag::CreatingSharedSessions.override_enabled(true);
```
- UI numeric assertions (lines):
```rust
assert_lines_approx_eq!(actual_lines, INLINE_BANNER_HEIGHT);
```
- Concurrency: keep `model.lock()` scopes minimal; avoid nested/re-entrant locks in the same call chain.
- Don’t call `initialize_settings_for_tests` directly when using `initialize_app_for_terminal_view` (it already calls it).
- Async needs: use `#[tokio::test]` when a real runtime is required; otherwise prefer `App::test`.
- Tests touching global/external state: consider `serial_test`'s `#[serial]` or local mocking instead of parallelism.

## Running unit tests
- Workspace (parallel):
```bash
cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2
```
- Single crate:
```bash
cargo nextest run -p <crate_name>
```
- Single test (filter by name):
```bash
cargo nextest run -E 'test(<substring>)'
```
- Doc tests:
```bash
cargo test --doc
```

## Linting and formatting
Run before submitting changes:
```bash
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

For a full local check before a PR, you can also run:
```bash
./script/presubmit
```
