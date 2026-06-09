---
name: rust-unit-tests
description: Write, improve, and run Rust unit tests in the warp Rust codebase.
---

# Rust Unit Tests in warp

## Scope
- This skill focuses on crate-level unit tests.
- Favor incremental, well-scoped tests that exercise a single function or behavior per case.

## Where unit tests live
- Put unit tests in separate files named `${filename}_tests.rs` or `mod_test.rs`.
- Include the test module at the end of the corresponding source file:

```rust
#[cfg(test)]
#[path = "filename_tests.rs"] // or "mod_test.rs"
mod tests;
```

## Writing good tests
- Use descriptive names: `fn parses_utf8_sequence_when_valid()`.
- Prefer `assert_eq!`/`assert_ne!` over `assert!` for clearer diffs.
- Use `#[should_panic]` only when panic semantics are intended API.
- Minimize global state; inject dependencies via traits/constructors to make logic testable without heavy mocking.
- When adding enums or expanding behavior, prefer exhaustive matches in code under test and mirror cases in tests.
- Be mindful of terminal model locking: avoid patterns that acquire multiple `model.lock()` calls in the same call stack from tests.

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
