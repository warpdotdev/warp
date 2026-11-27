---
name: create-pr-local
specializes: create-pr
specializes_source: warpdotdev/common-skills:.agents/skills/create-pr
description: Repo-specific PR-creation guidance for the warp client. Layers Warp's cargo/presubmit checks, changelog markers, and testing requirements on top of the generic create-pr skill.
---

# Repo-specific PR-creation guidance for `warp`

## Prerequisite: install the parent skill

This skill specializes the core `create-pr` skill (named in the `specializes` frontmatter field) and is not functional on its own. Before applying its guidance, confirm the parent skill is installed and resolvable by name — either project-locally at `.agents/skills/create-pr/SKILL.md` or globally at `~/.agents/skills/create-pr/SKILL.md`. If it is missing from both locations, install it first by copying the skill directory from the source declared in the `specializes_source` frontmatter field (`warpdotdev/common-skills:.agents/skills/create-pr`). Then continue with the guidance below.

This file is a companion to the core `create-pr` skill. It does not redefine the generic PR workflow (branch review, Linear linking, description structure, `gh` usage). It only layers the Warp client's toolchain-specific checks and testing requirements.

## Related skills

- `fix-errors` (with `fix-errors-local`) — resolve presubmit failures before opening a PR.
- `warp-integration-test` — add or update integration coverage for user-visible flows, regressions, and P0 use cases.
- `add-feature-flag` — gate risky changes behind a `FeatureFlag`.

## Pre-PR checks for code changes

If the PR includes Rust/native code changes, run presubmit before opening or updating it:

```bash
./script/presubmit
```

`./script/presubmit` runs:
- `./script/format --check` — code formatting
- `cargo clippy` — linting with all warnings as errors
- All tests (unit, doc, and integration)

The individual commands (matching the versions in `./script/presubmit`) are:

```bash
./script/format
cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings
cargo clippy -p warp_completer --all-targets --tests -- -D warnings
```

You **must** run `./script/format` and `cargo clippy` before:
- opening a new PR that includes code changes,
- pushing new commits that include code changes to an existing PR branch,
- any reviewed branch update that changes code.

If presubmit fails, use the `fix-errors` skill (and `fix-errors-local`) to resolve issues. Documentation-only PRs (skills, markdown, other non-code content) do not need `cargo fmt`/`cargo clippy` to open or update.

## Changelog entries

Add changelog entries when appropriate using the markers at the bottom of `.github/pull_request_template.md`. Use these prefixes (without `{{}}` brackets):
- `CHANGELOG-NEW-FEATURE:` — new, relatively sizable features (use sparingly; these may get marketing/docs).
- `CHANGELOG-IMPROVEMENT:` — new functionality of existing features.
- `CHANGELOG-BUG-FIX:` — fixes related to known bugs or regressions.
- `CHANGELOG-IMAGE:` — GCP-hosted image URLs.

Leave changelog lines blank or remove them if no changelog entry is needed.

## Testing requirements

Include tests when required, scoped to the logical change:

- **Bug fixes** require a regression test that fails before the fix and passes after, named to indicate the bug it prevents.
- **Algorithmic / non-trivial logic** (custom data structures, search APIs, core layout code) requires unit tests. See the `rust-unit-tests` skill for conventions (test files named `${filename}_tests.rs` or `mod_test.rs`, included via `#[cfg(test)] #[path = "..."] mod tests;`).
- **UI components** (implementations of `View`) should have a simple layout test asserting the component lays out without a panic:

```rust
#[test]
fn test_component_can_layout() {
    use warpui::App;
    use warp::test_util::{terminal::initialize_app_for_terminal_view, add_window_with_terminal};

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let term = add_window_with_terminal(&mut app, None);
        term.update(&mut app, |view, ctx| {
            // Create and lay out the component — should not panic.
        });
    })
}
```

- **P0 use cases** (any behavior that, if broken, warrants an out-of-band release) require an integration test under `crates/integration/` that exercises the full user-facing flow. Use the `warp-integration-test` skill for implementation, registration, and validation details.

## Co-author attribution

The canonical Oz attribution — the commit trailer and the agent reply prefix — is defined by the `agent-attribution` skill (`warpdotdev/warp-skills:.agents/skills/agent-attribution`). Treat that skill as the source of truth: if it is resolvable, follow its wording. Install it from that source if it is missing.

For convenience, the trailer it defines is:

```
Co-Authored-By: Oz <oz-agent@warp.dev>
```

Include this trailer on every commit message and PR description.
