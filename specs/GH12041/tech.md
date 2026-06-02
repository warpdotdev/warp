# Git Graph Panel — Technical Spec (as built)

## Context

A commit DAG visualization tab (`ToolPanelView::GitGraph`) in the left tools
panel. It follows the git repository of the currently active pane, renders the
commit graph, shows a commit's detail on click, opens a read-only file diff in
the main area, supports a repository picker, branch filtering, manual refresh,
and "load more" pagination. Browsing is read-only; a separately flag-gated layer
(`FeatureFlag::GitGraphWrite`) adds right-click context-menu write operations.
Gated by `FeatureFlag::GitGraph` plus the `show_git_graph` user setting.

### Key technical constraints
1. **The render layer only has rectangle-family primitives.** `Scene` in
   `crates/warpui_core/src/scene.rs` provides only `Rect` (with `Border` /
   `CornerRadius` / `DropShadow` / `Dash`), `Image`, `Glyph`, `Icon` — **no
   line / path / bezier, no rotation.** DAG connectors are therefore drawn as
   **orthogonal polylines** (thin vertical/horizontal rects) with square corners;
   rounded bends are deferred. This is an explicit trade-off vs git-graph's beziers.
2. **Custom drawing goes through `Element::paint`.** `row_canvas.rs` implements a
   custom `Element` whose `paint` calls `ctx.scene.draw_rect_with_hit_recording`
   (pattern mirrors `crates/warpui_core/src/elements/rect.rs`). A node dot is a
   small square with `corner_radius = half side` (→ circle).
3. **No git library.** Data is fetched by shelling out to `git` (async).
4. **`metal` toolchain required to build on macOS** (`warpui/build.rs`
   unconditionally compiles Metal shaders).

## Implementation

### Module structure
```
app/src/workspace/view/git_graph/
  mod.rs          declares submodules; re-exports GitGraphView
  data.rs         data types + git log/show/branch/diff parsing (pure) + async fetch
  ops.rs          write-op layer: GitWriteOp + pure arg-builders + async runners
  menu.rs         pure right-click context-menu builders (MenuKind / PromptKind)
  layout.rs       pure lane-layout algorithm (assign_lanes)
  row_canvas.rs   GitGraphRowCanvas: custom Element painting one row's lanes
  view.rs         GitGraphView + GitGraphAction + GitGraphEvent
  data_tests.rs / layout_tests.rs / ops_tests.rs / menu_tests.rs   unit tests
app/src/code/commit_diff_view.rs           read-only commit-file diff view
app/src/pane_group/pane/commit_diff_pane.rs host pane for the diff view
app/src/settings/git.rs                    GitSettings (show_git_graph, scan depth)
app/src/settings_view/git_page.rs          "Git" settings page UI
```
State is held directly in `GitGraphView` (single, unshared view); no separate
model module — a `GitGraphModel` would be premature.

### Data layer (data.rs)
```
struct CommitNode  { hash, short_hash, parents: Vec<String>, author_name,
                     author_email, author_time: i64, subject, refs: Vec<RefLabel> }
enum   RefKind     { Head, LocalBranch, RemoteBranch, Tag }
struct RefLabel    { kind: RefKind, name: String }
struct BranchRef   { full ref + display name + kind }       // branch-filter list
struct ChangedFile { path: String, additions: u32, deletions: u32 }
struct CommitDetail{ committer_name, committer_time: i64, message, files }
struct CommitFileDiff { base_content: String, hunks: Vec<DiffHunk> } // file diff
```
Async fetch + pure parsers (unit-tested; `load_*` wrappers are thin):
- `discover_repositories(anchor, depth)` — scans the anchor dir and (down to
  `depth` levels) finds git repo roots for the repository picker.
- `load_branches(dir)` — local + remote branch refs for the branch filter.
- `load_commit_graph(dir, filter, limit, skip)` — `git log --all --date-order
  --decorate=full` with `%x1f`/`%x1e` separators; `filter` restricts to the
  selected branches; `limit`/`skip` drive pagination.
- `load_commit_detail(dir, hash)` — `git show --numstat` → committer + message +
  changed files.
- `load_file_diff_at_commit(dir, hash, path)` — the file's parent-commit content
  + unified `DiffHunk`s for the read-only diff pane (compares against first
  parent; root commit falls back to whole-file additions).

### Lane layout (layout.rs) — core algorithm
Input `&[CommitNode]` (newest→oldest). Output `GraphLayout { rows, max_lanes }`
where each `GraphRow` carries `node_col`, `node_color`, `node_continues_up`,
`passing`, `to_parents`, `from_children`. Top-down scan maintaining
`lanes: Vec<Option<Lane>>`; first parent continues the node's column (a merged
branch visually rejoins the mainline), extra parents open new lanes. **No lane
compaction** — a lane keeps its column for life, so adjacent rows align and each
row paints independently. Covered for linear / fork / merge / octopus /
multi-root / freed-lane-reuse shapes.

### Per-row painting (row_canvas.rs)
`GitGraphRowCanvas { row, lane_count }` implements `Element`: width
`lane_count * LANE_WIDTH`, fixed `ROW_HEIGHT`. `paint` draws 2px rects —
`passing` verticals, `node_continues_up` top→mid, `from_children` (vertical +
horizontal elbow), `to_parents` (horizontal + vertical elbow), and the commit
dot. Colors from a fixed 7-entry `PALETTE`.

### View (view.rs)
`GitGraphView` holds all state: `scan_anchor`, `repositories` + `selected_repo` +
`repo_dropdown`, `branches` + `selected_branches` + `saved_branch_selections` +
branch-filter overlay state, `commits` + `layout` + `state`
(NoRepo/Loading/Loaded/Error), `selected` + `detail`, list/detail scroll states,
a draggable detail-area height (`ResizableState`), and `has_more`/`loading_more`.
`GitGraphAction` = `SelectCommit` | `SelectRepository` | `Refresh` | `LoadMore` |
branch-filter toggles | `OpenFileDiff(idx)` | the write/menu actions listed in
"Write operations" above. Layout: header (commit count +
repository picker when >1 repo + branch-filter button + refresh) over a single
column with `Shrinkable` factors (graph list alone, or list + detail when a
commit is selected). The detail area (message + author/committer + full hash +
changed files) is one `ClippedScrollable`; its height is user-draggable.

**Active-repo resolution**: `LeftPanelView`'s
`WorkingDirectoriesEvent::DirectoriesChanged` handler pushes the most-recent
local directory into `GitGraphView::set_working_directory`, which re-runs
`discover_repositories`. `git log` resolves the repo from any subdirectory.

### Read-only file diff pane
Clicking a changed file dispatches `GitGraphAction::OpenFileDiff(idx)`; the view
emits `GitGraphEvent::OpenCommitFileDiff { repo_relative_path, short_hash,
base_content, hunks }`, forwarded up by the left panel to the workspace, which
builds a `CommitDiffView` (`app/src/code/commit_diff_view.rs`) hosted in a
`CommitDiffPane`. The diff renders through the **existing code-review diff
machinery** — `hunks` are converted via
`code_review::diff_state::convert_hunks_to_diff_deltas` and shown in a
`CodeEditorView`, so commit-file diffs reuse the same editor/diff rendering.
Re-clicking another file reuses the first visible commit-diff pane (updates its
content in place) instead of opening a new one. The pane is non-restorable
(`source: None`) so a historical revision is never written back to the working
tree.

### Write operations (ops.rs / menu.rs / view dialogs)
Gated at the UI layer by `FeatureFlag::GitGraphWrite`; the git layer is the same
`warp_util::git::run_git_command` shell-out used by the read-only fetches (no git
library), so write operations add no new IO mechanism.

- **`ops.rs`** — `GitWriteOp` enumerates every ready-to-run mutating action
  (AddTag / CreateBranch / CheckoutCommit / CherryPick / Revert / DropCommit /
  Merge / Rebase / Reset{mode} / CheckoutBranch / DeleteRemoteBranch / Pull /
  RenameBranch / PushBranch / DeleteTag / PushTag / Archive). `GitWriteOp::args`
  (the exact `git` argv) and `confirm_message` are **pure** (unit-tested);
  `run_write_op` is a thin async wrapper. Helpers:
  `split_remote_ref` (`origin/x` → remote+branch), `archive_format_from_path`
  (extension → zip/tar.gz). Notable argv: Drop = `rebase --onto <h>^ <h>`,
  DeleteRemoteBranch = `push <remote> --delete <branch>`.
- **`menu.rs`** — pure builders turning a `MenuKind` (Commit / ShortHash / Tag /
  RemoteBranch / LocalBranch, each carrying the anchor row index; LocalBranch also
  carries `is_current`, set from the HEAD vs LocalBranch badge, to drop self-only
  ops on the current branch; ShortHash yields a single read-only "Copy Short Hash
  to Clipboard" for the 7-char hash) + the anchor commit + `write_enabled` into
  `Vec<MenuItem<GitGraphAction>>`, grouped to match the
  screenshots (groups joined by separators, empty groups dropped). Read-only
  items (copy / view-details / unselect) are unconditional; mutating items are
  added only when `write_enabled` (the "…current branch" items are always offered
  when writing — git handles a detached HEAD, the menu does not special-case it).
  `PromptKind` (AddTag / CreateBranch / RenameBranch) carries the dialog title +
  initial text and `into_op(text)` builds the final `GitWriteOp`.
- **view.rs wiring** — `GitGraphView` holds a shared `Menu<GitGraphAction>` child
  view (`context_menu`, with `prevent_interaction_with_other_elements` like the
  Project Explorer, so a click/right-click outside the open menu dismisses it
  rather than switching; `open_menu` is cleared on the menu's `Close` event), the
  open target (`open_menu` + `menu_offset`), a `DialogState` (None / Input /
  Confirm / ResetMode) with a single-line `EditorView` (`dialog_input`), and
  `op_running` / `op_error`. Right-click is wired on the commit row (→
  `MenuKind::Commit`), on the short-hash text (→ `MenuKind::ShortHash`), and on
  each ref badge (→ Tag / RemoteBranch / LocalBranch); the short-hash and badge
  handlers sit above the row so they take precedence; all compute the offset
  relative to the panel's `SavePosition` id. New
  `GitGraphAction`s: `OpenMenu` / `CopyToClipboard` / `PromptInput` / `SubmitInput`
  / `PromptResetMode` / `BeginWriteOp` (confirm-then-run) / `RunOp` (run now) /
  `BeginArchive` (OS save dialog) / `CancelDialog` / `DismissOpError`. `run_op`
  guards re-entrancy, spawns `run_write_op`, and on completion reloads — push
  included, since it updates the local remote-tracking ref — (or `refresh`es with
  `fetch --prune` for a remote-branch deletion; nothing only for an archive,
  which writes a file and changes no ref) or fills the error banner via
  `clean_git_error`.

### Settings (`Settings → Git`)
`GitSettings` (`app/src/settings/git.rs`, via `define_settings_group!`):
- `show_git_graph: bool` (default true; toml `git.show_graph_panel`) — gates the
  toolbelt tab when `FeatureFlag::GitGraph` is on.
- `git_graph_scan_depth: u32` (default 1; toml `git.graph_scan_depth`) — how many
  directory levels below the working directory `discover_repositories` probes.
The "Git" settings page (`app/src/settings_view/git_page.rs`) surfaces both.

### Integration points (wiring)
- `left_panel.rs`: `ToolPanelView::GitGraph` + `LeftPanelAction::GitGraph`,
  `git_graph_view` field, toolbelt button (`Icon::GitBranch`), working-directory
  subscription, and `LeftPanelEvent::OpenCommitFileDiff` forwarding.
- `workspace/view.rs`: `compute_left_panel_views` adds `GitGraph` when
  `cfg!(feature="local_fs") && FeatureFlag::GitGraph.is_enabled()`; builds the
  `CommitDiffView` on the forwarded event.
- `app_state.rs`: `LeftPanelDisplayedTab::GitGraph` snapshot mapping.
- Feature flags: cargo feature `git_graph` (`app/Cargo.toml`, not default);
  `FeatureFlag::GitGraph` (the panel) and `FeatureFlag::GitGraphWrite` (the
  right-click write layer) — both in `crates/warp_features/src/lib.rs` +
  `DOGFOOD_FLAGS`, both bridged from the `git_graph` cargo feature in
  `app/src/features.rs`. Keeping the write flag separate lets the read-only base
  ship without the mutating layer.
- `crates/warpui_core/src/elements/resizable.rs`: `dragbar_hover_color` support
  used by the draggable detail-area splitter.

## Testing and validation
- **Unit tests** (`data_tests.rs` / `layout_tests.rs`): `assign_lanes` across DAG
  shapes (invariants 4–5); `parse_commit_log` / `parse_decorate` /
  `parse_commit_detail` / `parse_numstat` / repo discovery edge cases
  (invariants 6, 8, 10).
- **Write-layer unit tests** (`ops_tests.rs` / `menu_tests.rs`): every
  `GitWriteOp::args` argv (reset modes, `push --delete`, archive format/path,
  rename, annotated vs lightweight tag), `confirm_message` presence, the path/ref
  helpers, and each menu's item set + order across write-on / read-only /
  detached-HEAD (invariants 19–22).
- **Integration tests** (`crates/integration/src/test/git_graph.rs`):
  `test_git_graph_loads_commits` (read path: real repo → panel → graph loads,
  invariants 1–4) and `test_git_graph_create_branch` (write path: runs the
  "Create Branch" op at the top commit, then asserts the branch appears after the
  reload, invariants 19/23). Drive the panel via `pub(crate)` test accessors on
  `GitGraphView` / `LeftPanelView` / `WorkspaceView` and helpers in
  `app/src/integration_testing/git_graph.rs`.
- Manual verification under `--features local_fs,git_graph`.

## Non-goals (deferred)
- Stash operations; in-app resolution of merge/rebase/cherry-pick conflicts
  (finished in the terminal); per-branch push-remote resolution (push uses
  `origin`).
- Auto-refresh on repo change — needs repo-watcher plumbing + debounce; manual
  refresh covers it.
- Rounded (bezier) connectors — render-layer limitation; orthogonal corners today.
- In-graph commit search.
- Per-file A/M/D/R status and formatted commit timestamps in the detail area.
- Theme-token colors — fixed palette today.
