# Repository layout

This guide gives contributors a quick map of the Warp repository so they can find the right files without reorganizing the source tree.

## Top-level directories

- `.agents/` — Agent-readable skills and workflow guidance used by Oz and other coding agents.
- `.cargo/` — Cargo configuration shared by local development and repository tooling.
- `.claude/` — Claude Code settings, skills, and helper scripts for contributors who use Claude-based agents.
- `.config/` — Repository-local configuration for developer tools such as Nextest.
- `.github/` — GitHub issue, pull request, and automation configuration.
- `.vscode/` — Recommended Visual Studio Code settings, tasks, launch profiles, and extensions.
- `.warp/` — Warp-specific repository metadata and local workflow assets.
- `app/` — The main Warp application crate, including platform entry points and product features.
- `command-signatures-v2/` — Command signature data and supporting tooling.
- `crates/` — Rust workspace crates shared by the app, tests, tooling, and integrations.
- `docker/` — Development and CI container assets.
- `resources/` — Packaged resources and generated/static assets that are shared at repository scope.
- `script/` — Developer, build, formatting, and validation scripts.
- `specs/` — Product and technical specs grouped by ticket identifier.

## Common places to start

- Start with `README.md` for installation, contribution, licensing, and support links.
- Use `CONTRIBUTING.md` for the issue-to-PR workflow, readiness labels, spec process, and review expectations.
- Use `WARP.md` for build commands, coding style, testing guidance, and architectural notes.
- Use `Cargo.toml` to see the workspace members and shared dependency declarations.
- Use `app/src/` when changing the main Warp client behavior.
- Use `crates/integration/` when adding or updating end-to-end integration tests.
- Use `crates/warpui/`, `crates/warpui_core/`, and `crates/ui_components/` when working on UI framework or shared component code.

## Where to put new files

- Add application feature code under the owning module in `app/src/`.
- Add reusable Rust functionality as an existing crate module under `crates/`, or create a new crate only when the boundary is intentional and reusable.
- Keep assets close to the code that owns them when they are feature-specific. Use `resources/` for assets that are packaged or shared across the repository.
- Add developer automation under `script/` when it is repository-wide; add crate-local helpers next to the crate when they are specific to one crate.
- Add specs under `specs/<ticket-id>/` using the ticket identifier as the directory name.
- Avoid creating broad language- or type-based buckets such as `assets/python/` for source code; prefer the owning feature, crate, or tool directory.
- Prefer updating this guide and the existing owning directory over creating a broad catch-all directory.

## Finding code by area

- AI and agent behavior: `app/src/ai/` and related crates under `crates/`.
- Editor behavior: `app/src/editor/` and `crates/editor/`.
- Settings and preferences: `app/src/settings/` and `crates/settings/`.
- Persistence and migrations: `crates/persistence/`.
- GraphQL schema and client code: `crates/graphql/` and `crates/warp_graphql_schema/`.
- Terminal model and emulation: terminal-related modules in `app/src/` and `crates/warp_terminal/`.
- Warp UI primitives and components: `crates/warpui/`, `crates/warpui_core/`, and `crates/ui_components/`.

When in doubt, search for nearby feature names, review the owning crate's `README.md` if present, and keep new files near the code paths they modify.
