# Contributing to Warper

Warper is a small hard fork of [Warp](https://www.warp.dev). The contribution workflow is intentionally minimal: open an issue, send a PR. No labels-as-permissions, no bot review, no spec gates.

## Bugs and feature requests

Open a [GitHub issue](https://github.com/ruslanvakhitov/warper/issues).

For bugs, include:

- A clear summary of what's broken.
- Steps to reproduce.
- Expected vs. actual behavior.
- Your OS and how you built Warper (debug build via `./script/run`, packaged bundle, etc.).
- Relevant logs (see the **Local data** section of [README.md](README.md) for log paths).

For feature requests, describe the user-facing problem before any proposed implementation.

If a bug also reproduces in upstream Warp, please file it with them too — they have engineering resources we don't.

## Scope check

Warper is intentionally narrow. Before opening a feature request, skim the **What's out** section of [README.md](README.md). Features that need an account, a server, a cloud drive, team objects, session sharing, or any hosted infrastructure are out of scope and will be closed.

## Code changes

1. Branch from `master`.
2. Make the change.
3. Run `./script/presubmit` and fix anything it flags.
4. Open a PR against `master` describing what changed and why.

Keep PRs focused on a single logical change. Smaller PRs review faster.

## Code style

`cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` must pass. See [WARP.md](WARP.md) for the full style guide, including WarpUI patterns and the terminal model locking rules.

## Tests

Add tests where they make sense:

- Regression tests for bug fixes.
- Unit tests for non-trivial logic.
- Integration coverage for user-facing flows that can be exercised end to end.

Run unit tests with `cargo nextest run`. The integration test framework lives under `crates/integration/`; see [WARP.md](WARP.md) for details.

## Coding agents

Use whatever you like — your own editor, Claude Code, Codex, Gemini CLI, Cursor, no agent at all. There's no in-repo agent harness and no contributor distinction based on tooling. The bar is the same code regardless of who or what typed it: presubmit passes, tests cover the change, PR is focused.

## Code of conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Security

Don't file security issues publicly. See [SECURITY.md](SECURITY.md).
