# Git source control in the Tools panel: technical design

## Context

This implements the behavior in [PRODUCT.md](./PRODUCT.md) on top of Warp's existing Tools-panel, repository-discovery, and Git-status infrastructure. The inspected Warp revision is `a0d589460b58bc8ad2cd4d6795339fdef79ed0bd`; the inspected VS Code revision is `1224e99c64b0b19b8b71da6bd1ac0dc86b3df224`.

- [`app/src/workspace/view/left_panel.rs @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/workspace/view/left_panel.rs) owns the Tools tab enum, icon row, active pane-group wiring, focus, and tab content.
- [`app/src/workspace/view.rs:23667 @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/workspace/view.rs#L23667) computes the available Tools tabs; it is the runtime feature-flag gate.
- [`app/src/pane_group/working_directories.rs:229 @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/pane_group/working_directories.rs#L229) publishes per-pane-group repository and focused-repository changes, including worktrees and multiple repositories.
- [`app/src/code_review/git_repo_model/local.rs @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/code_review/git_repo_model/local.rs) already owns a shared repository watcher and emits debounced status metadata updates. The source-control view should subscribe to that shared model rather than create a second filesystem watcher.
- [`extensions/git/src/historyProvider.ts:249 @ 1224e99`](https://github.com/microsoft/vscode/blob/1224e99c64b0b19b8b71da6bd1ac0dc86b3df224/extensions/git/src/historyProvider.ts#L249) loads history in 50-item pages, preserves refs and parent IDs, and resolves per-commit changes separately.

The portable lesson from VS Code is the bounded history provider model. Warp should not port VS Code's extension-host abstractions or DOM tree implementation.

## Proposed changes

### Feature gate and Tools integration

- Add `FeatureFlag::GitToolsPanel`, enable it for dogfood/local developer builds, and add an optional `git_tools_panel` Cargo feature bridge for targeted local verification.
- Add `ToolPanelView::SourceControl`, `LeftPanelAction::SourceControl`, a persistent mouse state, a `GitBranch` toolbelt icon, focus routing, active styling, and snapshot serialization via `LeftPanelDisplayedTab::SourceControl`.
- Add `workspace/view/source_control/mod.rs` and initialize its view from `LeftPanelView` only when the tab is available.

### View state and repository ownership

- `SourceControlView` is a `TypedActionView` that owns UI-only state: available repositories, selected repository, focused repository, one scroll state, history loading/pagination state, the latest immutable snapshot, and a monotonically increasing request generation.
- `LeftPanelView` forwards `RepositoriesChanged` and `FocusedRepoChanged` events for the active pane group to `SourceControlView`. `set_active_pane_group` seeds it from `WorkingDirectoriesModel` so switching tabs is immediate even before the next event.
- Only `LocalOrRemotePath::Local` values are passed to the Git loader. A selected remote path yields the explicit unsupported state from PRODUCT behavior 8.
- On selection, `SourceControlView` acquires the shared `GitRepoStatusModel` from `GitRepoModels` and subscribes to `MetadataChanged`. The subscription callback schedules a coalesced source-control reload. Switching repositories explicitly unsubscribes the old model before replacing its handle.

### Git data loader

- `source_control/data.rs` contains pure parsers plus async command wrappers.
- Branch/tracking display comes from the shared `GitRepoStatusModel` metadata, avoiding redundant branch/upstream commands.
- History uses `git log --date-order --decorate=full --no-color -n <limit> --skip <skip> --pretty=format:<US-delimited fields><RS> --all`. The parser returns `CommitNode { hash, parents, subject, author, timestamp, refs }` and ignores malformed records rather than panicking.
- Per-commit short statistics are loaded with a parallel bounded `git log --shortstat` call and attached when available.
- Repository snapshots contain only commits, pagination state, and whether `HEAD` exists. Loading does not run diff, index, or untracked-file commands.
- Manual refresh never calls `git fetch`; it reflects local refs only.

### Rendering

- The top bar contains a repository selector (only when multiple repositories exist), branch/tracking summary, and a Warp-themed refresh icon button.
- The error banner remains fixed below the top bar. Commit rows begin immediately below it in one scrollable region with a subtle `surface_3` top border; there is no section header or splitter.
- Icon buttons have explicit sizes through `UiComponentStyles`, so the vertical scroller's unbounded axis cannot produce infinite rectangles.
- Commit lanes use a small custom `Element` because WarpUI has no ready-made DAG element. A pure lane-assignment module produces per-row draw instructions; the element paints theme-compatible thin rectangles and circular nodes. Each bounded page uses the panel's clipped scrollable list.
- Commit rows are single-line and keep ref badges visible. Rich hover cards show author/time, body, refs, short hash, and available insertion/deletion statistics.

### Stale-result and concurrency rules

- Every load captures `(selected_repo, generation)`. The completion applies only if both still match.
- Refresh requests received during a load set a `reload_after_current` bit instead of spawning an unbounded second command set.
- Pagination has its own `history_page_in_progress` guard and appends only if repository/generation/base row count still match.
- Logs include the operation kind and repository path at debug/warn level, but never credentials.

## Testing and validation

- Pure unit tests for PRODUCT 13-15:
  - history records, root/merge parents, decorations (`HEAD`, local, remote, tag), malformed records, pagination append, and lane layouts (linear, branch, merge, multiple roots)
- Shortstat parser tests cover insertion/deletion variants and commits without statistics.
- View/state tests for PRODUCT 4-10 and 17-18 where the existing WarpUI test harness can exercise them without shelling out: repository selection, stale generation rejection, error retention, and focused-repo following.
- Run `./script/format`.
- Run focused unit tests for the source-control module and feature-flag tests.
- Run the app package's Clippy command from `./script/presubmit` with all required features/targets. If the full workspace check is impractical because of unrelated environment failures, record the exact command and failure.
- Manual verification against PRODUCT 1-21:
  1. Launch a dogfood/local build and open the fifth Tools tab in a repository.
  2. Create branch, merge, tag, and remote-tracking history; verify graph labels, hover cards, and Load more.
  3. Confirm the history list fills the panel below the fixed header/error banner with no changes section, History header, or splitter.
  4. Switch tabs, repositories, worktrees, and non-repository directories; verify stale results never cross repositories.

## Parallelization

Parallel sub-agents are not proposed. The active repository instructions prohibit delegation unless the user explicitly requests it, and the main changes are tightly coupled through `LeftPanelView`, `WorkingDirectoriesModel`, snapshot state, and one source-control view. Independent commands such as format, tests, and lint can still run concurrently after implementation.

## Risks and mitigations

- New polling can regress large repositories: subscribe to Warp's shared watcher, debounce/coalesce loads, and never refresh while the tab is inactive unless a current cached snapshot needs invalidation.
- A late async response can show the wrong repository: gate every completion by repository and generation.
- Snapshot enum changes can affect restore: keep existing variants unchanged, add one variant, and cover both directions of the `ToolPanelView` mapping.
