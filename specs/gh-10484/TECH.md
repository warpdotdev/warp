# Git source control in the Tools panel: technical design

## Context

This implements the behavior in [PRODUCT.md](./PRODUCT.md) on top of Warp's existing Tools-panel, repository-discovery, Git-status, and Code Review infrastructure. The inspected Warp revision is `a0d589460b58bc8ad2cd4d6795339fdef79ed0bd`; the inspected VS Code revision is `1224e99c64b0b19b8b71da6bd1ac0dc86b3df224`.

- [`app/src/workspace/view/left_panel.rs @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/workspace/view/left_panel.rs) owns the Tools tab enum, icon row, active pane-group wiring, focus, and tab content.
- [`app/src/workspace/view.rs:23667 @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/workspace/view.rs#L23667) computes the available Tools tabs; it is the runtime feature-flag gate.
- [`app/src/pane_group/working_directories.rs:229 @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/pane_group/working_directories.rs#L229) publishes per-pane-group repository and focused-repository changes, including worktrees and multiple repositories.
- [`app/src/code_review/git_repo_model/local.rs @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/code_review/git_repo_model/local.rs) already owns a shared repository watcher and emits debounced status metadata updates. The source-control view should subscribe to that shared model rather than create a second filesystem watcher.
- [`app/src/code_review/diff_state/mod.rs @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/code_review/diff_state/mod.rs) and [`app/src/code_review/code_review_view.rs @ a0d589460`](https://github.com/warpdotdev/warp/blob/a0d589460b58bc8ad2cd4d6795339fdef79ed0bd/app/src/code_review/code_review_view.rs) remain the owners of rich diff rendering, discard confirmation, commit, push, and create-PR flows.
- [`src/vs/workbench/contrib/scm/browser/scmViewPane.ts @ 1224e99`](https://github.com/microsoft/vscode/blob/1224e99c64b0b19b8b71da6bd1ac0dc86b3df224/src/vs/workbench/contrib/scm/browser/scmViewPane.ts) models the VS Code change list as repositories, resource groups, resources, row actions, and a virtualized tree.
- [`extensions/git/src/repository.ts @ 1224e99`](https://github.com/microsoft/vscode/blob/1224e99c64b0b19b8b71da6bd1ac0dc86b3df224/extensions/git/src/repository.ts) keeps merge, index, and working-tree resource groups distinct and refreshes them from Git status.
- [`extensions/git/src/historyProvider.ts:249 @ 1224e99`](https://github.com/microsoft/vscode/blob/1224e99c64b0b19b8b71da6bd1ac0dc86b3df224/extensions/git/src/historyProvider.ts#L249) loads history in 50-item pages, preserves refs and parent IDs, and resolves per-commit changes separately.

The portable lesson from VS Code is the separation of provider data from view grouping and actions. Warp should not port VS Code's extension-host abstractions or DOM tree implementation.

## Proposed changes

### Feature gate and Tools integration

- Add `FeatureFlag::GitToolsPanel`, enable it for dogfood/local developer builds, and add an optional `git_tools_panel` Cargo feature bridge for targeted local verification.
- Add `ToolPanelView::SourceControl`, `LeftPanelAction::SourceControl`, a persistent mouse state, a `GitBranch` toolbelt icon, focus routing, active styling, and snapshot serialization via `LeftPanelDisplayedTab::SourceControl`.
- Add `workspace/view/source_control/mod.rs` and initialize its view from `LeftPanelView` only when the tab is available.

### View state and repository ownership

- `SourceControlView` is a `TypedActionView` that owns UI-only state: available repositories, selected repository, focused repository, collapse state, scroll state, load/mutation state, the latest immutable snapshot, and a monotonically increasing request generation.
- `LeftPanelView` forwards `RepositoriesChanged` and `FocusedRepoChanged` events for the active pane group to `SourceControlView`. `set_active_pane_group` seeds it from `WorkingDirectoriesModel` so switching tabs is immediate even before the next event.
- Only `LocalOrRemotePath::Local` values are passed to the Git loader. A selected remote path yields the explicit unsupported state from PRODUCT behavior 8.
- On selection, `SourceControlView` acquires the shared `GitRepoStatusModel` from `GitRepoModels` and subscribes to `MetadataChanged`. The subscription callback schedules a coalesced source-control reload. Switching repositories explicitly unsubscribes the old model before replacing its handle.

### Git data loader

- `source_control/data.rs` contains pure parsers plus async command wrappers.
- Working-tree groups are loaded concurrently from NUL-delimited Git output:
  - staged: `git diff --cached --name-status -z`
  - unstaged/conflicted: `git diff --name-status -z`
  - untracked: `git ls-files --others --exclude-standard -z`
- `parse_name_status_z` maps status records to `GitChangeKind`, preserving old/new paths for rename/copy records. The merge group takes conflicted entries; duplicate paths are allowed across staged and unstaged groups by design.
- Branch/tracking display comes from the shared `GitRepoStatusModel` metadata, avoiding redundant branch/upstream commands.
- History uses `git log --date-order --decorate=full --no-color -n <limit> --skip <skip> --pretty=format:<US-delimited fields><RS> --all`. The parser returns `CommitNode { hash, parents, subject, author, timestamp, refs }` and ignores malformed records rather than panicking.
- Manual refresh never calls `git fetch`; it reflects local refs only.

### Git mutations

- Add small command wrappers in `data.rs`:
  - stage path: `git add -- <path>`
  - stage all: `git add -A`
  - unstage path/all with `git reset -q HEAD -- ...` when `HEAD` exists
  - unborn-branch unstage with `git rm --cached -q -- ...`
- Every mutation is spawned off the UI thread. A single `mutation_in_progress` guard disables other mutation actions. Completion increments the request generation and performs a full reload; errors retain the last successful snapshot and show an operation banner.
- The panel does not expose discard/reset. File selection emits `OpenCodeReview { repo_path }`; `Workspace` opens its existing Code Review surface, which owns discard confirmations and commit actions.

### Rendering

- The top bar contains a repository selector (only when multiple repositories exist), branch/tracking summary, and a Warp-themed refresh icon button.
- The changes area renders collapsible sections and compact file rows using existing `Container`, `Flex`, `Text`, `Hoverable`, `Shrinkable`, icon-button, tooltip, and theme abstractions. Row-level actions use the existing Naked button theme or icon-button abstraction without changing shared themes.
- Bare warpui icons expand to their maximum layout constraint, which is unbounded inside a flex row. Every icon in the panel is therefore given an explicit size (icon buttons via `UiComponentStyles`, chevrons via a fixed-size `ConstrainedBox`), so the vertical scroller's unbounded axis can never produce infinite rectangles.
- The history area is separated by a draggable or proportional split only if an existing stable split primitive fits without cross-panel changes; otherwise changes and history share one vertical scroll surface with independent collapsible headers. The simpler shared scroll surface is the default.
- Commit lanes use a small custom `Element` because WarpUI has no ready-made DAG element. A pure lane-assignment module produces per-row draw instructions; the element paints theme-compatible thin rectangles and circular nodes. History is paginated and rendered with `UniformList` when the combined structure permits stable row heights; otherwise each bounded page uses a clipped scrollable list.

### Stale-result and concurrency rules

- Every load captures `(selected_repo, generation)`. The completion applies only if both still match.
- Starting a manual refresh aborts or supersedes the previous load. Watcher notifications received during a load set a `reload_after_current` bit instead of spawning an unbounded second command set.
- Pagination has its own `history_page_in_progress` guard and appends only if repository/generation/base row count still match.
- Logs include the operation kind and repository path at debug/warn level, but not Git stdout containing paths when it may be sensitive and never credentials.

## Testing and validation

- Pure unit tests for PRODUCT 12-16 and 18-21:
  - normal, deleted, added, conflicted, rename/copy, empty, malformed, unusual whitespace, and NUL-delimited status records
  - a path appearing in both staged and unstaged results
  - unborn-branch command selection
- Pure unit tests for PRODUCT 23-25:
  - history records, root/merge parents, decorations (`HEAD`, local, remote, tag), malformed records, pagination append, and lane layouts (linear, branch, merge, multiple roots)
- View/state tests for PRODUCT 4-10 and 27-29 where the existing WarpUI test harness can exercise them without shelling out: repository selection, stale generation rejection, collapse-state preservation, error retention, and focused-repo following.
- Run `./script/format`.
- Run focused unit tests for the source-control module and feature-flag tests.
- Run the app package's Clippy command from `./script/presubmit` with all required features/targets. If the full workspace check is impractical because of unrelated environment failures, record the exact command and failure.
- Manual verification against PRODUCT 1-7, 11-32:
  1. Launch a dogfood/local build and open the fifth Tools tab in a repository.
  2. Create modified, staged, untracked, deleted, renamed, and conflicted examples.
  3. Exercise per-file and bulk stage/unstage, including a file with both staged and unstaged edits.
  4. Confirm Code Review opens from a file row and retains its discard/commit workflows.
  5. Create branch, merge, tag, and remote-tracking history; verify graph labels and Load more.
  6. Switch tabs, repositories, worktrees, and non-repository directories; verify stale results never cross repositories.

## Parallelization

Parallel sub-agents are not proposed. The active repository instructions prohibit delegation unless the user explicitly requests it, and the main changes are tightly coupled through `LeftPanelView`, `WorkingDirectoriesModel`, snapshot state, and one source-control view. Independent commands such as format, tests, and lint can still run concurrently after implementation.

## Risks and mitigations

- Git status parsing can corrupt rename or whitespace-heavy paths: use `-z` output and pure parser coverage.
- New polling can regress large repositories: subscribe to Warp's shared watcher, debounce/coalesce loads, and never refresh while the tab is inactive unless a current cached snapshot needs invalidation.
- A late async response can show the wrong repository: gate every completion by repository and generation.
- Staging is state-mutating: use exact path arguments after `--`, serialize mutations, retain errors, and never expose discard without confirmation.
- Snapshot enum changes can affect restore: keep existing variants unchanged, add one variant, and cover both directions of the `ToolPanelView` mapping.
