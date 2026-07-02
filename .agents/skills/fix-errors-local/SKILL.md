---
name: fix-errors-local
specializes: fix-errors
specializes_source: warpdotdev/common-skills:.agents/skills/fix-errors
description: Repo-specific error-fixing guidance for the warp Rust client. Layers Warp's cargo/clippy commands, WASM and local_fs gating, and presubmit/nextest workflow on top of the generic fix-errors skill.
---

# Repo-specific error-fixing guidance for `warp`

## Prerequisite: install the parent skill

This skill specializes the core `fix-errors` skill (named in the `specializes` frontmatter field) and is not functional on its own. Before applying its guidance, confirm the parent skill is installed and resolvable at `.agents/skills/fix-errors/SKILL.md`. If it is missing, install it first by copying the skill directory from the source declared in the `specializes_source` frontmatter field (`warpdotdev/common-skills:.agents/skills/fix-errors`). Then continue with the guidance below.

This file is a companion to the core `fix-errors` skill. It does not redefine the generic categories of compilation, lint, and test errors. It only layers the Warp client's exact cargo toolchain commands and the WASM/`local_fs` conventions specific to this repository.

## Presubmit

Run all checks at once before opening or updating a PR:

```bash
./script/presubmit
```

This runs formatting, linting, and all tests. If it passes, you're ready to open a PR.

## Individual checks

Run checks separately when debugging specific issues.

**Rust formatting:**
```bash
./script/format --check
```
Run `./script/format` to fix formatting. It wraps `cargo fmt` with `RUSTC_BOOTSTRAP` and the repo's import-grouping configuration.

**Clippy (full workspace):**
```bash
cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings
cargo clippy -p warp_completer --all-targets --tests -- -D warnings
```

**WASM clippy:**
```bash
cargo clippy --locked --target wasm32-unknown-unknown --profile release-wasm-debug_assertions -- -D warnings
```

**Objective-C/C/C++ formatting:**
```bash
./script/run-clang-format.py -r --extensions 'c,h,cpp,m' ./crates/warpui/src/ ./app/src/
```

**All tests:**
```bash
cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2
cargo nextest run -p warp_completer --features v2
```

**Doc tests:**
```bash
cargo test --doc
```

## Running specific tests

```bash
# Single package
cargo nextest run -p <package_name>

# Filter by test name
cargo nextest run -E 'test(<substring>)'

# Specific package with filter
cargo nextest run -p <package_name> -E 'test(<substring>)'

# With output (no capture)
cargo nextest run -p <package> --nocapture
```

## WASM-specific errors

WASM builds (`wasm32-unknown-unknown` target) don't support filesystem operations. Code that uses filesystem APIs must be gated behind the `local_fs` feature flag. This is the one case where inline (non-top-level) imports and `#[cfg(...)]` gating are expected, rather than runtime feature checks.

Common WASM errors:
- Dead-code warnings for code only used in non-WASM builds.
- Unused code only relevant when `local_fs` is available.
- Tests that require filesystem access.

Fixes:

**Gate tests behind `local_fs`:**
```rust
#[test]
#[cfg(feature = "local_fs")]
fn test_find_git_repo_with_worktree() {
    // Test that uses filesystem operations.
}
```

**Conditionally allow dead code for types only used when `local_fs` is enabled:**
```rust
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[derive(Clone, EnumDiscriminants, Serialize)]
pub enum ExampleType {
    Variant1,
    Variant2,
    Variant3,
}
```

Discover WASM errors by running the WASM clippy command above. UI-framework code lives under `crates/warpui/`; the main app under `app/src/`.

## Repository conventions when fixing

Apply the Rust conventions from this repo's `AGENTS.md` while resolving errors:
- Keep imports at top level (the exception is `cfg`-guarded branches such as `local_fs`).
- Prefer inline format arguments in macros (`eprintln!("{message}")`) to satisfy the `uninlined_format_args` lint.
- Prefer exhaustive `match` arms over a wildcard `_` so new enum variants surface at compile time.
- For new feature gating, prefer `FeatureFlag::YourFlag.is_enabled()` runtime checks over `#[cfg(...)]` unless the code cannot compile without a compile-time gate.

## After fixing

- Always run `./script/format` and `cargo clippy` before pushing.
- Run `./script/presubmit` before opening or updating a PR (see the `create-pr` and `create-pr-local` skills).
- Verify tests pass in the areas you modified.
