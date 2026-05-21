# WARP.md

Engineering reference for working in this repository. For an overview of what Warper is and isn't, see [README.md](README.md). For the contribution workflow, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Development Commands

### Build and Run

- `./script/bootstrap` — platform-specific setup
- `./script/run` — build and run Warper as an app bundle (macOS)
- `cargo run` — build and run directly through cargo

### Testing

- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2` — workspace tests with nextest
- `cargo nextest run -p warp_completer --features v2` — completer tests with v2 features
- `cargo test --doc` — doc tests
- `cargo test` — standard tests for individual packages

### Linting and Formatting

- `./script/presubmit` — full presubmit (fmt, clippy, tests)
- `cargo fmt` — format Rust
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` — clippy
- `./script/run-clang-format.py -r --extensions 'c,h,cpp,m' ./crates/warpui/src/ ./app/src/` — format C/C++/Obj-C
- `find . -name "*.wgsl" -exec wgslfmt --check {} +` — check WGSL shader formatting

### Smoke checks specific to Warper

- `./script/warper_offline_local_smoke` — verifies the app runs offline with no hosted dependencies
- `./script/check_warper_static_denylist all-runtime` — guards against reintroducing hosted Warp/Oz/account/billing/Drive/team/sharing/telemetry/update entrypoints

### Platform Setup

- `./script/bootstrap` calls platform-specific bootstrap scripts.
- `./script/install_cargo_build_deps` — install Cargo build dependencies
- `./script/install_cargo_test_deps` — install Cargo test dependencies

## Architecture Overview

A Rust terminal emulator with a custom UI framework called **WarpUI**.

### Key Components

**WarpUI Framework** (`ui/`):

- Custom UI framework with an Entity-Component-Handle pattern
- Global `App` object owns all views/models (entities)
- Views hold `ViewHandle<T>` references to other views
- `AppContext` provides temporary access to handles during render/events
- Elements describe visual layout (Flutter-inspired)
- Actions system for event handling
- `MouseStateHandle` must be created once during construction and then referenced/cloned anywhere mouse input is tracked. Inline `MouseStateHandle::default()` during render breaks mouse interactions.

**Main App** (`app/`):

- Terminal emulation and shell management (`terminal/`)
- AI integration via OpenRouter (`ai/`) — the server-tied AI assistant paths from upstream are removed or stubbed in this fork
- Settings and preferences (`settings/`)
- Workspace and session management (`workspace/`)

**Core Libraries**:

- `warp_core/` — core utilities and platform abstractions
- `editor/` — text editing
- `ui/` — custom UI framework
- `ipc/` — inter-process communication
- `graphql/` — GraphQL client inherited from upstream; most server-bound call sites are stubbed or removed in this fork

### Key Architectural Patterns

1. **Entity-Handle System** — views reference other views via handles, not direct ownership.
2. **Modular Structure** — the workspace contains multiple configurations, each with terminals, notebooks, etc.
3. **Cross-Platform** — native implementations for macOS, Windows, and Linux, plus a WASM target.

### Removed in this fork

The upstream codebase includes a large amount of hosted-product code that is deliberately removed or gated off in Warper. Do not reintroduce:

- Warp account, sign-up, sign-in, SSO, anonymous cloud users
- Subscriptions, billing, credits, request limits
- Warp Drive cloud sync, team drives, shared-with-me, cloud trash
- Teams, enterprise admin, ACLs, audit, hosted workspaces
- Session sharing, remote control, RTC presence, link collaboration
- Oz, cloud agents, hosted task orchestration
- Telemetry, hosted crash reporting, RudderStack, Sentry
- Warp-hosted autoupdate
- Remote feature flag / experiment fetching

`./script/check_warper_static_denylist all-runtime` is the source of truth for the denylist.

## Development Guidelines

### Workspace Structure

- Cargo workspace with 34+ member crates
- Main binary lives in `app/`, UI framework in `ui/`
- Platform-specific code is conditionally compiled
- Integration tests live in `crates/integration/`

### Coding Style Preferences

- Avoid unnecessary type annotations, especially in closure params.
- Avoid path qualifiers; prefer imports at the top of the file. Exception: inside `cfg`-guarded branches you can embed the import into the relevant scope or use an absolute path for one-offs.
- If a function takes a context parameter (`AppContext`, `ViewContext`, or `ModelContext`), name it `ctx` and put it last. The one exception is functions that take a closure parameter, in which case the closure goes last.
- Remove unused parameters completely rather than prefixing them with `_`. Update the function signature and all call sites accordingly.
- Prefer inline format arguments in `println!`, `eprintln!`, `format!`, etc. (`eprintln!("{message}")` rather than `eprintln!("{}", message)`). Satisfies clippy's `uninlined_format_args` lint.
- Don't remove existing comments when making unrelated changes. Only modify a comment if the logic it describes has changed.

### Terminal Model Locking

- Be extremely careful when calling `model.lock()` on the terminal model (`TerminalModel`). Acquiring multiple locks on the same model from different call sites can deadlock and freeze the UI (beach ball on macOS).
- Before adding a new `model.lock()` call, verify that no caller in the current call stack already holds the lock.
- Prefer passing already-locked model references down the call stack rather than acquiring new locks.
- If you must lock the model, keep the lock scope as short as possible and avoid calling other functions that might also attempt to lock.

### Testing

- Use `cargo nextest` for parallel test execution.
- Integration tests use a custom framework in `crates/integration/`.
- Run tests via `./script/presubmit` before opening a PR.
- Unit tests go in separate files using the naming convention `${filename}_tests.rs` or `mod_test.rs`.
- Include test files at the end of their corresponding module with:

  ```rust
  #[cfg(test)]
  #[path = "filename_tests.rs"]
  mod tests;
  ```

### Pull Request Workflow

- Run `cargo fmt` and `cargo clippy` (the versions specified in `./script/presubmit`) before pushing.
- All checks must pass before opening a PR.
- Open PRs against `master`. Keep them focused on a single logical change.

### Database

- Diesel ORM with SQLite.
- Migrations in `migrations/`.
- Schema in `app/src/persistence/schema.rs`.

### GraphQL

- Schema lives in `graphql/api/schema.graphql`.
- Generated client code and TypeScript types are inherited from upstream. Many server-bound call sites are stubbed in Warper; if you're adding a new GraphQL call, check that the endpoint isn't part of the removed hosted surface first.

## Feature Flags

Warp uses compile-time feature flags with a small runtime plumbing layer.

How to add a feature flag:

- Add a new variant to `warp_core/src/features.rs` in the `FeatureFlag` enum.
- (Optional) Enable it by default for dogfood builds by listing it in `DOGFOOD_FLAGS`.
- Gate code paths with `FeatureFlag::YourFlag.is_enabled()`.
- For preview or release rollout, add to `PREVIEW_FLAGS` or `RELEASE_FLAGS` respectively.

Best practices:

- Prefer runtime checks (`FeatureFlag::YourFlag.is_enabled()`) over `#[cfg(...)]` directives so flags can be toggled without recompilation and are easier to clean up. Use `#[cfg(...)]` only when the code cannot compile without it (e.g. platform-specific code, conditional dependencies).
- Keep flags high-level and product-focused, not per-call-site.
- Remove the flag and dead branches once the gated behavior stabilizes.
- For UI sections that expose a new feature, hide the UI behind the same flag.

Example:

```rust
#[derive(Sequence)]
pub enum FeatureFlag {
    YourNewFeature,
}

pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::YourNewFeature,
];

if FeatureFlag::YourNewFeature.is_enabled() {
    // gated behavior
}
```

## Exhaustive Matching

When adding or editing `match` statements, avoid the wildcard `_` where practical. Exhaustive matching catches new enum variants at compile time, which is especially useful when the upstream codebase keeps evolving and new variants land via merges.
