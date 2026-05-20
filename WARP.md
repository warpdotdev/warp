# WARP.md

This file provides guidance when working with code in this repository.

## Development Commands

### Build and Run
- `cargo run` - Build and run Warp locally
- `cargo bundle --bin warp` - Bundle the main app

### Running with local warp-server
To connect Warp client to a local warp-server instance:

```bash
# Connect to server on default port 8080
cargo run --features with_local_server

# Connect to server on custom port (e.g., 8082)
SERVER_ROOT_URL=http://localhost:8082 WS_SERVER_URL=ws://localhost:8082/graphql/v2 cargo run --features with_local_server
```

Environment variables:
- `SERVER_ROOT_URL` - HTTP endpoint (default: `http://localhost:8080`)
- `WS_SERVER_URL` - WebSocket endpoint (default: `ws://localhost:8080/graphql/v2`)

### Testing
- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2` - Run tests with nextest
- `cargo nextest run -p warp_completer --features v2` - Run completer tests with v2 features
- `cargo test --doc` - Run doc tests
- `cargo test` - Run standard tests for individual packages

### Linting and Formatting
- `./script/presubmit` - Run all presubmit checks (fmt, clippy, tests)
- `cargo fmt` - Format code
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` - Run clippy
- `./script/run-clang-format.py -r --extensions 'c,h,cpp,m' ./crates/warpui/src/ ./app/src/` - Format C/C++/Obj-C code
- `find . -name "*.wgsl" -exec wgslfmt --check {} +` - Check WGSL shader formatting

### Platform Setup
- `./script/bootstrap` - Platform-specific setup plus common agent skill installation from `skills-lock.json`; prompts for project/global when an install or update is needed unless a target flag or environment override is provided.
- `./script/bootstrap --skip-common-skills` - Platform setup without installing or updating common agent skills.
- `./script/bootstrap --install-common-skills` - Explicitly install common agent skills from `skills-lock.json`; this is the default behavior.
- `./script/bootstrap --install-common-skills-in-repo` - Platform setup plus common agent skill installation in this checkout's `.agents/skills`.
- `./script/bootstrap --install-common-skills-globally` - Platform setup plus common agent skill installation in `~/.agents/skills`.
- `../common-skills/scripts/install_common_skills --repo-root "$PWD" --project --if-needed` - Install or refresh shared agent skills in this checkout's `.agents/skills`.
- `../common-skills/scripts/install_common_skills --repo-root "$PWD" --global --if-needed` - Install or refresh shared agent skills in `~/.agents/skills`.
- `../common-skills/scripts/remove_common_skills --repo-root "$PWD"` - Remove shared agent skills listed in `skills-lock.json` from this checkout's `.agents/skills`.
- `../common-skills/scripts/remove_common_skills --repo-root "$PWD" --global` - Remove shared agent skills listed in `skills-lock.json` from `~/.agents/skills`.
- `../common-skills/scripts/remove_common_skills --repo-root "$PWD" --clear-lock` - Remove shared agent skills from this checkout and delete `skills-lock.json`.
- `./script/install_cargo_build_deps` - Install Cargo build dependencies
- `./script/install_cargo_test_deps` - Install Cargo test dependencies

`skills-lock.json` is the standard project lock file managed by `npx skills`. `warpdotdev/common-skills/scripts/install_common_skills` requires an explicit install target before restoring: pass `--project`, pass `--global`, set `WARP_COMMON_SKILLS_INSTALL_TARGET`, or answer the interactive prompt from bootstrap. Non-interactive flows fail if no target is explicit. The installer creates `skills-lock.json` from `warpdotdev/common-skills` if it is missing, uses global as the recommended interactive default, errors if common skills are present in both project and global locations, prevents a global install pinned to one lock from being silently overwritten by another checkout pinned to a different lock, and verifies installed skills against the lock after successful install or skip paths. `script/run` and `script/bootstrap` execute this installer with `script/resolve_common_skills`, which uses `WARP_COMMON_SKILLS_SCRIPTS_DIR` only when explicitly set and otherwise runs the raw script from `warpdotdev/common-skills`. To test a remote common-skills branch, set `WARP_COMMON_SKILLS_REF=<branch>`. Cloud setup should use `common-skills/scripts/install_common_skills --repo-root <warp-checkout> --project --if-needed --non-interactive` or set `WARP_COMMON_SKILLS_INSTALL_TARGET=project` to avoid the prompt. To update the locked common skills, run `npx --yes skills@1.5.6 update -p -y` and commit the resulting `skills-lock.json` changes.

## Architecture Overview

This is a Rust-based terminal emulator with a custom UI framework called **WarpUI**.

### Key Components

**WarpUI Framework** (`ui/`):
- Custom UI framework with Entity-Component-Handle pattern
- Global `App` object owns all views/models (entities)
- Views hold `ViewHandle<T>` references to other views
- `AppContext` provides temporary access to handles during render/events
- Elements describe visual layout (Flutter-inspired)
- Actions system for event handling
- MouseStateHandle must be created once during construction, and then referenced/cloned anywhere we're using mouse input to track mouse changes. Inline `MouseStateHandle::default()` while rendering will cause no mouse interactions to work.

**Main App** (`app/`):
- Terminal emulation and shell management (`terminal/`)
- AI integration including Agent Mode (`ai/`)
- Cloud synchronization and Drive features (`drive/`)
- Authentication and user management (`auth/`)
- Settings and preferences (`settings/`)
- Workspace and session management (`workspace/`)

**Core Libraries**:
- `crates/warp_core/` - Core utilities and platform abstractions
- `crates/editor/` - Text editing functionality
- `crates/warpui/` and `crates/warpui_core/` - Custom UI framework
- `crates/ipc/` - Inter-process communication
- `crates/graphql/` - GraphQL client and schema

### Key Architectural Patterns

1. **Entity-Handle System**: Views reference other views via handles, not direct ownership
2. **Modular Structure**: Workspace contains multiple workspace configurations, each with terminals, notebooks, etc.
3. **Cross-Platform**: Native implementations for macOS, Windows, Linux, plus WASM target
4. **AI Integration**: Built-in AI assistant with context awareness and codebase indexing
5. **Cloud Sync**: Objects can be synchronized across devices via Warp Drive

### Development Guidelines

**Workspace Structure**:
- This is a Cargo workspace with 60+ member crates
- Main binary is in `app/`, UI framework in `crates/warpui/`
- Platform-specific code is conditionally compiled
- Integration tests are in `crates/integration/`

**Coding Style Preferences**:
- Avoid unnecessary type annotations, especially in closure params.
- Avoid using too many Rust path qualifiers and use imports for concision. Place import statements at the top of the file as per convention.
  An exception to this is inside cfg-guarded code branches. In those cases, you can either embed the import into the relevant scope or just use an absolute path for one-offs.
- If a function takes a context parameter (`AppContext`, `ViewContext`, or `ModelContext`), it should be named `ctx` and go last. The one exception is for
  functions that take a closure parameter, in which case the closure should be last.
- Always remove unused parameters completely rather than prefixing them with `_`. Update the function signature and all call sites accordingly.
- Prefer inline format arguments in macros like `println!`, `eprintln!`, and `format!` (for example, `eprintln!("{message}")` instead of `eprintln!("{}", message)`) to satisfy Clippy's `uninlined_format_args` lint.
- Do not pass `Itertools::format` results directly to logging macros (`log::*`, `safe_*`, etc.). `Itertools::format` produces a single-use formatter, while logging implementations may format a message more than once. Use a reusable `String` such as `iter.join(", ")` for logging arguments instead. Direct use in `format!` or `write!` is fine.
- Do not remove existing comments when making unrelated changes. Only remove or modify a comment if the logic it describes has changed.

**Terminal Model Locking**:
- Be extremely careful when calling `model.lock()` on the terminal model (`TerminalModel`). Acquiring multiple locks on the same model from different call sites can cause a deadlock, resulting in a UI freeze (beach ball on macOS).
- Before adding a new `model.lock()` call, verify that no caller in the current call stack already holds the lock.
- Prefer passing already-locked model references down the call stack rather than acquiring new locks.
- If you must lock the model, keep the lock scope as short as possible and avoid calling other functions that might also attempt to lock.

**Testing**:
- Use `cargo nextest` for parallel test execution
- Integration tests use custom framework in `integration/`
- Tests should be run via presubmit script before submitting
- Unit tests should be placed in separate files using the naming convention `${filename}_tests.rs` or `mod_test.rs`
- Test files should be included at the end of their corresponding module with:
  ```rust
  #[cfg(test)]
  #[path = "filename_tests.rs"]  // or "mod_test.rs"
  mod tests;
  ```

**Pull Request Workflow**:
- **ALWAYS** run cargo fmt and cargo clippy (the versions specified in ./script/presubmit) before opening a PR or pushing updates to an existing PR branch
- Those commands must pass completely before creating or updating a pull request
- Specifically, ensure `cargo fmt` and `cargo clippy` checks pass
- If they fail, fix all issues before proceeding with the PR
- This applies to:
  - Opening new pull requests
  - Pushing new commits to existing PR branches
  - Any branch updates that will be reviewed
 - When opening PRs, use the PR template at `.github/pull_request_template.md`
 - Add changelog entries when appropriate using the format at the bottom of the PR template. Use the following prefixes (without the `{{}}` brackets):
   - `CHANGELOG-NEW-FEATURE:` for new, relatively sizable features (use sparingly - these may get marketing/docs)
   - `CHANGELOG-IMPROVEMENT:` for new functionality of existing features
   - `CHANGELOG-BUG-FIX:` for fixes related to known bugs or regressions
   - `CHANGELOG-IMAGE:` for GCP-hosted image URLs
   - Leave changelog lines blank or remove them if no changelog entry is needed

**Database**:
- Uses Diesel ORM with SQLite
- Migrations in `crates/persistence/migrations/`
- Schema defined in `crates/persistence/src/schema.rs`

**GraphQL**:
- Schema and client code generation from `crates/warp_graphql_schema/api/schema.graphql`
- TypeScript types generated for frontend integration

### Feature Flags

Warp uses compile-time feature flags with a small runtime plumbing layer.

How to add a feature flag:
- Add a new variant to `warp_core/src/features.rs` in the `FeatureFlag` enum
- (Optional) Enable it by default for dogfood builds by listing it in `DOGFOOD_FLAGS`
- Gate code paths with `FeatureFlag::YourFlag.is_enabled()`
- For preview or release rollout, add to `PREVIEW_FLAGS` or `RELEASE_FLAGS` respectively (as appropriate)

Best practices:
- **Prefer runtime checks over cfg directives**: Prefer `FeatureFlag::YourFlag.is_enabled()` over `#[cfg(...)]` compile-time directives so flags can be toggled without recompilation and are easier to clean up later. Use `#[cfg(...)]` only when the code cannot compile without them (for example, platform-specific code or dependencies that do not exist when the feature is disabled).
- Keep flags high-level and product-focused rather than per-call-site
- Remove the flag and dead branches after launch has stabilized
- For UI sections that expose a new feature, hide the UI behind the same flag

Example:
```rust
#[derive(Sequence)]
pub enum FeatureFlag {
    YourNewFeature,
}

// Default-on for dogfood builds
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::YourNewFeature,
];

// Use in code
if FeatureFlag::YourNewFeature.is_enabled() {
    // gated behavior
}
```

### Exhaustive Matching

When adding/editing match statements, avoid using the wildcard _ when at all possible. Exhaustive matching is helpful for ensuring that all variants are handled, especially when adding new variants to enums in the future.
