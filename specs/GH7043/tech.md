# Side-by-Side Diff Layout in the Code Review Pane - Tech Spec
Product spec: `specs/GH7043/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/7043
Roadmap reference: https://github.com/warpdotdev/warp/issues/9233

## Context

The v1 diff rendering surface is the Code Review pane only. AI block-list diffs and inline banner diffs continue to use the existing inline path until a v2 spec extends the layout setting to those surfaces.

Relevant Code Review and editor primitives:

- `app/src/code/editor/view.rs::CodeEditorView` is the rendering target. V1 adds a side-by-side diff render mode to this view instead of introducing a sibling side-by-side view type.
- `app/src/code/editor/element.rs::EditorWrapper` owns editor rendering, layout, hit testing, gutters, and text shaping for a `CodeEditorView`. V1 keeps one `EditorWrapper` and teaches it to render both panes.
- `warp_editor::render::model::RenderState` holds the state used for shaped text, gutters, visible rows, and hit testing. V1 uses two render states inside the single wrapper: `baseline_render_state` and `modified_render_state`.
- `app/src/code/editor/diff.rs` holds editor-level diff line decoration and gutter rendering. `DiffLineType` classifies lines as `Context`, `Add`, `Delete`, and `HunkHeader`.
- `app/src/code_review/diff_state.rs` holds hunk state, the `DiffMode` enum (`Head` / `MainBranch` / `OtherBranch(String)`, comparison base rather than layout), and the per-file diff state model.
- `app/src/code_review/comments/diff_hunk_parser.rs` parses hunks into ordered per-line records. The side-by-side aligner consumes those records; no new parser is introduced.
- `app/src/code_review/editor_state.rs::CodeReviewEditorState` owns Code Review editor state. The layout hook belongs here or in the equivalent shared wrapper, not in per-file `InlineDiffView` migration code.
- `app/src/code/local_code_editor.rs::LocalCodeEditorView` owns the local editor path that hosts Code Review editors. Layout flips route through this host into `CodeEditorView::set_diff_layout`.
- `app/src/settings/code.rs` is the settings group for `code.*`. Settings are declared via `define_settings_group!` with `toml_path`, `default`, `supported_platforms`, and `sync_to_cloud` fields.
- `app/src/settings_view/code_page.rs` is the explicit Code settings UI. Declaring a settings entry does not render it; the page needs a concrete widget registered in the Code section.
- `crates/warp_features/src/lib.rs` defines the canonical `FeatureFlag` enum, `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, and changelog descriptions. `app/src/features.rs` re-exports the feature API.
- `app/src/lib.rs` builds the set of compiled-in feature flags through cfg-gated `FeatureFlag::Variant` entries. The corresponding Cargo feature declarations live in `app/Cargo.toml`.
- `app/src/code_review/scroll_preservation.rs` holds scroll preservation helpers that the side-by-side scroll-sync model can build on.
- `app/src/code_review/comments/comment.rs` and `comment_list_view.rs` own comment rendering. Comment placement gains a per-pane gutter marker; existing anchoring on `EditorLineLocation` remains.
- `app/src/code_review/telemetry_event.rs` defines `CodeReviewTelemetryEvent`. The layout-change event registers here.
- `app/src/code_review/find_model.rs` holds the find-in-diff state model that needs to traverse both panes in side-by-side.

The implementation introduces a `DiffLayout` enum (`Inline` / `SideBySide`), stores it as `code.editor.diff_layout`, exposes it in Settings -> Code, and gates the Code Review path behind `SideBySideDiffLayout`.

Architecture choice: `CodeEditorView` gains a `DiffLayout::SideBySide` render mode. The single `EditorWrapper` inside `CodeEditorView` renders both sides and owns two `RenderState`s. This follows Kevin's current lean and keeps scroll sync, gap resizing, hit testing, and modified-pane edits internal to one editor view.

## Proposed changes

### 1. Introduce the `DiffLayout` enum

Add `app/src/code/diff_layout.rs`:

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

Re-export from `app/src/code/mod.rs`. The enum is `Copy` so it can travel through view contexts without lifetime concerns. The `serde` representation (`"inline"` / `"side_by_side"`) matches the setting value.

`DiffLayout` is intentionally separate from display and comparison state:

- `DisplayMode` answers "where on screen does this diff live" (own pane vs embedded vs inline banner).
- `DiffMode` answers "what is being compared" (head vs main branch vs another branch).
- `DiffLayout` answers "how do we render the diff content" (one column vs two columns).

V1 only reads `DiffLayout` in the Code Review pane. AI block-list and inline banner hosts keep their current inline behavior even when the stored value is `side_by_side`.

### 2. Add the `code.editor.diff_layout` setting

Extend `define_settings_group!` in `app/src/settings/code.rs`:

```rust
diff_layout: DiffLayoutSetting {
    type: crate::code::diff_layout::DiffLayout,
    default: crate::code::diff_layout::DiffLayout::Inline,
    supported_platforms: SupportedPlatforms::DESKTOP,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "code.editor.diff_layout",
    description: "Layout for Code Review diff views: 'inline' or 'side_by_side'.",
},
```

The setting type system already supports enums via `serde`, matching how other setting groups carry strongly typed values. Default is `Inline`, so existing users are unaffected unless they opt in.

Resolution: this setting is intentionally global storage even though v1 has one participating surface. That leaves room for v2 surfaces without adding a second setting name.

### 3. Settings page integration in `settings_view/code_page.rs`

`code.editor.diff_layout` is exposed in Settings -> Code under a new "Diff layout" subsection. This is the primary user entry point for the Code Review preference.

Add a `DiffLayoutWidget` row in `app/src/settings_view/code_page.rs`:

- The widget renders a two-option segmented control: "Inline" and "Side by side".
- The selected segment reads from `CodeSettings::DiffLayout`.
- Segment changes write `code.editor.diff_layout` through the settings store and emit the layout-change telemetry event.
- Register the widget explicitly in the Code settings group near the existing Code Review settings widgets. This resolves the CodeSettings rendering concern: declaring the setting alone does not make it appear in the page.
- Gate widget registration on `FeatureFlag::SideBySideDiffLayout.is_enabled()` so the control is hidden while the runtime flag is off.

The diff toolbar continues to own only per-view ephemeral controls, such as whitespace visibility. It does not expose `code.editor.diff_layout`.

Resolution: the settings-page widget integration is part of v1 and is not left as implicit settings metadata.

### 4. Add the `SideBySideDiffLayout` feature flag

The flag is wired in the actual repo feature-flag locations:

1. **Enum variant**: add `SideBySideDiffLayout,` to `crates/warp_features/src/lib.rs::FeatureFlag`, near related Code Review flags such as `CodeReviewFind`.

2. **Cargo feature and compiled-in registration**:
   - Add `side_by_side_diff_layout = []` to `[features]` in `app/Cargo.toml`, following the existing `code_review_find = []` pattern.
   - Add the cfg-gated entry in `app/src/lib.rs` alongside the other compiled-in feature flags:
     ```rust
     #[cfg(feature = "side_by_side_diff_layout")]
     FeatureFlag::SideBySideDiffLayout,
     ```

3. **Dogfood and preview runtime defaults**:
   - Add `FeatureFlag::SideBySideDiffLayout` to `DOGFOOD_FLAGS` in `crates/warp_features/src/lib.rs` for the first internal phase.
   - Move it to `PREVIEW_FLAGS` when widening beyond dogfood. Preview flags are automatically included in dogfood builds.
   - Do not add it to `RELEASE_FLAGS` until the staged rollout is complete.

4. **Changelog description**: add a `description_for_changelog` match arm:
   ```rust
   SideBySideDiffLayout => Some("Enables a side-by-side diff layout in the code review pane."),
   ```

Rollout:

- Compile-time gate: `side_by_side_diff_layout` controls whether the app binary includes `FeatureFlag::SideBySideDiffLayout` in the compiled-in flag list.
- Runtime gate: the feature-flag service decides whether `FeatureFlag::SideBySideDiffLayout.is_enabled()` returns true for the current channel/user.
- Dispatch bridge: Code Review construction first checks the runtime flag. If disabled, it treats the effective layout as `DiffLayout::Inline` regardless of stored settings. If enabled, it reads `code.editor.diff_layout` and calls `CodeEditorView::set_diff_layout`.
- Default rollout schedule: off in shipping builds -> 5% dogfood -> 25% dogfood -> 100% dogfood -> preview -> release.

Resolution: hidden flag state suppresses both the settings widget and the runtime layout path.

### 5. Make `CodeEditorView` layout-aware

`CodeEditorView` gains a layout setter and pane-aware accessors instead of introducing a sibling side-by-side view type:

```rust
impl CodeEditorView {
    pub fn set_diff_layout(
        &mut self,
        layout: DiffLayout,
        diff_hunks: &[DiffHunk],
        ctx: &mut ViewContext<Self>,
    );

    pub fn editor_for(&self, side: Side) -> &CodeEditor;

    pub fn editor(&self) -> &CodeEditor {
        self.editor_for(Side::Modified)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Baseline,
    Modified,
}
```

Semantics:

- `editor_for(Side::Modified)` returns the post-diff editor state and is the only write-capable side.
- `editor_for(Side::Baseline)` returns the pre-diff read-only side when `DiffLayout::SideBySide` is active. In inline mode it may return the same editor state with read-only diff decorations, depending on implementation details.
- `editor()` remains available for legacy call sites and returns the modified side. This keeps existing save, accept, cursor, and selection paths pointed at the side that can change.
- New side-aware call sites use `editor_for(side)` only when they intentionally render or inspect a specific pane.

Caller routing:

| Caller / behavior | Pane-aware route |
|---|---|
| Cursor focus, local selection, copy from modified side | `editor_for(Side::Modified)` |
| Copy from baseline side | `editor_for(Side::Baseline)` with selection-only focus |
| `changed_lines` | `editor_for(Side::Modified)` |
| Accept diff, save diff, reject-to-modified-buffer operations | `editor_for(Side::Modified)` |
| Hunk navigation | `CodeEditorView` row alignment plus modified-side focus |
| Scroll preservation | `CodeEditorView` layout state, not cross-view event mirroring |
| Comment rendering and gutter markers | `baseline_render_state` and `modified_render_state` via row alignment |

Resolution: the old single-editor assumption remains valid for legacy write paths because the default editor is the modified side. New pane-specific behavior opts into `editor_for`.

### 6. Render both panes in one `EditorWrapper`

`EditorWrapper` gains a diff-layout branch:

```rust
pub struct EditorWrapper {
    render_state: RenderState,
    side_by_side: Option<SideBySideRenderState>,
}

pub struct SideBySideRenderState {
    baseline_render_state: RenderState,
    modified_render_state: RenderState,
    alignment: HunkAlignment,
    focused_side: Side,
    divider_x: Pixels,
}
```

In `DiffLayout::Inline`, the existing `render_state` path is used unchanged.

In `DiffLayout::SideBySide`:

- The single wrapper lays out two equal-width panes and one divider.
- `baseline_render_state` is initialized from the pre-diff buffer rows and delete decorations.
- `modified_render_state` is initialized from the post-diff buffer rows and add decorations.
- Text shaping dispatches per pane, using each pane's width and buffer rows.
- Gutter rendering dispatches per pane, with delete gutters on the baseline side and add gutters on the modified side.
- Hit testing first determines the pane from x-position, then queries that pane's render state.
- The left pane accepts focus only for selection and copy. It never exposes an edit cursor.
- The right pane owns cursor state and all writeable editor interactions.

Gap rows are emitted into both render states symmetrically. If one side has real content and the other side has a gap, the gap render state still receives a full-height empty row with gutter metadata so layout, hit testing, comments, and scrolling agree.

Resolution: scroll sync and gap resizing happen inside one component. There is no pair of `CodeEditorView`s and no cross-view scroll-event mirroring.

### 7. Pane content construction

Side-by-side reuses the existing unified-diff parser but does not reuse inline deleted-line rendering. The side-by-side pipeline is:

1. Parse the unified diff to `DiffHunk[]` using the existing parser. No parser changes are needed.
2. Build two `PaneBuffer` structs:
   - `baseline` contains pre-diff lines plus delete decorations.
   - `modified` contains post-diff lines plus add decorations.
3. Run hunk alignment over the ordered hunk lines. Each `AlignedRow` maps to a row index in both panes. Gap rows do not exist in either source file, but they render as full-height empty rows with the appropriate gutter color.
4. Initialize `baseline_render_state` with baseline rows, delete decorations, and baseline gap rows.
5. Initialize `modified_render_state` with modified rows, add decorations, and modified gap rows.

`apply_diffs_if_any` remains the inline path. When `DiffLayout::SideBySide` is active, `CodeEditorView` routes through the pane-content pipeline instead:

- Removed lines render only in `baseline_render_state`.
- Added lines render only in `modified_render_state`.
- Inline temp-block deletion rendering is disabled for the modified side to prevent deletion bleed.
- Accept, reject, save, and changed-line computation continue to read the modified buffer, matching inline behavior.

Resolution: this supersedes the old plan to build a second side-by-side view and avoids mixing inline deletion temp blocks into the modified pane.

### 8. Per-pane interaction state

Baseline pane:

- Always read-only.
- Supports text selection and copy for selectable baseline content.
- Does not expose an insertion cursor.
- Does not consume keyboard edit events.
- Does not participate in file-backed save.
- Deleted-line ranges are not selectable.

Modified pane:

- Owns the cursor.
- Owns all writeable interactions.
- Follows the existing Code Review rules for accept, reject, save, revert, and hunk navigation.
- Is the only side registered with `FileModel`.

`set_diff_layout` applies Code Review interaction state to the modified pane and hard-codes baseline interaction state to read-only selection/copy. `FullPane` behavior from other surfaces is not part of v1.

Resolution: this addresses the right-pane-only edit and cursor requirements directly in the editor interaction model.

### 9. RowIndex and hunk alignment

Add or update `app/src/code/hunk_alignment.rs`:

```rust
pub struct DiffHunk {
    pub header: UnifiedDiffHeader,
    pub lines: Vec<DiffLine>,
}

pub enum DiffLine {
    Context(String),
    Add(String),
    Delete(String),
}

pub struct AlignedHunk {
    pub rows: Vec<AlignedRow>,
}

pub struct AlignedRow {
    pub left: PaneLine,
    pub right: PaneLine,
    pub row_index: RowIndex,
}

pub enum PaneLine {
    Line { buffer_line: usize, text: String },
    Gap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowIndex {
    Baseline(usize),
    Modified(usize),
    Gap { after_row: usize },
}

pub struct HunkAlignment {
    pub baseline_rows: Vec<RowIndex>,
    pub modified_rows: Vec<RowIndex>,
    pub row_map: Vec<(Option<RowIndex>, Option<RowIndex>)>,
}

impl HunkAlignment {
    pub fn from_diff_hunks(hunks: &[DiffHunk]) -> Self;
}
```

`RowIndex` semantics:

- `RowIndex::Baseline(n)` means this aligned row corresponds to baseline buffer row `n`.
- `RowIndex::Modified(n)` means this aligned row corresponds to modified buffer row `n`.
- `RowIndex::Gap { after_row }` means this aligned row is a render-only gap inserted after aligned row `after_row`. It does not correspond to either side's source line numbering.

The alignment producer emits `RowIndex` values while it walks hunks:

- Context rows produce `Baseline(b)` on the left and `Modified(m)` on the right.
- Paired delete/add modification rows produce `Baseline(b)` on the left and `Modified(m)` on the right.
- Pure deletion rows produce `Baseline(b)` on the left and `Gap { after_row }` on the right.
- Pure addition rows produce `Gap { after_row }` on the left and `Modified(m)` on the right.

Each `RenderState` consumes only its side's `RowIndex`:

- `Baseline(n)` and `Modified(n)` map to real shaped text rows for that pane.
- `Gap { after_row }` maps to a full-height empty render row with gutter and background metadata.
- Selection, copy, cursor, save, and find ignore gap rows as source text. Hit testing a gap row resolves to the nearest valid row for scroll anchoring and comment positioning.

The v1 algorithm is single-pass and pairs collapsed delete/add runs before emitting gap rows:

```text
for hunk in hunks:
    pending_deletes = []
    pending_adds = []

    for line in hunk.lines:
        if line is Context:
            flush_pending_pairs()
            emit row(left = context, right = context)

        if line is Delete:
            if pending_adds is not empty:
                flush_pending_pairs()
            pending_deletes.push(line)

        if line is Add:
            pending_adds.push(line)

    flush_pending_pairs()

flush_pending_pairs():
    pair_count = min(pending_deletes.len, pending_adds.len)
    emit pair_count rows with left delete and right add
    emit remaining deletes with right Gap
    emit remaining adds with left Gap
    clear pending_deletes and pending_adds
```

Example:

```text
Input hunk lines:
Context("fn old() {")
Delete("    a();")
Delete("    b();")
Add("    c();")
Add("    d();")
Context("}")

Aligned rows:
1. left Context("fn old() {") | right Context("fn old() {")
2. left Delete("    a();")    | right Add("    c();")
3. left Delete("    b();")    | right Add("    d();")
4. left Context("}")          | right Context("}")
```

Edge cases:

- Pure insertion blocks produce rows with left `Gap` and right `Add`; the baseline render state receives gap rows.
- Pure deletion blocks produce rows with left `Delete` and right `Gap`; the modified render state receives gap rows.
- A `Context` inside a modification resets the pairing window. Deletes before the context are flushed before adds after the context are considered.
- A delete run followed by an add run with unrelated text still pairs by position. Word-level highlighting is out of scope; v1 guarantees row alignment.

Resolution: `line_index` is not a free integer. The API uses `RowIndex` so baseline rows, modified rows, and render-only gaps are distinguishable.

### 10. Intra-component scroll sync

Side-by-side scroll sync is owned by `CodeEditorView` / `EditorWrapper`:

```rust
pub struct SideBySideScrollState {
    focused_side: Side,
    vertical_scroll: ScrollOffset,
    horizontal_scroll_by_side: BTreeMap<Side, ScrollOffset>,
}

impl SideBySideScrollState {
    pub fn on_scroll_wheel(&mut self, side: Side, delta_y: f32);

    pub fn corresponding_row(
        &self,
        side: Side,
        row_index: RowIndex,
        alignment: &HunkAlignment,
    ) -> Option<RowIndex>;
}
```

There are no mirrored scroll events between two independent views. A wheel event mutates one shared vertical scroll offset used by both render states. Horizontal scroll remains per pane.

Cursor movement belongs to the modified side. When cursor navigation changes the modified row, `corresponding_row` computes the nearest baseline row for visibility but does not move a baseline cursor. Hunk navigation updates the shared scroll anchor and the modified-side cursor.

Resolution: the old `ScrolledByUser` recursion-suppression concern is resolved by removing cross-view event mirroring. If implementation still needs event source tagging inside the single wrapper, it is local defensive state rather than an architectural dependency.

### 11. Code Review pane integration

The Code Review path uses `LocalCodeEditorView` / `CodeReviewEditorState`, not per-file `InlineDiffView` instances. The integration:

- Add a `DiffLayout` field or derived setting hook on `CodeReviewEditorState` (or the equivalent shared wrapper that owns Code Review editor configuration).
- On Code Review editor construction, compute the effective layout. If `SideBySideDiffLayout` is off, use `Inline`. If it is on, read `code.editor.diff_layout`.
- Route construction through `LocalCodeEditorView` into `CodeEditorView::set_diff_layout(effective_layout, diff_hunks, ctx)`.
- Subscribe to setting updates in the Code Review host. When `code.editor.diff_layout` changes, call `LocalCodeEditorView` -> `CodeEditorView::set_diff_layout` for visible editors and preserve scroll position via `app/src/code_review/scroll_preservation.rs`.
- Find-in-diff gains a `Side`-aware iterator over the single `CodeEditorView`'s render states when the active diff is side-by-side. Inline returns only the modified side.
- Hidden-line expansion updates the single alignment model, then rebuilds both render states from the same expanded hunk set.

There is no `InlineDiffView` migration step in this architecture.

Resolution: this retargets the integration plan to the actual Code Review editor host and makes hidden-line sync a single-source-of-truth alignment update.

### 12. AI block-list and inline banner v2 deferral

AI block-list and inline banner integration are out of scope for v1:

- `app/src/ai/blocklist/inline_action/code_diff_view.rs` continues constructing inline diffs.
- `InlineBanner` continues using the existing inline rendering path.
- The v1 settings helper text names Code Review only.
- Telemetry does not claim AI block-list or inline banner adoption.

V2 can extend the same `DiffLayout` setting to those surfaces after the Code Review architecture is validated.

Resolution: this addresses the request to start by only using side-by-side in the Code Review panel.

### 13. Comment threads

`app/src/code_review/comments/` anchors comment threads on `EditorLineLocation`. Comments stay attached to those locations and render under the targeted line. The integration:

- The renderer for a side-by-side row checks each comment's side, captured from existing comment metadata such as `CommentSide` in `app/src/ai/agent/action.rs`, and renders the thread under the matching pane's row.
- The opposite pane shows a small marker glyph in its gutter at the same row to indicate that the other side has a thread there. The marker is non-interactive in this spec.
- Multi-line comment ranges that span both deleted and added regions stay on the side they were authored against.

Because both panes are render states inside one `CodeEditorView`, comment placement uses the shared `HunkAlignment` row map and does not need cross-view coordinate conversion.

### 14. Telemetry

Add a `CodeReviewTelemetryEvent::DiffLayoutChanged { from: DiffLayout, to: DiffLayout }` variant in `app/src/code_review/telemetry_event.rs`. `settings_view/code_page.rs` emits the event when the setting changes.

The Code Review host may emit a separate render-applied event if product wants adoption-by-opened-diff metrics, but v1 needs a single owner for the setting-change event to avoid duplicate telemetry.

Resolution: Settings -> Code is the telemetry owner for layout changes. Code Review render code should not emit a second setting-change event.

### 15. Accessibility

Side-by-side adds the following accessibility requirements:

- VoiceOver: the single editor view exposes two logical regions with accessible labels: "Original" for baseline and "Modified" for the post-diff pane.
- The modified pane is the only edit-focused region and the only region with a cursor.
- The baseline pane can receive focus for selection/copy if the platform accessibility API can express that without exposing edit actions.
- Aligned rows announce as a single logical row when read together: "Original: <text>; Modified: <text>".
- Keyboard navigation: `Tab` reaches the modified edit region; baseline focus is selection/copy only. `Cmd+Option+Left/Right`, if implemented, cycles the logical pane focus without moving edit ownership away from modified.
- Color contrast: gap-row backgrounds use a dedicated theme token, `diff.gap.background`, that meets 3:1 contrast against the editor background in both light and dark themes.

Open risk: if the platform accessibility tree cannot represent two logical panes inside one view with acceptable screen-reader behavior, implementation must escalate before shipping side-by-side beyond dogfood.

Resolution: accessibility coverage remains explicit even though v1 uses one `CodeEditorView`.

## Test plan

### Unit tests

- `app/src/code/hunk_alignment_tests.rs`:
  - Empty diff: row_map contains matching `Baseline(n)` / `Modified(n)` entries for every context row, no gaps.
  - Pure addition: rows for the added section are `(Gap { after_row }, Modified(m))`; baseline render state has matching gap rows.
  - Pure deletion: rows for the deleted section are `(Baseline(b), Gap { after_row })`; modified render state has matching gap rows.
  - Collapsed modification: `Context Delete Delete Add Add Context` produces four aligned rows: one context row, two paired modification rows, and one context row.
  - Context inside a delete/add sequence resets the pairing window.
  - Multi-hunk file: alignment composes correctly across hunks separated by unchanged context.
  - Large diff (5,000 lines, 200 hunks): completes in under 50ms.

- `app/src/code/editor/side_by_side_render_state_tests.rs`:
  - Pane content for `DiffType::Update`: baseline render state holds pre-diff content with delete decorations; modified render state holds post-diff content with add decorations.
  - Pane content for `DiffType::Create`: baseline render state is empty with gaps; modified holds the new file content.
  - Pane content for `DiffType::Delete`: modified render state is empty with gaps; baseline holds the original content.
  - `apply_diffs_if_any` is not used when `DiffLayout::SideBySide` is active.
  - Removed lines render only in `baseline_render_state`.
  - Added lines render only in `modified_render_state`.
  - Gap rows have the same rendered height on both sides.

- `app/src/code/editor/side_by_side_interaction_tests.rs`:
  - `editor()` returns the modified side in side-by-side mode.
  - `editor_for(Side::Baseline)` is read-only and does not expose an edit cursor.
  - `editor_for(Side::Modified)` is the only side registered with `FileModel`.
  - Baseline selection copies selectable content only.
  - Deleted-line ranges are not selectable.
  - Cursor movement affects the modified side only.
  - Layout switch from `Inline` to `SideBySide` and back preserves scroll position to within one row.

- `app/src/code/editor/side_by_side_scroll_tests.rs`:
  - Wheel delta updates the shared vertical scroll offset for both panes.
  - Cursor move on modified scrolls baseline to the corresponding row.
  - Cursor on a `(Gap { after_row }, Modified(m))` row (pure add) scrolls baseline to the next surrounding context line.
  - No cross-view mirrored scroll event is emitted.

### Integration tests

- `app/src/settings_view/code_page_tests.rs`:
  - The Code page renders `DiffLayoutWidget` when `SideBySideDiffLayout` is enabled.
  - The widget is hidden when the runtime flag is disabled.
  - Selecting "Side by side" writes `code.editor.diff_layout = "side_by_side"`.
  - The helper text names Code Review only.

- `app/src/code_review/code_review_view_tests.rs`:
  - Single-file diff in side-by-side renders both panes inside one `CodeEditorView`.
  - Multi-file diff: each file's editor honors the same layout.
  - Setting flip while open calls `LocalCodeEditorView` -> `CodeEditorView::set_diff_layout` for every visible editor and preserves scroll.
  - Find-in-diff matches both render states.
  - Hunk navigation (`f` / `F`) advances the focused hunk on both panes simultaneously while cursor remains on the modified side.
  - Comment thread on a baseline-side line renders under the baseline pane's row; modified pane shows the gutter marker.
  - Hidden-line expansion updates both render states from the same alignment model.

### Accessibility validation

- Snapshot tests assert the accessible labels "Original" and "Modified" for side-by-side panes inside the single editor view.
- Keyboard tests cover `Tab` and any pane-switching shortcut selected during implementation.
- Tests assert baseline focus does not expose edit actions or an insertion cursor.
- Theme tests assert `diff.gap.background` meets 3:1 contrast against editor backgrounds in light and dark themes.
- Manual screen-reader smoke test on macOS VoiceOver reads a paired modification row as one logical original/modified row.

### Manual smoke test

- macOS, M1 MacBook Air, dogfood build with `SideBySideDiffLayout` enabled:
  - Open Settings -> Code. Change "Diff layout" from "Inline" to "Side by side" and confirm the visible Code Review diff refreshes within 200ms.
  - Open Code Review with a 200-file diff. Confirm the active diff renders in two panes inside one editor view.
  - Scroll wheel on each pane. Confirm both panes scroll together without jitter.
  - Drag-select on the baseline pane. Confirm selectable content copies but deleted-line ranges are not selectable.
  - Drag-select on the modified pane. Confirm selection stays in the modified pane.
  - Confirm the cursor appears only in the modified pane.
  - Cmd-A on each pane. Confirm only that pane's selectable content is selected.
  - Resize the window narrow enough that side-by-side is cramped. Confirm horizontal scrollbars on each pane behave independently and the divider stays at 50%.
  - Open an AI block-list embedded diff and an inline banner diff. Confirm both still render inline in v1.
  - Linux (Ubuntu 24.04), Windows 11: repeat the settings toggle smoke test on each platform to confirm rendering and keybindings.

### Compile-parity checklist

Every site that destructures `DisplayMode` in a `match` must compile after the change. The current call sites include:

- `app/src/code/diff_viewer.rs`: trait helpers; unchanged because `DiffLayout` is a new orthogonal axis.
- `app/src/ai/blocklist/inline_action/code_diff_view.rs`: unchanged in v1; continues inline rendering.
- `app/src/ai/blocklist/block/view_impl/output.rs`: match on `DisplayMode::FullPane`; unchanged.

Every Code Review site that accesses the editor through `editor()` should be checked. Existing write, save, cursor, and accept/reject paths should keep using `editor()` because it returns the modified side. Rendering, comments, find, selection, and accessibility call sites should use side-aware render-state access where needed.

## Open questions

1. Confirm `CodeEditorView` + one `EditorWrapper` + two `RenderState`s is the right implementation shape, versus `SplitView` + two `CodeEditorView`s or `CodeEditorView` + two `EditorWrapper`s.
2. Confirm intra-component scroll sync is feasible inside one `EditorWrapper`, especially for hidden-line expansion and gap resizing, versus cross-view event mirroring.
3. Confirm screen-reader and tab-order requirements can be expressed inside a single view that exposes two logical panes.
4. Comment thread interaction with the opposite-pane gutter marker (Change 13) is non-interactive in this spec. Whether the marker should be clickable is a UX call for the Code Review SME and a candidate follow-up.
5. The segmented-control primitive used by `DiffLayoutWidget` needs SME confirmation. The current spec references existing settings segmented controls as precedent; the actual primitive name and import path should be confirmed during implementation review.
6. Resizable panel split is out of scope for the first ship per product Non-goals. Whether to revisit this later depends on telemetry and user feedback after the initial release.

## Revision notes

- v4 (this revision): narrowed v1 scope to Code Review only, deferred AI block-list and inline banner to v2, pivoted architecture from a separate side-by-side view type to `CodeEditorView` + one `EditorWrapper` + two `RenderState`s, clarified right-pane-only editing and cursor invariants, replaced ambiguous `line_index` with `RowIndex`, retargeted integration to `CodeReviewEditorState` / `LocalCodeEditorView`, and added open questions for @kevinyang372 architecture confirmation.
