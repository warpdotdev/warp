# Side-by-Side Diff Layout in the AI Block-List and Code Review Pane - Tech Spec
Product spec: `specs/GH7043/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/7043
Roadmap reference: https://github.com/warpdotdev/warp/issues/9233

## Context

The diff rendering surface is shared by the AI block-list inline diff (`InlineDiffView`) and the full Code Review pane (`code_review_view::CodeReviewView`). Both build on the same primitives:

- `app/src/code/diff_viewer.rs` defines the shared abstraction. `DisplayMode` carries the visual context: `FullPane`, `Embedded { max_height }`, and `InlineBanner { max_height, is_expanded, is_dismissed }`.
- `app/src/code/inline_diff.rs::InlineDiffView` wraps a single `CodeEditorView`, registers a `FileModel`-backed file when one exists, and applies parsed deltas to that editor on construction through `apply_diffs_if_any`.
- `app/src/code/editor/view.rs::CodeEditorView` and `app/src/code/editor/model.rs` are the rendering target. Both inline and side-by-side reuse them.
- `app/src/code/editor/diff.rs` holds the editor-level diff line decoration and gutter rendering. `DiffLineType` classifies lines as `Context`, `Add`, `Delete`, and `HunkHeader`.
- `app/src/code_review/diff_state.rs` holds hunk state, the `DiffMode` enum (`Head` / `MainBranch` / `OtherBranch(String)`, comparison base rather than layout), and the per-file diff state model.
- `app/src/code_review/comments/diff_hunk_parser.rs` parses hunks into ordered per-line records. The side-by-side aligner consumes those records; no new parser is introduced.
- `app/src/settings/code.rs` is the settings group for `code.*`. Settings are declared via `define_settings_group!` with `toml_path`, `default`, `supported_platforms`, and `sync_to_cloud` fields.
- `app/src/settings_view/code_page.rs` is the explicit Code settings UI. Declaring a settings entry does not render it; the page needs a concrete widget registered in the Code section.
- `crates/warp_features/src/lib.rs` defines the canonical `FeatureFlag` enum, `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, and changelog descriptions. `app/src/features.rs` re-exports the feature API.
- `app/src/lib.rs` builds the set of compiled-in feature flags through cfg-gated `FeatureFlag::Variant` entries. The corresponding Cargo feature declarations live in `app/Cargo.toml`.
- `app/src/code_review/scroll_preservation.rs` holds scroll preservation helpers that the side-by-side scroll-sync model can build on.
- `app/src/code_review/comments/comment.rs` and `comment_list_view.rs` own comment rendering. Comment placement gains a per-pane gutter marker; existing anchoring on `EditorLineLocation` remains.
- `app/src/code_review/telemetry_event.rs` defines `CodeReviewTelemetryEvent`. The layout-change event registers here.
- `app/src/code_review/find_model.rs` holds the find-in-diff state model that needs to traverse both panes in side-by-side.

The implementation introduces a `DiffLayout` enum (`Inline` / `SideBySide`), stores it as `code.editor.diff_layout`, exposes it in Settings -> Code, builds a `SideBySideDiffView` in `app/src/code/side_by_side_diff.rs`, and gates the path behind `SideBySideDiffLayout`.

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

`DiffLayout` is intentionally separate from `DisplayMode` because they are orthogonal:

- `DisplayMode` answers "where on screen does this diff live" (own pane vs embedded vs inline banner).
- `DiffLayout` answers "how do we render the diff content" (one column vs two columns).

Every `DisplayMode` honors the user's `DiffLayout` choice when the feature flag is enabled.

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
    description: "Layout for diff views: 'inline' or 'side_by_side'.",
},
```

The setting type system already supports enums via `serde`, matching how other setting groups carry strongly typed values. Default is `Inline`, so existing users are unaffected unless they opt in.

### 3. Settings page integration in `settings_view/code_page.rs`

`code.editor.diff_layout` is exposed in Settings -> Code under a new "Diff layout" subsection. This is the primary user entry point for the preference because it applies to all diff surfaces.

Add a `DiffLayoutWidget` row in `app/src/settings_view/code_page.rs`:

- The widget renders a two-option segmented control: "Inline" and "Side by side".
- The selected segment reads from `CodeSettings::DiffLayout`.
- Segment changes write `code.editor.diff_layout` through the settings store and emit the layout-change telemetry event.
- Register the widget explicitly in the Code settings group near the existing Code Review settings widgets. This resolves the CodeSettings rendering concern: declaring the setting alone does not make it appear in the page.
- Gate widget registration on `FeatureFlag::SideBySideDiffLayout.is_enabled()` so the control is hidden while the runtime flag is off.

The diff toolbar continues to own only per-view ephemeral controls, such as whitespace visibility. It does not expose `code.editor.diff_layout`.

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
   SideBySideDiffLayout => Some("Enables a side-by-side diff layout in the code review pane and AI block-list diffs."),
   ```

Rollout:

- Compile-time gate: `side_by_side_diff_layout` controls whether the app binary includes `FeatureFlag::SideBySideDiffLayout` in the compiled-in flag list.
- Runtime gate: the feature-flag service decides whether `FeatureFlag::SideBySideDiffLayout.is_enabled()` returns true for the current channel/user.
- Dispatch bridge: diff construction first checks the runtime flag. If disabled, it treats the effective layout as `DiffLayout::Inline` regardless of stored settings. If enabled, it reads `code.editor.diff_layout` and dispatches to `InlineDiffView` or `SideBySideDiffView`.
- Default rollout schedule: off in shipping builds -> 5% dogfood -> 25% dogfood -> 100% dogfood -> preview -> release.

### 5. Pane-aware diff viewer contract

Replace the single-editor assumption in `DiffViewer` with a pane-aware contract that keeps existing inline behavior but names side-by-side responsibilities explicitly:

```rust
pub trait PaneAwareDiffViewer {
    fn focused_editor(&self) -> &CodeEditor;
    fn modified_editor(&self) -> &CodeEditor;
    fn baseline_editor(&self) -> &CodeEditor;
    fn for_each_editor(&self, f: impl FnMut(&CodeEditor));
}
```

Semantics:

- `focused_editor` is the current focused pane, used for cursor, selection, focus, and local keyboard state.
- `modified_editor` is always the post-diff pane, used for `changed_lines`, accept, save, and file-backed edits.
- `baseline_editor` is always the pre-diff pane and is read-only.
- `for_each_editor` is used for operations that intentionally span both panes.

Caller routing:

| Caller / behavior | Pane-aware route |
|---|---|
| Cursor focus, local selection, copy, find focus | `focused_editor` |
| `changed_lines` | `modified_editor` |
| Accept diff, save diff, reject-to-modified-buffer operations | `modified_editor` |
| Hunk navigation | `for_each_editor` for lockstep scroll/cursor sync; the modified pane remains the source of writeable hunk state |
| Scroll preservation | `for_each_editor` |
| Comment rendering and gutter markers | `baseline_editor` and `modified_editor` via row alignment |

For `InlineDiffView`, all four methods return or iterate the same single editor, preserving current behavior.

### 6. Build `SideBySideDiffView`

Add `app/src/code/side_by_side_diff.rs`. The new type mirrors `InlineDiffView`'s lifecycle but holds two editors:

```rust
pub struct SideBySideDiffView {
    baseline: ViewHandle<CodeEditorView>,
    modified: ViewHandle<CodeEditorView>,
    diff_type: Option<DiffType>,
    file_path: Option<StandardizedPath>,
    alignment: HunkAlignment,
    scroll_sync: ScrollSyncModel,
    focused_pane: PaneId,
    backing_file_id: Option<FileId>,
    was_edited: bool,
    #[cfg(not(target_family = "wasm"))]
    is_new_file: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneId {
    Baseline,
    Modified,
}
```

Key methods:

- `new(baseline_editor, modified_editor, diff_type, display_mode, file_path, ctx)` constructs the view, subscribes to both editors' `CodeEditorEvent`, builds pane content, computes `HunkAlignment`, and pushes padding rows into both editors.
- `register_file(session_type, ctx)` registers only the modified editor with `FileModel`, mirroring `InlineDiffView::register_file`. The baseline pane is always read-only.
- `set_display_mode(mode, ctx)` applies display-mode behavior to the modified pane only. Baseline interaction state remains read-only regardless of display mode.
- `PaneAwareDiffViewer` is implemented with the routing from Change 5.

### 7. Pane content construction

Side-by-side does not reuse `apply_diffs_if_any`, because that function drives the inline model by mutating the edited editor and rendering removed lines as temp blocks.

The side-by-side pipeline is:

1. Parse the unified diff to `DiffHunk[]` using the existing parser. No parser changes are needed.
2. Build two `PaneBuffer` structs:
   - `baseline` contains pre-diff lines plus delete decorations.
   - `modified` contains post-diff lines plus add decorations.
3. Run hunk alignment over the ordered hunk lines. Each `AlignedRow` maps to a row index in both panes. Gap rows are zero-height in the source buffer but render as full-height empty rows with the appropriate gutter color.
4. Pass baseline content and delete decorations to the left editor. Pass modified content and add decorations to the right editor.
5. Apply padding rows from `HunkAlignment` so both panes have the same rendered row count.

`apply_diffs_if_any` remains reserved for `InlineDiffView`. Side-by-side never renders removed lines as temp blocks in the modified pane, and never loses delete decorations on the baseline pane.

### 8. Per-pane interaction state

Baseline pane:

- Always `read_only = true`.
- Supports text selection and copy.
- Does not consume keyboard edit events.
- Does not participate in file-backed save.

Modified pane:

- Follows the existing `DisplayMode` rules for the surface.
- In Code Review, it is read-only with accept/reject actions.
- In AI block-list `FullPane`, it is editable when the existing inline path would be editable.
- It is the only pane registered with `FileModel`.

`set_display_mode` on `SideBySideDiffView` applies the mode to the modified pane only. The baseline pane is hard-coded to read-only regardless of display mode so `FullPane` can never make baseline content editable.

### 9. Editor padding-row API

`CodeEditorView` and the underlying model gain a render-only padding-row concept: a row that takes up vertical space, has a gutter, and renders empty content. The public API:

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
    AddSide,
    DeleteSide,
}
```

Padding rows are inserted at render time only. They do not affect the underlying buffer, so save, find, copy, selection, and cursor navigation all see the real file content.

### 10. Hunk alignment

Add `app/src/code/hunk_alignment.rs`:

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
}

pub enum PaneLine {
    Line { buffer_line: usize, text: String },
    Gap,
}

pub struct HunkAlignment {
    pub baseline_padding: Vec<PaddingRow>,
    pub modified_padding: Vec<PaddingRow>,
    pub row_map: Vec<(Option<usize>, Option<usize>)>,
}

impl HunkAlignment {
    pub fn from_diff_hunks(hunks: &[DiffHunk]) -> Self;
}
```

The existing parsed-line enum upstream already contains ordered context/add/delete records. The aligner consumes `&[DiffHunk]`; existing inline-diff paths may keep using only the header data they already need.

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

- Pure insertion blocks produce rows with left `Gap` and right `Add`; the baseline pane receives delete-side padding rows.
- Pure deletion blocks produce rows with left `Delete` and right `Gap`; the modified pane receives add-side padding rows.
- A `Context` inside a modification resets the pairing window. Deletes before the context are flushed before adds after the context are considered.
- A delete run followed by an add run with unrelated text still pairs by position. Word-level highlighting is out of scope; v1 guarantees row alignment.

### 11. Scroll sync model

Add `app/src/code/scroll_sync.rs`:

```rust
pub enum ScrollSource {
    User,
    Mirrored { from: PaneId, sync_id: u64 },
}

pub struct ScrollSyncModel {
    focused_pane: PaneId,
    next_sync_id: u64,
}

impl ScrollSyncModel {
    pub fn on_scroll_wheel(&mut self, from: PaneId, delta_y: f32) -> MirroredScroll;

    pub fn corresponding_row(
        &self,
        from: PaneId,
        new_buffer_line: usize,
        alignment: &HunkAlignment,
    ) -> Option<usize>;
}
```

Every mirrored scroll carries a `ScrollSource::Mirrored { from, sync_id }` token. The receiving editor's scroll-event handler checks the token. If the token is `Mirrored`, the handler consumes the event and does not re-emit a cross-pane scroll. Only `ScrollSource::User` triggers cross-pane sync.

Invariant: `Mirrored -> Mirrored` events are dropped at the second subscriber, which breaks feedback loops and prevents scroll jitter.

Wheel events on either pane drive both panes by the same row delta. Cursor moves only move the cursor on the focused pane; the other pane scrolls to keep `corresponding_row` in view but does not move its own cursor.

### 12. Code Review pane integration

`app/src/code_review/code_review_view.rs` is the parent that holds per-file `InlineDiffView` instances today. The integration:

- On view construction, compute the effective layout. If `SideBySideDiffLayout` is off, use `Inline`. If it is on, read `code.editor.diff_layout`.
- Build `SideBySideDiffView` for each file when the effective layout is `SideBySide`; otherwise build `InlineDiffView`.
- Subscribe to setting updates. When `code.editor.diff_layout` changes, swap the per-file view with the same diff data and preserve scroll position via `app/src/code_review/scroll_preservation.rs`.
- Find-in-diff gains a `PaneId`-aware iterator that walks both editors when the active diff is side-by-side. Inline returns only the single modified editor.

The `code_review_view_integration.rs` and `code_review_view_tests.rs` fixtures gain a `with_layout(DiffLayout)` helper that sets the layout in the test settings store before constructing the view.

### 13. AI block-list integration

`app/src/ai/blocklist/inline_action/code_diff_view.rs` decides which `DisplayMode` to use per inline-action diff. The integration:

- Read the effective layout at the same place these `DisplayMode` decisions are made.
- For `DiffLayout::Inline`, construct `InlineDiffView` as today.
- For `DiffLayout::SideBySide`, construct `SideBySideDiffView` with the same `DisplayMode`.

`InlineBanner` currently uses `InlineDiffView`; under side-by-side mode it switches to `SideBySideDiffView` constrained to the banner's viewport, typically a single-hunk diff. Banner sizing follows the side-by-side pane heights with the existing collapse/expand affordances.

### 14. Comment threads

`app/src/code_review/comments/` anchors comment threads on `EditorLineLocation`. Comments stay attached to those locations and render under the targeted line. The integration:

- The renderer for a side-by-side row checks each comment's pane, captured from `CommentSide` in `app/src/ai/agent/action.rs`, and renders the thread under the matching pane's row.
- The opposite pane shows a small marker glyph in its gutter at the same row to indicate that the other side has a thread there. The marker is non-interactive in this spec.
- Multi-line comment ranges that span both deleted and added regions stay on the side they were authored against.

### 15. Telemetry

Add a `CodeReviewTelemetryEvent::DiffLayoutChanged { from: DiffLayout, to: DiffLayout, surface: TelemetrySurface }` variant in `app/src/code_review/telemetry_event.rs`. The `surface` enum distinguishes Code Review, AI block-list, inline banner, and Settings. `settings_view/code_page.rs`, `code_review_view`, and the AI inline-action diff host emit the event when the setting changes and the visible surface refreshes.

### 16. Accessibility

Side-by-side adds the following accessibility requirements:

- VoiceOver: each pane is its own scrollable region with an accessible label: "Original" for baseline and "Modified" for the post-diff pane.
- Aligned rows announce as a single logical row when read together: "Original: <text>; Modified: <text>".
- Keyboard navigation: `Tab` moves focus across panes; `Cmd+Option+Left/Right` cycles between panes; `Cmd+Option+Up/Down` jumps hunk-by-hunk in lockstep.
- Color contrast: gap-row backgrounds use a dedicated theme token, `diff.gap.background`, that meets 3:1 contrast against the editor background in both light and dark themes.

## Test plan

### Unit tests

- `app/src/code/hunk_alignment_tests.rs`:
  - Empty diff: row_map is `[(Some(0), Some(0)), ...]` for every line, no padding.
  - Pure addition: rows for the added section are `(None, Some(m))`; baseline padding has the matching count.
  - Pure deletion: rows for the deleted section are `(Some(b), None)`; modified padding has the matching count.
  - Collapsed modification: `Context Delete Delete Add Add Context` produces four aligned rows: one context row, two paired modification rows, and one context row.
  - Context inside a delete/add sequence resets the pairing window.
  - Multi-hunk file: alignment composes correctly across hunks separated by unchanged context.
  - Large diff (5,000 lines, 200 hunks): completes in under 50ms.

- `app/src/code/scroll_sync_tests.rs`:
  - Wheel delta on baseline drives both panes equally.
  - Cursor move on modified scrolls baseline to the corresponding row.
  - Cursor on a `(None, Some(m))` row (pure add) scrolls baseline to the next surrounding context line.
  - A `ScrollSource::Mirrored { from, sync_id }` event is consumed without re-emitting.
  - A mirrored event cannot trigger a second mirrored event.

- `app/src/code/side_by_side_diff_tests.rs`:
  - Pane content for `DiffType::Update`: baseline holds pre-diff content with delete decorations; modified holds post-diff content with add decorations.
  - Pane content for `DiffType::Create`: baseline editor is empty with padding; modified holds the new file content.
  - Pane content for `DiffType::Delete`: modified editor is empty with padding; baseline holds the original content.
  - `register_file` registers only the modified editor with `FileModel`; the baseline editor remains read-only.
  - `set_display_mode` affects the modified pane only; baseline remains read-only.
  - `accept_and_save_diff` writes the modified editor's buffer text via `FileModel`, identical to `InlineDiffView`'s save path.
  - `reject_diff` discards the modification and rebuilds the side-by-side view with identical panes.
  - Layout switch from `Inline` to `SideBySide` and back preserves scroll position to within one row.

### Integration tests

- `app/src/settings_view/code_page_tests.rs`:
  - The Code page renders `DiffLayoutWidget` when `SideBySideDiffLayout` is enabled.
  - The widget is hidden when the runtime flag is disabled.
  - Selecting "Side by side" writes `code.editor.diff_layout = "side_by_side"`.

- `app/src/code_review/code_review_view_tests.rs`:
  - Single-file diff in side-by-side renders both panes.
  - Multi-file diff: each file's view honors the same layout.
  - Setting flip while open swaps every visible file's view from `InlineDiffView` to `SideBySideDiffView` and preserves scroll.
  - Find-in-diff matches both panes.
  - Hunk navigation (`f` / `F`) advances the focused hunk on both panes simultaneously.
  - Comment thread on a baseline-side line renders under the baseline pane's row; modified pane shows the gutter marker.

- `app/src/ai/blocklist/inline_action/code_diff_view_tests.rs`:
  - Embedded AI diffs honor `SideBySide`.
  - `InlineBanner` diffs honor `SideBySide` and constrain the view to banner height.

### Accessibility validation

- Snapshot tests assert the accessible labels "Original" and "Modified" for side-by-side panes.
- Keyboard tests cover `Tab`, `Cmd+Option+Left/Right`, and `Cmd+Option+Up/Down`.
- Theme tests assert `diff.gap.background` meets 3:1 contrast against editor backgrounds in light and dark themes.
- Manual screen-reader smoke test on macOS VoiceOver reads a paired modification row as one logical original/modified row.

### Manual smoke test

- macOS, M1 MacBook Air, dogfood build with `SideBySideDiffLayout` enabled:
  - Open Settings -> Code. Change "Diff layout" from "Inline" to "Side by side" and confirm the visible diff refreshes within 200ms.
  - Open Code Review with a 200-file diff. Confirm the active diff renders in two panes.
  - Scroll wheel on each pane. Confirm both panes scroll together without jitter.
  - Drag-select on each pane. Confirm the selection stays in that pane.
  - Cmd-A on each pane. Confirm only that pane's content is selected.
  - Resize the window narrow enough that side-by-side is cramped. Confirm horizontal scrollbars on each pane behave independently and the divider stays at 50%.
  - Open an AI block-list embedded diff and an inline banner diff. Confirm both honor the chosen layout.
  - Linux (Ubuntu 24.04), Windows 11: repeat the settings toggle smoke test on each platform to confirm rendering and keybindings.

### Compile-parity checklist

Every site that destructures `DisplayMode` in a `match` must compile after the change. The current call sites include:

- `app/src/code/diff_viewer.rs`: trait helpers; unchanged because `DiffLayout` is a new orthogonal axis.
- `app/src/ai/blocklist/inline_action/code_diff_view.rs`: `DisplayMode` construction and matching; now layout-aware at construction.
- `app/src/ai/blocklist/block/view_impl/output.rs`: match on `DisplayMode::FullPane`; unchanged.

Every site that constructs `InlineDiffView` is a candidate for layout-aware view construction. Code Review and AI block-list integration cover the two parent surfaces.

## Open questions

1. Comment thread interaction with the opposite-pane gutter marker (Change 14) is non-interactive in this spec. Whether the marker should be clickable is a UX call for the Code Review SME and a candidate follow-up.
2. The segmented-control primitive used by `DiffLayoutWidget` needs SME confirmation. The current spec references `app/src/ui_components/tab_selector.rs` and the appearance theme picker as precedents; the actual primitive name and import path should be confirmed during implementation review.
3. Resizable panel split is out of scope for the first ship per product Non-goals. Whether to revisit this later depends on telemetry and user feedback after the initial release.

## Revision notes

- v3 (this revision): pivoted the layout control from a diff-local menu to Settings -> Code, added explicit `DiffLayoutWidget` rendering in `app/src/settings_view/code_page.rs`, removed the InlineBanner exception, made collapsed delete/add pairing the v1 hunk-alignment algorithm, replaced single-editor assumptions with `PaneAwareDiffViewer`, documented pane content construction and read-only baseline state, added mirrored-scroll suppression tokens, and added accessibility implementation and validation requirements.
