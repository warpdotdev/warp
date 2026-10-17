# gh-13341 — Recursive file search in the file finder outside git repositories

See `specs/gh-13341/PRODUCT.md` for behavior. Researched at commit `dfabfa5bb29d1b7fde7a946cde965cb78332a00b`.

## Context
The file finder's file results come from a shared `FileSearchModel` plus two data sources wired into the AI context menu. Recursive results depend on the repo-metadata index, which only exists for detected git repositories — hence the gap this feature closes.

- [`app/src/search/files/model.rs (77-99) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/search/files/model.rs#L77-L99) — `FileSearchModel::repo_root_location`: resolves the working directory to a repo root via `DetectedRepositories::get_root_for_path`. Returns `None` outside a repo, which is why `get_repo_contents` yields nothing there.
- [`app/src/search/files/model.rs (112-188) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/search/files/model.rs#L112-L188) — `get_folder_contents`: the non-recursive `std::fs::read_dir` of the immediate cwd used for the non-git zero state.
- [`app/src/search/files/model.rs (199-222) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/search/files/model.rs#L199-L222) — `get_repo_contents`: the recursive, index-backed path (git only). `fuzzy_match_path` (L404+) is the shared matcher.
- [`app/src/search/ai_context_menu/files/data_source.rs (42-120) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/search/ai_context_menu/files/data_source.rs#L42-L120) — `file_data_source_for_current_repo` (recursive) and `file_data_source_for_pwd` (immediate dir); `MAX_RESULTS = 200`; `FileSnapshot` carries `last_opened` recents used for zero-state ranking.
- [`app/src/search/ai_context_menu/view.rs (829-874) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/search/ai_context_menu/view.rs#L829-L874) — `reset_mixer`: `AIContextMenuCategory::CurrentFolderFiles` → `file_data_source_for_pwd`; `RepoFiles` → `file_data_source_for_current_repo`. Category selection is where the non-git recursive source gets routed (also the AllCategories path at L1073-1085).
- [`app/src/search/async_snapshot_data_source.rs:23 @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/search/async_snapshot_data_source.rs#L23) — `AsyncSnapshotDataSource<S, A>`: the async snapshot+filter plumbing the new source builds on.
- [`crates/repo_metadata/src/repositories.rs (228-236) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/crates/repo_metadata/src/repositories.rs#L228-L236) — `DetectedRepositories::get_root_for_path`; [`find_git_repo (337-345)`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/crates/repo_metadata/src/repositories.rs#L337-L345) walks up to `$HOME` and returns `None` if no `.git` — the exact condition that gates the new path.
- [`crates/warp_ripgrep/src/search.rs (8, 67) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/crates/warp_ripgrep/src/search.rs#L67) — existing `ignore::WalkBuilder` usage; the `ignore` crate (a `repo_metadata` dep) is the traversal engine to reuse.
- [`app/src/code/file_tree/view.rs (1719-1722) @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/code/file_tree/view.rs#L1719-L1722) and `CodeSettings::show_hidden_files` (used at L292/355) plus the `file_tree:toggle_hidden_files` command in [`app/src/workspace/mod.rs:782 @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/app/src/workspace/mod.rs#L782) — the existing hidden-files setting to reuse for PRODUCT §6.
- [`crates/warp_core/src/features.rs @ dfabfa5b`](https://github.com/warpdotdev/warp/blob/dfabfa5bb29d1b7fde7a946cde965cb78332a00b/crates/warp_core/src/features.rs) — `FeatureFlag` enum for the gate (PRODUCT §14); follow the `add-feature-flag` skill.

## Proposed changes
1. **New non-git recursive source.** Add `file_data_source_for_pwd_recursive()` in `app/src/search/ai_context_menu/files/data_source.rs`, alongside the existing two, built on `AsyncSnapshotDataSource`. It walks the cwd on each non-empty query using `ignore::WalkBuilder` (reusing the `crates/warp_ripgrep` pattern) configured to match the PRODUCT decisions:
   - `hidden(!show_hidden)` driven by the reused `CodeSettings::show_hidden_files` (PRODUCT §6); `git_ignore(false)` / `require_git(false)` so `.gitignore` is not honored (§8); `follow_links(false)` (§9).
   - Skip `.git` and `node_modules` via a `filter_entry` predicate (§7); keep the list in one place for later configurability.
   - `max_depth` + a scan/time budget and `MAX_RESULTS` (200) cap, streaming matches into the snapshot as they are found; cancel on query change via the existing async-source lifecycle (§4, §10).
   - Reuse `FileSearchModel::fuzzy_match_path` for scoring and the existing `last_opened` recency for tie-breaks (§11).
2. **Model helper.** Add a `FileSearchModel` method (e.g. `get_pwd_recursive_contents(query, app)`) that owns the `WalkBuilder` traversal and returns `FileSearchResult`s relative to the cwd, so the data source stays thin and the traversal is unit-testable without the menu. Reuse `get_folder_contents` unchanged for the zero state (§3).
3. **Category routing.** In `app/src/search/ai_context_menu/view.rs` (`reset_mixer` and the AllCategories setup), when `repo_root_location` is `None` (not in a repo) and the flag is on, use `file_data_source_for_pwd_recursive()` for non-empty queries and keep `file_data_source_for_pwd` for the zero state. Inside a repo, nothing changes.
4. **Feature flag.** Add a `FeatureFlag` variant gating the new routing (per `add-feature-flag`); with it off, the current `file_data_source_for_pwd`-only behavior remains (§14).
5. **Setting.** Reuse the existing `show_hidden_files` setting rather than adding a new one (pending the PRODUCT §6 open question); no new command needed since `file_tree:toggle_hidden_files` already toggles it.

Rationale: this mirrors the existing three-source structure and the `ignore`-crate traversal already used elsewhere, so the change is localized to the file-search module plus one flag, with no change to the in-repo index path.

## Testing and validation
- Unit tests in `app/src/search/ai_context_menu/files/data_source_tests.rs` / a `FileSearchModel` test module, over a temp non-git directory fixture:
  - recursive match across subdirectories returns nested files (§2, §4); zero-state returns only immediate dir + recents (§3).
  - `.git` / `node_modules` skipped (§7); gitignored file still present (§8); symlinked dir not followed (§9).
  - hidden included by default and excluded when the setting is off (§6).
  - depth/result caps produce partial results without error (§10); unreadable dir is skipped (§13).
- Flag-off test: outside a repo, only immediate-dir results are returned (§14).
- `cargo check`, `./script/format`, and `cargo clippy` per `AGENTS.md`; manual smoke: `cd` to a non-git dir with nested files, `⌘O`, type a substring, confirm nested matches stream in.
- Each numbered PRODUCT invariant above maps to one of these checks.

## Parallelization
Not beneficial: the change is small and tightly coupled — one data-source file, one model helper, one view routing site, and a flag all in the file-search module. A single agent implements it on branch `oz/gh-13341-nongit-file-finder-spec` (this branch), which currently carries the specs; implementation lands in a follow-up PR once the spec is approved.
