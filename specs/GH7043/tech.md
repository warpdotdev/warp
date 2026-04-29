# Side-by-Side Diff Layout in the AI Block-List and Code Review Pane - Tech Spec
Product spec: `specs/GH7043/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/7043
Roadmap reference: https://github.com/warpdotdev/warp/issues/9233

## Context

The diff rendering surface is shared by two consumers: the AI block-list inline diff (`InlineDiffView`) and the full Code Review pane (`code_review_view::CodeReviewView`). Both build on the same primitives:

- `app/src/code/diff_viewer.rs` defines the shared abstraction. `DisplayMode` (lines 14-29) carries the visual context: `FullPane`, `Embedded { max_height }`, `InlineBanner { max_height, is_expanded, is_dismissed }`. The `DiffViewer` trait (lines 110-160) holds a single `&ViewHandle<CodeEditorView>` and exposes hunk-navigation, accept, reject, and revert.
- `app/src/code/inline_diff.rs::InlineDiffView` (12.6KB, lines 1-300) wraps a single `CodeEditorView`, registers a `FileModel`-backed file when one exists, and applies the parsed deltas to the editor on construction (`apply_diffs_if_any`, lines 196-228).
- `app/src/code/editor/view.rs::CodeEditorView` (84KB) and `app/src/code/editor/model.rs` (155KB) are the rendering target. Both inline and side-by-side reuse them.
- `app/src/code/editor/diff.rs` (30KB) holds the editor-level diff line decoration and gutter rendering. `DiffLineType` (referenced from `app/src/code_review/comments/diff_hunk_parser.rs`) classifies lines as `Context`, `Add`, `Delete`, `HunkHeader`.
- `app/src/code_review/diff_state.rs` (112KB) holds hunk state, parsed `UnifiedDiffHeader` (lines 67-73), the `DiffMode` enum at line 266 (`Head` / `MainBranch` / `OtherBranch(String)`, *comparison base, not layout*), and the per-file diff state model.
- `app/src/code_review/diff_menu.rs` (501 lines) is the existing View Options popup that the user opens from the diff toolbar. Today its rows are sourced from `Vec<DiffTarget>` (line 99) and the menu emits `CodeReviewDiffMenuEvent::Select(DiffMode)` (line 47) to change comparison base. The new layout selector slots in here.
- `app/src/code_review/code_review_view.rs` (311KB) hosts the diff per file and renders into the pane. It's the consumer that subscribes to setting changes and rebuilds visible diffs on layout change.
- `app/src/ai/blocklist/inline_action/code_diff_view.rs` (where `DisplayMode::FullPane`, `Embedded`, `InlineBanner` are constructed at lines 796, 798, 1248, 1258, 1311, 2026, 2065, 2170, 2204, 2206) is the AI block-list inline-action diff host that wraps `InlineDiffView` and decides which `DisplayMode` to use.
- `app/src/settings/code.rs` (63 lines) is the settings group for `code.*`. Settings are declared via the `define_settings_group!` macro with `toml_path`, `default`, `supported_platforms`, and `sync_to_cloud` fields. Existing entries set the pattern for the new `diff_layout` setting.
- `app/src/features/` defines `FeatureFlag` variants (referenced from `app/src/code_review/diff_state.rs:28`). The new `SideBySideDiffLayout` flag declares here.
- `app/src/code_review/scroll_preservation.rs` (8KB) holds scroll preservation helpers that the side-by-side scroll-sync model can build on.
- `app/src/code_review/comments/diff_hunk_parser.rs` (181 lines) parses hunks into per-line records (`build_line_result`, lines 47-90). The hunk-alignment model for side-by-side reuses this parser; no new parser is introduced.
- `app/src/code_review/comments/comment.rs` and `comment_list_view.rs` (48KB) own comment rendering. Comment placement gains a per-pane gutter marker; the existing thread placement logic is unchanged because comments are still anchored on `EditorLineLocation`.
- `app/src/code_review/telemetry_event.rs` (16KB) defines `CodeReviewTelemetryEvent`. The layout-change event registers here.
- `app/src/code_review/find_model.rs` (13KB) holds the find-in-diff state model that needs to traverse both panes in side-by-side.

The narrowest fix: introduce a `DiffLayout` enum (`Inline` / `SideBySide`), thread it through `DiffViewer`, build a `SideBySideDiffView` in `app/src/code/side_by_side_diff.rs` that holds two `CodeEditorView` instances and a shared scroll-sync model, route the layout choice through the `View Options` menu and a new `code.editor.diff_layout` setting, and gate the whole change behind a `SideBySideDiffLayout` feature flag. The proposed changes below take the existing `DiffViewer` trait as a shared interface and have the side-by-side view implement it; everywhere that today asks "give me the editor" gets back the *focused* editor, and a small set of new accessors expose both editors when needed.

## Proposed changes

### 1. Introduce the `DiffLayout` enum

Add a new file `app/src/code/diff_layout.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLayout {
    #[default]
    Inline,
    SideBySide,
}

impl DiffLayout {
    pub fn is_side_by_side(&self) -> bool {
        matches!(self, DiffLayout::SideBySide)
    }
}
```

Re-export from `app/src/code/mod.rs`. The enum is `Copy` so it can travel through view contexts without lifetime concerns. The `serde` representation (`"inline"` / `"side_by_side"`) matches the `toml_path` value the settings system stores.

`DiffLayout` is intentionally separate from `DisplayMode` because they are orthogonal:
- `DisplayMode` answers "where on screen does this diff live" (own pane vs embedded vs inline banner).
- `DiffLayout` answers "how do we render the diff content" (one column vs two columns).

A `DisplayMode::FullPane` diff and a `DisplayMode::Embedded { max_height }` diff both honor the user's `DiffLayout` choice.

### 2. Add the `code.editor.diff_layout` setting

Extend `define_settings_group!` in `app/src/settings/code.rs:5-62`:

```rust
diff_layout: DiffLayoutSetting {
    type: crate::code::diff_layout::DiffLayout,
    default: crate::code::diff_layout::DiffLayout::Inline,
    supported_platforms: SupportedPlatforms::DESKTOP,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "code.editor.diff_layout",
    description: "Layout for diff views: 'inline' or 'side_by_side'.",
},
```

The setting type system already supports enums via `serde`, matching how other setting groups carry strongly-typed values. Default is `Inline`, so every existing user is unaffected unless they opt in.

### 3. Add the `SideBySideDiffLayout` feature flag

Add `SideBySideDiffLayout` to the `FeatureFlag` enum in `app/src/features/`. The flag defaults to off in shipping builds and on in dogfood builds. The flag is consulted in two places:

- `app/src/code_review/diff_menu.rs` to decide whether to show the Layout radio group rows.
- `app/src/code/inline_diff.rs::InlineDiffView::new` and the Code Review pane's per-diff construction site to decide whether to read the setting at all (when off, layout is hard-coded to `Inline`).

Once the feature stabilizes, the flag is removed in a follow-up PR. The setting and the menu rows persist.

### 4. Build `SideBySideDiffView`

Add `app/src/code/side_by_side_diff.rs`. The new type mirrors `InlineDiffView`'s lifecycle but holds two editors:

```rust
pub struct SideBySideDiffView {
    baseline: ViewHandle<CodeEditorView>,
    modified: ViewHandle<CodeEditorView>,
    diff_type: Option<DiffType>,
    file_path: Option<StandardizedPath>,
    /// Hunk alignment computed once per diff application; rebuilt on diff updates.
    alignment: HunkAlignment,
    /// Shared vertical scroll position that drives both panes.
    scroll_sync: ScrollSyncModel,
    /// Which pane is currently focused for cursor navigation.
    focused_pane: Pane,
    /// File registration state, identical to `InlineDiffView`.
    backing_file_id: Option<FileId>,
    was_edited: bool,
    #[cfg(not(target_family = "wasm"))]
    is_new_file: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pane {
    Baseline,
    Modified,
}
```

Key methods:

- `new(baseline_editor, modified_editor, diff_type, display_mode, file_path, ctx)` constructs the view, subscribes to both editors' `CodeEditorEvent`, applies the diff to `modified` and the pre-diff content to `baseline`, then computes `HunkAlignment` and pushes the resulting padding rows into both editors via a new `CodeEditorView::set_padding_rows(Vec<PaddingRow>)` API (see Change 5).
- `register_file(session_type, ctx)` registers the *modified* editor with `FileModel`, mirroring `InlineDiffView::register_file` (`app/src/code/inline_diff.rs:122-153`). The baseline pane is always read-only.
- `apply_diffs_if_any(ctx)` mirrors `InlineDiffView::apply_diffs_if_any` (`app/src/code/inline_diff.rs:196-228`). For `DiffType::Update`, both editors get content; for `DiffType::Create` the baseline pane shows an empty file with a "(new file)" header line; for `DiffType::Delete` the modified pane shows an empty file with "(deleted)".
- `set_display_mode(mode, ctx)` calls the existing `DiffViewer::set_display_mode` body once per editor.

Implement `DiffViewer` for `SideBySideDiffView` so that `editor()` returns the focused pane's editor and the trait's existing methods work without changes. Add new trait methods on a separate `MultiPaneDiffViewer` trait (defaulted on `DiffViewer`) for callers that need both panes:

```rust
pub trait MultiPaneDiffViewer: DiffViewer {
    fn baseline_editor(&self) -> &ViewHandle<CodeEditorView> {
        self.editor()
    }
    fn modified_editor(&self) -> &ViewHandle<CodeEditorView> {
        self.editor()
    }
    fn focused_pane(&self) -> Option<Pane> {
        None
    }
}
```

The default implementation delegates everything to `editor()`, so existing single-pane consumers (`InlineDiffView` and `LocalCodeEditorView`) need no changes. `SideBySideDiffView` overrides all three.

### 5. Editor padding-row API

`CodeEditorView` (`app/src/code/editor/view.rs`) and the underlying model (`app/src/code/editor/model.rs`) gain a new "padding row" concept: a row that takes up vertical space, has a gutter, and renders empty content. The new public API:

```rust
impl CodeEditorView {
    pub fn set_padding_rows(&mut self, rows: Vec<PaddingRow>, ctx: &mut ViewContext<Self>);
}

#[derive(Clone, Debug)]
pub struct PaddingRow {
    pub line_index: usize,
    pub count: usize,
    pub gutter_kind: PaddingGutterKind,
}

#[derive(Clone, Copy, Debug)]
pub enum PaddingGutterKind {
    /// Render the gutter as the diff-add color band, no glyph, no line number.
    AddSide,
    /// Render the gutter as the diff-delete color band, no glyph, no line number.
    DeleteSide,
}
```

Padding rows are inserted at render time only. They don't affect the underlying buffer, so:

- Save (`FileModel::save`) still writes the unmodified buffer text.
- Find, copy, selection, and cursor navigation all see the buffer as-is. Padding rows never become part of a selection.
- Hunk navigation (`navigate_next_diff_hunk`, `navigate_previous_diff_hunk` on the trait) operates on actual diff hunks, not padding rows.

Implementation lives in `app/src/code/editor/model.rs` next to the existing line-iteration code; the renderer in `app/src/code/editor/view.rs` consults the padding-row table when laying out lines and inserts a fixed-height empty row at each padding position.

This is the minimal addition that lets the right pane render blanks where the left pane has a deleted line, and vice versa, without diverging the buffer model from the file content.

### 6. `HunkAlignment`

Add `app/src/code/hunk_alignment.rs`:

```rust
pub struct HunkAlignment {
    /// Padding rows for the baseline pane.
    pub baseline_padding: Vec<PaddingRow>,
    /// Padding rows for the modified pane.
    pub modified_padding: Vec<PaddingRow>,
    /// Per-row mapping: rendered row index -> (baseline buffer line, modified buffer line).
    /// `None` on a side means that side renders padding at this row.
    pub row_map: Vec<(Option<usize>, Option<usize>)>,
}

impl HunkAlignment {
    pub fn from_unified_diff(
        baseline_lines: &[String],
        modified_lines: &[String],
        hunks: &[UnifiedDiffHeader],
    ) -> Self;
}
```

The algorithm walks unified-diff hunks (parsed by `app/src/code_review/comments/diff_hunk_parser.rs::build_line_result`):

- For each `Context` line: emit a `(Some(b), Some(m))` row, advance both indices.
- For each `Delete` line: emit a `(Some(b), None)` row, advance baseline only, append a one-line `PaddingRow { gutter_kind: AddSide }` to `modified_padding` at the current modified index.
- For each `Add` line: emit a `(None, Some(m))` row, advance modified only, append a one-line `PaddingRow { gutter_kind: DeleteSide }` to `baseline_padding` at the current baseline index.

Modifications (a `Delete` immediately followed by an `Add`) emit `(Some(b), None)` then `(None, Some(m))` by default. A follow-up optimization (out of scope for this spec) can collapse adjacent delete-add pairs into a single `(Some(b), Some(m))` row to keep modified lines on the same row as the line they replaced. The minimal, correctness-first version above is what ships first; the collapsed version follows once telemetry confirms users prefer it.

`row_map` is consulted by the scroll-sync model in Change 7.

### 7. `ScrollSyncModel`

Add `app/src/code/scroll_sync.rs`:

```rust
pub struct ScrollSyncModel {
    /// Currently focused pane for cursor moves.
    focused_pane: Pane,
}

impl ScrollSyncModel {
    /// Translate a wheel delta on `from` into matching scroll deltas for both panes.
    pub fn on_scroll_wheel(&self, from: Pane, delta_y: f32) -> (f32, f32);

    /// Translate a cursor move on `from` (new buffer line in that pane) into the
    /// corresponding row on the other pane via `row_map`.
    pub fn corresponding_row(
        &self,
        from: Pane,
        new_buffer_line: usize,
        alignment: &HunkAlignment,
    ) -> Option<usize>;
}
```

Wheel events on either pane drive both panes by the same row-delta. Cursor moves only move the cursor on the focused pane; the other pane scrolls to keep `corresponding_row` in view but does not move its own cursor. Builds on the existing scroll-preservation patterns in `app/src/code_review/scroll_preservation.rs`.

`SideBySideDiffView::new` subscribes to `CodeEditorEvent::ScrolledByUser` (existing event from `app/src/code/editor/view.rs`) on both editors and mirrors the resulting deltas through the model.

### 8. Layout selection in the View Options menu

Extend `app/src/code_review/diff_menu.rs::CodeReviewDiffMenu`:

- Add a `MenuRowKind` enum (`DiffTargetRow(DiffTarget)` for today's behavior, `LayoutRow(DiffLayout)` for the new rows). Replace `targets: Vec<DiffTarget>` (line 99) with `rows: Vec<MenuRow>`. The `filtered: Vec<(usize, Option<FuzzyMatchResult>)>` shape is unchanged.
- When the `SideBySideDiffLayout` feature flag is on, the menu builder appends two `LayoutRow` entries ("Inline" and "Side-by-Side") below the existing diff-target rows, separated by a divider row.
- `CodeReviewDiffMenuEvent` (line 47) gains a `SelectLayout(DiffLayout)` variant. When the menu emits it, the parent (`code_review_view`) writes the new layout to `code.editor.diff_layout` via the settings store and refreshes the visible diff.

For the AI block-list, the equivalent menu lives in `app/src/ai/blocklist/inline_action/code_diff_view.rs`. The same two-row group is appended to that menu's overflow with the same handler pattern.

### 9. Code Review pane integration

`app/src/code_review/code_review_view.rs` (311KB) is the parent that holds per-file `InlineDiffView` instances today. The integration:

- On view construction, read `code.editor.diff_layout` from the settings store. If `SideBySide` and the feature flag is on, build `SideBySideDiffView` for each file; otherwise build `InlineDiffView` as today.
- Subscribe to setting updates. When `code.editor.diff_layout` changes, the pane swaps the per-file view: tear down the current `InlineDiffView` / `SideBySideDiffView`, build the matching new one with the same diff data, and preserve scroll position via `app/src/code_review/scroll_preservation.rs`.
- Find-in-diff (`app/src/code_review/find_model.rs`) gains a `Pane`-aware iterator that walks both editors when the active diff is a `SideBySideDiffView`. The existing find loop becomes `for pane in active_panes()` where `active_panes()` returns `[Modified]` for inline and `[Baseline, Modified]` for side-by-side, in tab order.

The `code_review_view_integration.rs` and `code_review_view_tests.rs` test fixtures gain a `with_layout(DiffLayout)` helper that sets the layout in the test settings store before constructing the view.

### 10. AI block-list integration

`app/src/ai/blocklist/inline_action/code_diff_view.rs` decides which `DisplayMode` to use per inline-action diff. Today every site constructs `DisplayMode::with_embedded(MAX_EDITOR_HEIGHT)` (line 798) or `DisplayMode::with_inline_banner(INLINE_EDITOR_HEIGHT)` (line 796).

The integration:

- Read `code.editor.diff_layout` at the same place these `DisplayMode` decisions are made.
- For `DiffLayout::Inline`, construct `InlineDiffView` as today.
- For `DiffLayout::SideBySide`, construct `SideBySideDiffView` with the same `DisplayMode`.

`InlineBanner` mode is a special case: side-by-side at a small max height shows two cramped columns. For this spec, `SideBySide` falls back to `Inline` when the chosen `DisplayMode` is `InlineBanner { .. }`. The fallback is documented in product invariant 2 implicitly (the spec discusses the embedded and full-pane surfaces) and is called out here so the implementation is unambiguous. A future spec can revisit the `InlineBanner` side-by-side behavior.

### 11. Comment threads

`app/src/code_review/comments/` (8 files, 100KB+ total) anchors comment threads on `EditorLineLocation` (`app/src/code/editor/line.rs::EditorLineLocation`). Comments stay attached to those locations and render under the targeted line. The integration:

- The renderer for a side-by-side row checks each comment's pane (the original side it was authored against, captured from `CommentSide` in `app/src/ai/agent/action.rs`) and renders the thread under the matching pane's row.
- The opposite pane shows a small marker glyph in its gutter at the same row to indicate that the other side has a thread there. The marker is non-interactive in this spec (clicking it does not focus the comment); a follow-up spec can wire that.
- Multi-line comment ranges that span both deleted and added regions stay on the side they were authored against.

### 12. Telemetry

Add a `CodeReviewTelemetryEvent::DiffLayoutChanged { from: DiffLayout, to: DiffLayout, surface: TelemetrySurface }` variant in `app/src/code_review/telemetry_event.rs`. The `surface` enum distinguishes Code Review vs AI block-list. `code_review_view` and the AI inline-action diff host emit the event when the menu emits `SelectLayout`.

### 13. Settings UI

`code.editor.diff_layout` is reachable from both:
- The View Options menu in the diff toolbar (the primary path; product invariant 6).
- The Settings pane under Code, where users can flip it without opening a diff. This follows the existing pattern for settings declared in `app/src/settings/code.rs` (e.g. `show_project_explorer`, line 50).

Both surfaces write to the same setting key, so changes propagate through the existing settings subscription.

## Test plan

### Unit tests

- `app/src/code/hunk_alignment_tests.rs`:
  - Empty diff: row_map is `[(Some(0), Some(0)), ...]` for every line, no padding.
  - Pure addition: rows for the added section are `(None, Some(m))`; baseline_padding has the matching count.
  - Pure deletion: rows for the deleted section are `(Some(b), None)`; modified_padding has the matching count.
  - Modification: deleted line and added line both produce rows; both pads are emitted.
  - Multi-hunk file: alignment composes correctly across hunks separated by unchanged context.
  - Large diff (5,000 lines, 200 hunks): completes in under 50ms.

- `app/src/code/scroll_sync_tests.rs`:
  - Wheel delta on baseline drives both panes equally.
  - Cursor move on modified scrolls baseline to the corresponding row.
  - Cursor on a `(None, Some(m))` row (pure-add): baseline scrolls to the next surrounding context line.

- `app/src/code/side_by_side_diff_tests.rs`:
  - `apply_diffs_if_any` for `DiffType::Update`: both editors hold expected content; alignment is non-empty.
  - `apply_diffs_if_any` for `DiffType::Create`: baseline editor is empty with a header; modified holds the new file content.
  - `apply_diffs_if_any` for `DiffType::Delete`: modified editor is empty; baseline holds the original content.
  - `register_file` registers only the modified editor with `FileModel`; the baseline editor remains read-only.
  - `set_display_mode` propagates to both editors (assert via the existing `set_*` calls captured by the test fixture).
  - `accept_and_save_diff` writes the modified editor's buffer text via `FileModel`, identical to `InlineDiffView`'s save path.
  - `reject_diff` discards the modification; subsequent `apply_diffs_if_any` of the same diff produces an empty alignment.
  - Layout switch from `Inline` to `SideBySide` and back preserves scroll position to within one row.

### Integration tests

- `app/src/code_review/code_review_view_tests.rs` gains tests using the new `with_layout(DiffLayout)` helper:
  - Single-file diff in side-by-side renders both panes.
  - Multi-file diff: each file's view honors the same layout.
  - Setting flip while open swaps every visible file's view from `InlineDiffView` to `SideBySideDiffView` and preserves scroll.
  - Find-in-diff matches both panes.
  - Hunk navigation (`f` / `F`) advances the focused hunk on both panes simultaneously.
  - Comment thread on a baseline-side line renders under the baseline pane's row; modified pane shows the gutter marker.

### Manual smoke test

- macOS, M1 MacBook Air, dogfood build with `SideBySideDiffLayout` enabled:
  - Open Code Review with a 200-file diff. Toggle layout. Confirm the toggle takes effect within 200ms on the active diff.
  - Scroll wheel on each pane. Confirm both panes scroll together.
  - Drag-select on each pane. Confirm the selection stays in that pane.
  - Cmd-A on each pane. Confirm only that pane's content is selected.
  - Resize the window narrow enough that side-by-side is cramped. Confirm horizontal scrollbars on each pane behave independently and the divider stays at 50%.
  - Open an AI block-list diff with a recent agent edit. Toggle layout. Confirm the embedded diff swaps from one column to two and back.
  - Linux (Ubuntu 24.04), Windows 11: repeat the toggle smoke test on each platform to confirm rendering and keybindings.

### Compile-parity checklist

Every site that destructures `DisplayMode` in a `match` must compile after the change. The current call sites (from `grep -rn "DisplayMode::\(FullPane\|Embedded\|InlineBanner\)"`):

- `app/src/code/diff_viewer.rs:45,46,47,53,62,71,72,73,82,83,84,89,94,100,104,108`: trait helpers; unchanged because `DiffLayout` is a new orthogonal axis.
- `app/src/ai/blocklist/inline_action/code_diff_view.rs:796,798,1248,1258,1311,2026,2065,2170,2204,2206`: `DisplayMode` construction and matching; unchanged.
- `app/src/ai/blocklist/block/view_impl/output.rs:2061`: match on `DisplayMode::FullPane`; unchanged.

Every site that constructs `InlineDiffView` (per `grep -rn "InlineDiffView::new"`) is the candidate set for layout-aware view construction. The list is verified during implementation; the integration in Change 9 and Change 10 covers the two parent surfaces.

## Open questions

1. The `InlineBanner` fallback to `Inline` for side-by-side (Change 10) is the safest default. Should the toolbar show a "Side-by-side not available in this layout" hint to the user, or silently fall back? The current spec says silently; the Code Review SME may prefer a hint.
2. The collapsed delete-add row optimization (mentioned in Change 6) is out of scope for the first ship. A telemetry signal that quantifies how often a delete is immediately followed by an add helps prioritize the follow-up.
3. Comment thread interaction with the opposite-pane gutter marker (Change 11) is non-interactive in this spec. Whether the marker should be clickable (focuses the comment in the other pane) is a UX call for the Code Review SME and a candidate follow-up.
