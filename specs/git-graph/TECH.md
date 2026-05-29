# Git Graph Panel — Technical Spec

## Scope
Add a read-only commit DAG visualization tab (`ToolPanelView::GitGraph`) to the left tools panel. It follows the git repository of the currently active pane, renders the commit graph, and shows commit details on click. v1 contains no write operations. The whole panel is gated by `FeatureFlag::GitGraph`.

## Key technical constraints (read first)
1. **The render layer only has rectangle-family primitives.** `Scene` in `crates/warpui_core/src/scene.rs` provides only `Rect` (with `Border` / `CornerRadius` / `DropShadow` / `Dash`), `Image`, `Glyph`, and `Icon` — **no line / path / bezier, and no rotation.**
   - Consequence: DAG connectors are drawn as **orthogonal elbow polylines** — vertical line = thin rect; horizontal hop = thin rect; corner = a rect with `CornerRadius` to round the bend. **It cannot reproduce git-graph's bezier curves**; visually it is the straight rounded-elbow style of lazygit/tig. This is an explicit trade-off against the reference.
2. **Custom drawing goes through `Element::paint`.** `Rect::paint` at `crates/warpui_core/src/elements/rect.rs:117` demonstrates how a custom element draws in `paint(&mut self, origin, ctx, app)` by calling `ctx.scene.draw_rect_with_hit_recording(RectF)` and chaining `with_background / with_border / with_corner_radius`. The Git Graph lane cell is exactly such a custom `Element`. A node dot = a small square with `corner_radius` set to half its side length.
3. **No git library.** The repo does not depend on `git2`/`gix`. All data is fetched via `run_git_command(repo_path, args) -> Result<String>` at `crates/warp_util/src/git.rs:8` (async, shells out to `git`).

## Relevant code (integration points)
- `app/src/workspace/view/left_panel.rs`
  - `enum ToolPanelView` (100-106) — add `GitGraph` variant
  - `enum LeftPanelAction` (74-79) — add `GitGraph`
  - `struct LeftPanelView` (167-180) — add field `git_graph_view: ViewHandle<GitGraphView>`
  - `LeftPanelView::new` (196+) — construct `git_graph_view`
  - `create_toolbelt_button_config` (391) — add `GitGraph` arm (`Icon::GitBranch`, tooltip "Git Graph")
  - the render-body `match self.active_view.get()` (1147-1184) — add an arm rendering `git_graph_view`
  - `focus_active_view_on_entry` (684-721) — add an arm
  - tab active-state logic (around 863-870, the `is_*_active` / `update_button_active_states`) — add `is_git_graph_active`
- `app/src/workspace/view.rs`
  - `compute_left_panel_views` (21218) — push when `cfg!(feature="local_fs") && FeatureFlag::GitGraph.is_enabled()`
  - left-panel keybinding constants (around 609-613) — add `LEFT_PANEL_GIT_GRAPH_BINDING_NAME = "workspace:left_panel_git_graph"`
  - `left_panel_view` construction (2774-2779) — no change; reuses `compute_left_panel_views`
- `crates/warp_features/src/lib.rs`
  - `enum FeatureFlag` (cf. `GlobalSearch` @526 and its description @1039) — add `GitGraph` + description + default Dev stage
- Reusable assets
  - `Icon::GitBranch` (`crates/warp_core/src/ui/icons.rs:177`), `Icon::GitCommit` (310) — already exist, no new svg needed
  - `run_git_command` (`crates/warp_util/src/git.rs:8`)
  - `Repository` / repo resolution (`crates/repo_metadata/src/repository.rs:59`) — used to resolve "active pane's working directory" into "repository root"

## New module structure
Following the `global_search` subtree, the feature lives under the view tree for cohesion and low coupling:

```
app/src/workspace/view/git_graph/
  mod.rs          re-export GitGraphView / GitGraphEvent
  data.rs         data types + async fetch (calls warp_util::git)
  layout.rs       pure lane-layout algorithm + lane palette
  layout_tests.rs unit tests for layout (the core test surface)
  data_tests.rs   table-driven tests for parsing git log output
  model.rs        GitGraphModel (state machine + async loading + refresh subscription)
  view.rs         GitGraphView (renders header / list / detail; subscribes to model and repo changes)
  row_canvas.rs   GitGraphRowCanvas: custom Element that paints lanes/dots/connectors per row
```

## Data layer (data.rs)

### Types
```
struct CommitNode { hash, short_hash, parents: Vec<String>,
                    author_name, author_email, author_time: i64,
                    subject, refs: Vec<RefLabel> }
enum RefKind { Head, LocalBranch, RemoteBranch, Tag }
struct RefLabel { kind: RefKind, name: String }

struct ChangedFile { status: char /*A/M/D/R*/, path: String, additions: u32, deletions: u32 }
struct CommitDetail { committer_name, committer_time: i64, body: String, files: Vec<ChangedFile> }
```

### Fetch functions (responsibilities, not implementations)
- `async fn load_commit_graph(repo_root, limit, skip) -> Result<Vec<CommitNode>>`
  - Command: `git log --all --date-order --no-color -n {limit} --skip {skip} --pretty=format:%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e`
  - Uses the unit separator `%x1f` between fields and the record separator `%x1e` between commits, so subjects/refs containing arbitrary characters still parse reliably.
  - `%P` gives parents (space-separated; 0 = root, 2+ = merge); `%D` gives the decorate string (e.g. `HEAD -> main, origin/main, tag: v1`), which the parser splits into `RefLabel`s.
- `async fn load_commit_detail(repo_root, hash) -> Result<CommitDetail>`
  - One call for committer + body: `git show --no-patch --no-color --pretty=format:...{hash}`
  - One call for file changes: `git show --numstat --no-color --format= {hash}` (each line `add\tdel\tpath`), plus a `--name-status` pass for the status letter, or derive rename from `--numstat`'s first column.
- The parsers `parse_commit_log(stdout) -> Vec<CommitNode>`, `parse_decorate(d) -> Vec<RefLabel>`, `parse_numstat(...)` are pure functions, tested separately.

## Lane layout (layout.rs) — core algorithm

Input: `&[CommitNode]` (git log order, newest → oldest, children before their parents). Output:
```
struct GraphRow { node_col: usize, segments: Vec<LaneSegment> }
struct LaneSegment { from_col: usize, to_col: usize, color_idx: usize, kind: SegmentKind }
enum SegmentKind { ThroughVertical, /* row has a commit node */ NodeIn, BranchOut, MergeIn }
struct GraphLayout { rows: Vec<GraphRow>, max_lanes: usize }
fn assign_lanes(commits: &[CommitNode]) -> GraphLayout
```

Algorithm (top-down scan, maintaining `active_lanes: Vec<Option<expected_hash>>`, where each lane records the commit hash it expects to reach on the next row):
1. Find all lanes with `expected_hash == current commit.hash`; the commit's `node_col` = the leftmost such lane (if none, it's a branch tip — open a new leftmost empty lane).
2. The commit "consumes" those lanes: set lane `node_col`'s expected hash to the **first parent**; any other lanes pointing at this commit (multiple children merging in) collapse on this row, draw a `MergeIn` segment, and are freed.
3. Multiple parents (merge commit): parents 2..n each open a new lane (leftmost free slot), drawing a `BranchOut` segment.
4. Any lane not touched by this commit continues as-is, drawing a `ThroughVertical` segment.
5. `color_idx = lane_index % PALETTE_LEN`; palette comes from theme tokens (see below).

Shapes to cover (test cases): purely linear, single fork, single merge, consecutive merges, octopus merge (3+ parents), multi-root repository (several parentless commits), isolated branch tip.

## Per-row painting (row_canvas.rs)

`GitGraphRowCanvas` implements `Element`:
- Inputs: this row's `GraphRow` + lane width `LANE_W` (~14px) + row height + palette.
- `paint()`: for each `LaneSegment`,
  - `ThroughVertical`: draw a 2px-wide vertical rect spanning the row height, centered in column `from_col`.
  - `NodeIn` / `BranchOut` / `MergeIn` crossing columns: draw an orthogonal elbow polyline of "vertical segment + horizontal segment + rounded-corner rect" (use `CornerRadius` to round the bend).
  - Commit dot: at `node_col`, row midpoint, draw a small square with `corner_radius = half side` (= circle), colored by the lane; a HEAD commit may add a `Border` outline.
- For click selection, the node dot uses `draw_rect_with_hit_recording` to record a hit region; click events bubble to `GitGraphView` → `model.select(index)`.
- **Each row paints itself** (not one full-height canvas), which aligns naturally with the existing scrollable list / scrollbar elements and avoids global scroll-offset math.

## Model (model.rs)

`GitGraphModel` (warpui Model) fields: `repo_root: Option<PathBuf>`, `commits: Vec<CommitNode>`, `layout: GraphLayout`, `selected: Option<usize>`, `detail: Option<CommitDetail>`, `state: { NoRepo | Loading | Loaded | Error(String) }`, pagination cursor `loaded_count`.

Methods (responsibilities):
- `set_repo(root, ctx)`: called when the active repository changes; reset and `reload`.
- `reload(ctx)`: spawn an async task calling `load_commit_graph` → back on the main thread run `assign_lanes` → update state + `ctx.notify()`.
- `load_more(ctx)`: paginated append.
- `select(index, ctx)`: spawn `load_commit_detail` → fill `detail`.
- Subscribe to repo changes for auto-refresh: reuse `repo_metadata`'s repository watcher events (`crates/repo_metadata`), debounce refreshes (~300ms) so that bursts of file events don't repeatedly re-run `git log`.

## View (view.rs) + active-repo resolution

`GitGraphView` (warpui View):
- Header: repo name + current branch + refresh button (`icon_button`, reusing `app/src/ui_components/buttons`).
- Body: scrollable list, each row = `[GitGraphRowCanvas | ref labels + subject + author + relative time + short hash]`.
- Detail area (bottom/side): renders `model.detail`.
- Subscribes to `GitGraphModel` changes → `ctx.notify()` to repaint.

**Active-repo resolution** (follow the focused pane's repository): reuse the same path the file tree uses to locate "the current repo." `LeftPanelView` already holds `working_directories_model: ModelHandle<WorkingDirectoriesModel>` and `active_pane_group` (see `left_panel.rs:174-178`), and the file tree resolves the active working directory per pane group via `active_file_tree_view`. Git Graph follows suit: subscribe to `WorkingDirectoriesEvent::DirectoriesChanged` (already subscribed around `left_panel.rs:256`) + active-pane-group changes, resolve the active working directory to a repository root via `repo_metadata` repository lookup, and call `model.set_repo`.
> Note: the exact "working directory → repository root" API is confirmed with a small spike in Phase 0 (the file tree already has a working path to copy).

## Integration change checklist

1. **`left_panel.rs`**: the 7 arm/field changes listed under "Relevant code"; subscribe to `git_graph_view` events if needed (e.g. if `GitGraphEvent` must bubble, such as "open file" — v1 may have no events).
2. **`view.rs`**: push in `compute_left_panel_views`; add the keybinding constant and register the action where actions are registered (cf. how `LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME` is registered); a default keybinding can be left unbound.
3. **`crates/warp_features/src/lib.rs`**: add `FeatureFlag::GitGraph`. **Use the `add-feature-flag` skill**, default Dev stage.
4. **Settings**: v1 gates only on flag + cfg; do **not** add a `show_git_graph` user setting (keep the surface small). If a show/hide toggle consistent with other tabs is wanted, defer to a Phase 5 stretch (cf. `CodeSettings::show_project_explorer`).
5. **Telemetry**: optional, via the `add-telemetry` skill, to record "open Git Graph / select commit" — Phase 5.

## Testing

- `layout_tests.rs` (highest value, TDD-first): for hand-built commit sequences, assert `assign_lanes` output `node_col` / segments / `max_lanes`, covering linear / fork / merge / consecutive merge / octopus merge / multi-root.
- `data_tests.rs`: table-driven tests for `parse_commit_log` / `parse_decorate` / `parse_numstat`, including edge cases — subjects containing separators, empty `%D`, multiple parents, renames.
- Integration test (stretch): use `warp-integration-test` to verify panel load and rendering in a temp repo; the core logic is already covered by the two unit-test groups, so the integration test is not required.

## Risks and open items
1. **Connector look**: orthogonal rounded elbows ≠ git-graph beziers; confirm this is acceptable (already declared in PRODUCT non-goals).
2. **Active-repo API**: confirm the existing "working directory → repository root" call in a Phase 0 spike before implementing (low risk; file-tree precedent exists).
3. **Large-repo performance**: bounded by pagination (first ~200, lazy more) + layout is O(commits × lanes); debounced refresh avoids thrash.
4. **Edge shapes**: detached HEAD, worktrees, multi-root, empty repo — layout and parsing must handle these explicitly (covered in test cases).
5. **Refresh granularity**: repo watcher events are fine-grained; debounce, and if needed only re-run on changes under `.git/HEAD`, `.git/refs`, `.git/logs`.

## Implementation phases (suggested order)
- **Phase 0 — spike (~0.5d)**: confirm active-repo resolution; with hardcoded 3-row data, make `GitGraphRowCanvas` draw lanes + dots + one elbow connector, validating that the custom `Element` drawing works.
- **Phase 1 — data + layout (TDD)**: `data.rs` + `layout.rs` + both unit-test groups green, no UI.
- **Phase 2 — panel shell**: wire up `FeatureFlag::GitGraph` + the full set of `ToolPanelView` arms; `GitGraphView` first shows a **plain-text** commit list (no lanes), proving the "flag → button → view → fetch → render" path.
- **Phase 3 — graph rendering**: plug in `GitGraphRowCanvas`, lanes/dots/connectors + palette.
- **Phase 4 — detail and interaction**: click to select → detail (message + changed files + ins/del); refresh button; scroll lazy-load; auto-refresh subscription.
- **Phase 5 — polish**: ref-label styling and theme tokens, empty/error/no-commits states, keybinding, telemetry.
