# Git Graph Panel — Technical Spec (as built)

## Scope
A read-only commit DAG visualization tab (`ToolPanelView::GitGraph`) in the left
tools panel. It follows the git repository of the currently active pane, renders
the commit graph, shows a commit's detail on click, supports a manual refresh and
"load more" pagination. No write operations. Gated by `FeatureFlag::GitGraph`.

## Key technical constraints
1. **The render layer only has rectangle-family primitives.** `Scene` in
   `crates/warpui_core/src/scene.rs` provides only `Rect` (with `Border` /
   `CornerRadius` / `DropShadow` / `Dash`), `Image`, `Glyph`, `Icon` — **no
   line / path / bezier, no rotation.** DAG connectors are therefore drawn as
   **orthogonal polylines** (thin vertical/horizontal rects). They are currently
   **square (sharp) corners**; rounded bends are a deferred polish item. This is
   an explicit trade-off against git-graph's bezier curves.
2. **Custom drawing goes through `Element::paint`.** `row_canvas.rs` implements a
   custom `Element` whose `paint` calls `ctx.scene.draw_rect_with_hit_recording`
   and chains `with_background` / `with_corner_radius` (pattern mirrors
   `crates/warpui_core/src/elements/rect.rs:117`). A node dot is a small square
   with `corner_radius = half side` (→ circle).
3. **No git library.** Data is fetched via `run_git_command(repo, args)` at
   `crates/warp_util/src/git.rs:8` (async, shells out to `git`).
4. **`metal` toolchain required to build on macOS.** `warpui/build.rs`
   unconditionally compiles Metal shaders, so a full Xcode + the Metal Toolchain
   component (`xcodebuild -downloadComponent MetalToolchain`) is needed; the app
   crate can't build with Command Line Tools alone.

## Integration points (as wired)
- `app/src/workspace/view/left_panel.rs`
  - `ToolPanelView::GitGraph` + `LeftPanelAction::GitGraph` variants.
  - `git_graph_view: ViewHandle<GitGraphView>` field, built with
    `ctx.add_typed_action_view(GitGraphView::new)`.
  - arms in `create_toolbelt_button_config` (`Icon::GitBranch`, tooltip
    "Git Graph"), the render-body match, `focus_active_view_on_entry`, the second
    focus match, `handle_action_with_force_open`, and `update_button_active_states`.
  - the `WorkingDirectoriesEvent::DirectoriesChanged` subscription pushes the
    most-recent local directory into `git_graph_view.set_working_directory`.
- `app/src/workspace/view.rs`
  - `compute_left_panel_views` pushes `GitGraph` when
    `cfg!(feature="local_fs") && FeatureFlag::GitGraph.is_enabled()`.
  - `LEFT_PANEL_GIT_GRAPH_BINDING_NAME = "workspace:left_panel_git_graph"` (used
    only for the button tooltip; no key bound yet).
  - tooltip-string and snapshot-restore matches updated for the new variant.
- `app/src/app_state.rs`: `LeftPanelDisplayedTab::GitGraph` + the
  `ToolPanelView` ↔ `LeftPanelDisplayedTab` mappings (snapshot persistence).
- Feature flag: cargo feature `git_graph` in `app/Cargo.toml` (not in `default`);
  `FeatureFlag::GitGraph` in `crates/warp_features/src/lib.rs` (+ `DOGFOOD_FLAGS`);
  compile→runtime bridge `#[cfg(feature="git_graph")] FeatureFlag::GitGraph` in
  `app/src/features.rs`.

## Module structure
```
app/src/workspace/view/git_graph/
  mod.rs          declares submodules; re-exports GitGraphView
  data.rs         data types + git-log/show parsing (pure) + async fetch
  layout.rs       pure lane-layout algorithm (assign_lanes)
  row_canvas.rs   GitGraphRowCanvas: custom Element painting one row's lanes
  view.rs         GitGraphView + GitGraphAction + GitGraphEvent
  data_tests.rs   parsing unit tests
  layout_tests.rs lane-layout unit tests
```
No separate model module: state is held directly in `GitGraphView` (single,
unshared view). A `GitGraphModel` would be premature; introduce one only if the
state ever needs to be shared across views.

## Data layer (data.rs)
```
struct CommitNode { hash, short_hash, parents: Vec<String>,
                    author_name, author_email, author_time: i64,
                    subject, refs: Vec<RefLabel> }
enum   RefKind { Head, LocalBranch, RemoteBranch, Tag }
struct RefLabel { kind: RefKind, name: String }

struct ChangedFile { path: String, additions: u32, deletions: u32 }
struct CommitDetail { committer_name: String, committer_time: i64,
                      message: String, files: Vec<ChangedFile> }
```
(There is **no** A/M/D/R status field on `ChangedFile`: numstat alone gives
adds/dels, and a second `--name-status` pass was judged not worth the extra
command for v1.)

Fetch + parse:
- `load_commit_graph(repo, limit, skip)`:
  `git log --all --date-order --decorate=full --no-color -n {limit} --skip {skip}
   --pretty=format:%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e`.
  Fields separated by `%x1f` (Unit Sep), commits by `%x1e` (Record Sep) so
  arbitrary subjects/refs parse reliably. `--decorate=full` makes `%D` emit
  `refs/heads/…` `refs/remotes/…` `refs/tags/…`, which `parse_decorate`
  classifies (and drops remote symbolic `…/HEAD`).
- `load_commit_detail(repo, hash)`: a single
  `git show --numstat --no-color --format=%cn%x1f%ct%x1f%B%x1e {hash}`.
  `parse_commit_detail` splits header (committer name/time + full message `%B`)
  from the numstat block; `parse_numstat` parses `adds\tdels\tpath` (binary `-`
  → 0).
- Pure parsers (`parse_commit_log`, `parse_decorate`, `parse_commit_detail`,
  `parse_numstat`) are unit-tested; the async `load_*` wrappers are thin.

## Lane layout (layout.rs) — core algorithm
Input `&[CommitNode]` (newest → oldest, children before parents). Output:
```
struct PassingLane { col: usize, color_idx: usize }
struct Connection  { col: usize, color_idx: usize }
struct GraphRow {
    node_col: usize,
    node_color: usize,
    node_continues_up: bool,          // node reached via an existing lane (not a tip)
    passing: Vec<PassingLane>,        // lanes passing straight through this row
    to_parents: Vec<Connection>,      // node → each parent column (lower half)
    from_children: Vec<Connection>,   // each merging child → node (upper half)
}
struct GraphLayout { rows: Vec<GraphRow>, max_lanes: usize }
fn assign_lanes(commits: &[CommitNode]) -> GraphLayout
```
Top-down scan maintaining `lanes: Vec<Option<Lane>>` (each lane = expected next
hash + color). Per commit: find incoming lanes (expected == hash); `node_col` =
leftmost incoming, or a fresh leftmost-empty lane for a branch tip
(`node_continues_up = !incoming.is_empty()`). Other incoming lanes collapse into
`from_children`. The first parent continues `node_col` (so a merged branch
visually rejoins the mainline); extra parents open new lanes → `to_parents`.
**No lane compaction**: a lane keeps its column for life, so adjacent rows align
and each row can be painted independently. `color_idx` is the lane's creation
ordinal (monotonic); the renderer takes `% palette_len`.

Test coverage: linear, fork, merge, consecutive/​octopus merges, multi-root,
single/empty, freed-lane reuse.

## Per-row painting (row_canvas.rs)
`GitGraphRowCanvas { row: GraphRow, lane_count }` implements `Element`: fixed
width `lane_count * LANE_WIDTH` (14px), fixed height `ROW_HEIGHT` (22px, matched
by the text column so the `UniformList` rows are uniform). `paint` draws, with
2px-thick rects: `passing` lanes as full-height verticals; `node_continues_up` as
a top→mid vertical at `node_col`; `from_children` as vertical(top→mid at child)
+ horizontal(child→node at mid); `to_parents` as horizontal(node→parent at mid)
+ vertical(mid→bottom at parent); and the commit dot (8px circle) at the node.
Colors come from a fixed 7-entry `PALETTE` (const `ColorU` literals) — theming
via design tokens is deferred.

## View (view.rs) + active-repo resolution
`GitGraphView` holds all state: `working_dir`, `commits: Arc<Vec<CommitNode>>`,
`layout: Arc<GraphLayout>`, `state` (NoRepo/Loading/Loaded/Error), per-row
`MouseStateHandle`s, `selected`, `detail` (None/Loading/Loaded/Error),
two `UniformListState`s (graph + detail file list), refresh/​load-more mouse
states, and `has_more` / `loading_more` for pagination.

`GitGraphAction` = `SelectCommit(usize)` | `Refresh` | `LoadMore`, handled by the
`TypedActionView` impl. Rows are wrapped in `Hoverable` and dispatch
`SelectCommit`. Layout per state:
- header (when a dir is set): "{N} commits" / status + a refresh `icon_button`.
- body: a single `MainAxisSize::Max` column with `Shrinkable` factors — list
  alone (factor 1), or list (2) + detail (1) when a commit is selected. (Nesting
  two `Max` columns feeds the inner one an infinite constraint and panics — the
  view deliberately uses one column.)
- the graph list is a `UniformList`; when `has_more`, a trailing "Load more" row
  dispatches `LoadMore` (button-style pagination, not infinite-scroll).
- the detail area shows full message, author (name+email), committer (if
  different), full hash, and a virtualized changed-file list with `+adds/-dels`.
  Timestamps are not yet formatted/shown.

Each commit row = `[GitGraphRowCanvas | short hash + ref badges + subject]`.
Ref labels render as small rounded color-coded badges (HEAD/local/remote/tag).

**Active-repo resolution**: `LeftPanelView`'s existing
`WorkingDirectoriesEvent::DirectoriesChanged` handler already computes the active
pane group's local directories; it pushes the most-recent one into
`git_graph_view.set_working_directory`. `git log` resolves the repository from
any subdirectory, so no explicit "directory → repo root" lookup is needed; a
non-repo directory simply yields the NoRepo state.

## Testing
- `layout_tests.rs`: assert `assign_lanes` `node_col` / `node_continues_up` /
  passing / to_parents / from_children / `max_lanes` across DAG shapes.
- `data_tests.rs`: `parse_commit_log` / `parse_decorate` / `parse_commit_detail`
  edge cases (separators in subject, empty `%D`, detached HEAD, multiple parents,
  binary numstat, etc.).
- 22 unit tests total. Build/run verified manually under
  `--features local_fs,git_graph` (no integration test added; core logic is in
  the pure-function unit tests).

## Deferred / not yet implemented
- **Auto-refresh on repo change.** Needs a `RepoMetadataModel` handle threaded
  into the view, subscription to `RepoMetadataEvent::RepositoryUpdated` filtered
  by repo id, debounce, and large-repo perf care. Manual refresh covers v1.
- **Rounded elbow connectors** (cosmetic), **theme-token palette** (current fixed
  palette suits dark themes), **keybinding** for the tab, **telemetry**, and a
  `show_git_graph` user setting — all deliberately out of scope for now.
- **Changed-file A/M/D/R status** and **formatted commit timestamps** in detail.

## Implementation phases (delivered)
- **Phase 1** — data + layout + unit tests (no UI).
- **Phase 2** — feature flag + `ToolPanelView` wiring + plain-text commit list.
- **Phase 3** — lane graph rendering (`row_canvas`).
- **Phase 4a** — click → commit detail (+ startup flex-nesting crash fix).
- **Phase 4b** — manual refresh button.
- **Phase 4c** — "load more" pagination.
- **Phase 5 (partial)** — ref-label color badges. Remaining polish deferred (see
  above).
