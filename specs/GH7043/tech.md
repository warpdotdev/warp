# Side-by-Side Diff Layout in the Code Review Pane - Tech Spec
Product spec: `specs/GH7043/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/7043
Roadmap reference: https://github.com/warpdotdev/warp/issues/9233

## Context

The v1 diff rendering surface is the Code Review pane only. AI block-list diffs and inline banner diffs continue to use the existing inline path until a v2 spec extends the layout setting to those surfaces.

Relevant Code Review and editor primitives:

- `app/src/code/editor/view.rs::CodeEditorView` is the rendering target. V1 side-by-side uses two editor-view instances: a select-only baseline view and the normal modified view.
- `app/src/code/editor/element.rs::EditorWrapper` owns editor rendering, layout, hit testing, gutters, and text shaping for each `CodeEditorView`. V1 keeps the editor internals mostly unchanged and puts cross-pane coordination in a parent bridge wrapper.
- `warp_editor::render::model::RenderState` holds the state used for shaped text, gutters, visible rows, and hit testing. Each editor view keeps its own render state.
- `app/src/code/editor/diff.rs` holds editor-level diff line decoration and gutter rendering. `DiffLineType` classifies lines as `Context`, `Add`, `Delete`, and `HunkHeader`.
- `app/src/code_review/diff_state.rs` holds hunk state, the `DiffMode` enum (`Head` / `MainBranch` / `OtherBranch(String)`, comparison base rather than layout), and the per-file diff state model.
- `app/src/code_review/comments/diff_hunk_parser.rs` parses hunks into ordered per-line records. The side-by-side aligner consumes those records; no new parser is introduced.
- `app/src/code_review/editor_state.rs::CodeReviewEditorState` owns Code Review editor state. The layout hook belongs here or in the equivalent shared wrapper, not in per-file `InlineDiffView` migration code.
- `app/src/code/local_code_editor.rs::LocalCodeEditorView` owns the local editor path that hosts Code Review editors. Layout flips route through this host into the side-by-side bridge wrapper when the Code Review path renders side-by-side.
- `app/src/settings/code.rs` is the settings group for `code.*`. Settings are declared via `define_settings_group!` with `toml_path`, `default`, `supported_platforms`, and `sync_to_cloud` fields.
- `app/src/settings_view/code_page.rs` is the explicit Code settings UI. Declaring a settings entry does not render it; the page needs a concrete widget registered in the Code section.
- `crates/warp_features/src/lib.rs` defines the canonical `FeatureFlag` enum, `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, and changelog descriptions. `app/src/features.rs` re-exports the feature API.
- `app/src/lib.rs` builds the set of compiled-in feature flags through cfg-gated `FeatureFlag::Variant` entries. The corresponding Cargo feature declarations live in `app/Cargo.toml`.
- `app/src/code_review/scroll_preservation.rs` holds scroll preservation helpers that the side-by-side scroll-sync model can build on.
- `app/src/code_review/comments/comment.rs` and `comment_list_view.rs` own comment rendering. Comment placement gains a per-pane gutter marker; existing anchoring on `EditorLineLocation` remains.
- `app/src/code_review/telemetry_event.rs` defines `CodeReviewTelemetryEvent`. The layout-change event registers here.
- `app/src/code_review/find_model.rs` holds the find-in-diff state model that needs to traverse both panes in side-by-side.

The implementation introduces a `DiffLayout` enum (`Inline` / `SideBySide`), stores it as `code.editor.diff_layout`, exposes it in Settings -> Code, and gates the Code Review path behind `SideBySideDiffLayout`.

Architecture choice: `DiffLayout::SideBySide` is implemented as two `CodeEditorView` instances wrapped by a Code Review bridge component. The baseline view renders base content in select-only mode. The modified view renders the global buffer entry for the working file. The bridge owns cross-pane synchronization for hidden lines, scroll position, find state, and shared diff state while keeping the two views' buffers separate.

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
- Dispatch bridge: Code Review construction first checks the runtime flag. If disabled, it treats the effective layout as `DiffLayout::Inline` regardless of stored settings. If enabled, it reads `code.editor.diff_layout` and renders either the inline editor path or the side-by-side bridge wrapper.
- Default rollout schedule: off in shipping builds -> 5% dogfood -> 25% dogfood -> 100% dogfood -> preview -> release.

Resolution: hidden flag state suppresses both the settings widget and the runtime layout path.

### 5. Side-by-side bridge wrapper

`DiffLayout::SideBySide` is rendered by a bridge wrapper that owns two `CodeEditorView` children:

```rust
pub struct SideBySideDiffBridge {
    baseline_view: CodeEditorView,
    modified_view: CodeEditorView,
    shared_diff_state: HunkAlignment,
    hidden_lines: HiddenLineRanges,
    scroll_anchor: SideBySideScrollAnchor,
    find_state: CodeReviewFindState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Baseline,
    Modified,
}
```

The bridge wrapper sits inside the existing Code Review editor host. It owns the side-by-side layout, renders the two editor views with equal widths and one divider, and propagates shared state changes in both directions.

Bridge-owned synchronization:

- **Hidden line ranges**: collapsed unchanged regions are represented once and applied to both views. Expanding hidden lines from either pane updates the shared hidden-line state and reconfigures both views so heights stay aligned.
- **Scroll position**: scrolling either pane updates a shared line anchor, then drives the other pane to the corresponding aligned row. Sync is line-anchored rather than pixel-based so font, wrapping, or line-height differences cannot accumulate drift.
- **Find state**: a single Code Review find session spans both views. Matches from either buffer are highlighted, and "Next" / "Prev" can traverse across panes.
- **Shared diff state**: both views read the same diff result and `HunkAlignment` so they agree on which baseline rows are deletions, which modified rows are insertions, and where visual gaps belong. This shared diff state is not a shared buffer.

The two buffers stay separate:

- The baseline view's buffer source is the base pre-edit content.
- The modified view's buffer source is the global buffer entry for the working file.
- Edits to the modified buffer re-trigger the diff. The refreshed diff updates both views' decoration config: newly deleted baseline rows remain regular selectable rows in the baseline view, while the modified view renders the corresponding baseline-only rows as gaps. Newly inserted modified rows render as regular rows in the modified view and gaps in the baseline view.

Resolution: the editor view internals stay close to the existing model. Side-specific behavior lives in each child editor view, and cross-pane coordination lives in the bridge.

### 6. Baseline and modified editor views

In `DiffLayout::Inline`, the existing Code Review editor path is used unchanged.

In `DiffLayout::SideBySide`, the bridge configures two editor views:

- The baseline view is a select-only `CodeEditorView` instance backed by base content.
- The modified view is a normal `CodeEditorView` instance backed by the global buffer entry for the working file.
- The modified view uses a decoration config that renders baseline-only rows as visual gaps instead of temporary deletion blocks.
- Text shaping, gutter rendering, hit testing, selection, and copy stay inside each child editor view.
- The bridge determines the pane from the event target and only forwards edit-capable events to the modified view.

Caller routing:

| Caller / behavior | Pane-aware route |
|---|---|
| Cursor focus, local selection, copy from modified side | Modified `CodeEditorView` |
| Copy from baseline side | Baseline `CodeEditorView` with selection-only focus |
| `changed_lines` | Modified `CodeEditorView` |
| Accept diff, save diff, reject-to-modified-buffer operations | Modified `CodeEditorView` |
| Hunk navigation | Bridge row alignment plus modified-side focus |
| Scroll preservation | Bridge scroll anchor |
| Comment rendering and gutter markers | Bridge `HunkAlignment` row map plus the targeted child view |

Resolution: right-pane-only editing is enforced by using a normal editor view only for the modified buffer. The baseline editor is select-only and never exposes an insertion cursor.

### 7. Pane content construction

Side-by-side reuses the existing unified-diff parser but does not reuse inline deleted-line rendering. The side-by-side pipeline is:

1. Parse the unified diff to `DiffHunk[]` using the existing parser. No parser changes are needed.
2. Build a shared diff state from the base content, current modified global buffer content, and ordered hunk lines.
3. Run hunk alignment over the ordered hunk lines. Each `AlignedRow` maps to a row index in both panes. Gap rows do not exist in either source file, but the bridge passes them to the appropriate view as decoration metadata.
4. Configure the baseline view with base buffer rows, delete decorations, hidden lines, and baseline-side alignment metadata.
5. Configure the modified view with the global buffer entry, add decorations, hidden lines, and a decoration config that renders baseline-only rows as visual gaps.

`apply_diffs_if_any` remains the inline path. When `DiffLayout::SideBySide` is active, the bridge uses the pane-content pipeline instead:

- Removed lines render as normal selectable rows in the baseline view.
- Added lines render as normal editable-buffer rows in the modified view.
- Baseline-only rows render as visual gaps in the modified view.
- Modified-only rows render as visual gaps in the baseline view.
- Inline temp-block deletion rendering is disabled for the modified side to prevent deletion bleed.
- Accept, reject, save, and changed-line computation continue to read the modified buffer, matching inline behavior.

Resolution: side-by-side keeps the baseline and modified buffers independent while sharing the diff state needed for aligned rendering.

### 8. Per-pane interaction state

Baseline pane:

- Always read-only.
- Uses a select-only `CodeEditorView` instance backed by base content.
- Supports text selection and copy on all baseline rows, including deleted-line ranges, because those ranges are real rows in the baseline buffer.
- Does not expose an insertion cursor.
- Does not consume keyboard edit events.
- Does not participate in file-backed save.
- Receives delete decorations and hidden-line configuration from the bridge.

Modified pane:

- Uses a normal `CodeEditorView` instance backed by the global buffer entry for the working file.
- Owns the cursor.
- Owns all writeable interactions.
- Follows the existing Code Review rules for accept, reject, save, revert, and hunk navigation.
- Is the only side registered with `FileModel`.
- Uses a decoration config that renders baseline-only rows as visual gaps instead of temporary deletion blocks.

The bridge applies Code Review interaction state to the modified pane and hard-codes baseline interaction state to read-only selection/copy. `FullPane` behavior from other surfaces is not part of v1.

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

The bridge stores the alignment result and passes each editor view only the row metadata for its side:

- `Baseline(n)` maps to a real shaped text row in the baseline view.
- `Modified(n)` maps to a real shaped text row in the modified view.
- `Gap { after_row }` maps to a full-height empty visual row with gutter and background metadata in the opposite view.
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

- Pure insertion blocks produce rows with left `Gap` and right `Add`; the baseline view receives gap decorations.
- Pure deletion blocks produce rows with left `Delete` and right `Gap`; the modified view receives gap decorations.
- A `Context` inside a modification resets the pairing window. Deletes before the context are flushed before adds after the context are considered.
- A delete run followed by an add run with unrelated text still pairs by position. Word-level highlighting is out of scope; v1 guarantees row alignment.

Resolution: `line_index` is not a free integer. The API uses `RowIndex` so baseline rows, modified rows, and render-only gaps are distinguishable, and the bridge exposes that aligned row map to both views.

### 10. Bridge scroll sync

Side-by-side scroll sync is owned by the bridge wrapper:

```rust
pub struct SideBySideScrollAnchor {
    focused_side: Side,
    anchor_row: RowIndex,
    anchor_side: Side,
    horizontal_scroll_by_side: BTreeMap<Side, ScrollOffset>,
}

impl SideBySideScrollAnchor {
    pub fn on_scroll_wheel(&mut self, side: Side, anchor_row: RowIndex);

    pub fn corresponding_row(
        &self,
        side: Side,
        row_index: RowIndex,
        alignment: &HunkAlignment,
    ) -> Option<RowIndex>;
}
```

Scroll sync is line-anchored. A wheel event or hunk navigation event in either view updates the shared row anchor, and the bridge asks the other view to reveal the corresponding row from `HunkAlignment`. Horizontal scroll remains per pane.

Cursor movement belongs to the modified side. When cursor navigation changes the modified row, `corresponding_row` computes the nearest baseline row for visibility but does not move a baseline cursor. Hunk navigation updates the shared scroll anchor and the modified-side cursor.

Resolution: the bridge synchronizes views by row anchors, not by matching pixel offsets. If implementation needs event source tagging to avoid recursive updates, that state belongs in the bridge.

### 11. Code Review pane integration

The Code Review path uses the existing Code Review editor host, not per-file `InlineDiffView` instances. The integration:

- Represent side-by-side editor state as either two `CodeReviewEditorState` slices or one slice with `baseline_sub_state` and `modified_sub_state`. The two child views must have distinct lifecycles even when the parent reducer entry point stays shared.
- On Code Review editor construction, compute the effective layout. If `SideBySideDiffLayout` is off, use `Inline`. If it is on, read `code.editor.diff_layout`.
- In inline mode, keep the existing single-editor path.
- In side-by-side mode, render the bridge wrapper inside whichever existing Code Review container component renders the editor today. If that container is `LocalCodeEditorView`, the bridge replaces the single `LocalCodeEditorView` instance for the side-by-side path.
- Subscribe to setting updates in the Code Review host. When `code.editor.diff_layout` changes, rebuild visible editors into the selected layout and preserve scroll position via `app/src/code_review/scroll_preservation.rs`.
- Find-in-diff gains a `Side`-aware iterator over both child editor views when the active diff is side-by-side. Inline returns only the modified side.
- Hidden-line expansion updates the bridge's shared hidden-line and alignment state, then applies the same collapsed ranges to both child views.

There is no `InlineDiffView` migration step in this architecture.

Resolution: this retargets the integration plan to the actual Code Review editor host and makes hidden-line sync a bridge-owned shared-state update.

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

Because both panes read the bridge's shared `HunkAlignment`, comment placement uses the shared row map and does not need ad hoc coordinate conversion between unrelated diff models.

### 14. Telemetry

Add a `CodeReviewTelemetryEvent::DiffLayoutChanged { from: DiffLayout, to: DiffLayout }` variant in `app/src/code_review/telemetry_event.rs`. `settings_view/code_page.rs` emits the event when the setting changes.

The Code Review host may emit a separate render-applied event if product wants adoption-by-opened-diff metrics, but v1 needs a single owner for the setting-change event to avoid duplicate telemetry.

Resolution: Settings -> Code is the telemetry owner for layout changes. Code Review render code should not emit a second setting-change event.

### 15. Accessibility

Side-by-side adds the following accessibility requirements:

- VoiceOver: the bridge exposes two logical regions with accessible labels: "Original" for baseline and "Modified" for the post-diff pane.
- The modified pane is the only edit-focused region and the only region with a cursor.
- The baseline pane can receive focus for selection/copy if the platform accessibility API can express that without exposing edit actions.
- Aligned rows announce as a single logical row when read together: "Original: <text>; Modified: <text>".
- Keyboard navigation: `Tab` reaches the modified edit region; baseline focus is selection/copy only. `Cmd+Option+Left/Right`, if implemented, cycles the logical pane focus without moving edit ownership away from modified.
- Color contrast: gap-row backgrounds use a dedicated theme token, `diff.gap.background`, that meets 3:1 contrast against the editor background in both light and dark themes.

Open risk: if the platform accessibility tree cannot represent the two bridged editor views with acceptable screen-reader behavior, implementation must escalate before shipping side-by-side beyond dogfood.

Resolution: accessibility coverage remains explicit for the two editor views and their bridge wrapper.

## Test plan

### Unit tests

- `app/src/code/hunk_alignment_tests.rs`:
  - Empty diff: row_map contains matching `Baseline(n)` / `Modified(n)` entries for every context row, no gaps.
  - Pure addition: rows for the added section are `(Gap { after_row }, Modified(m))`; the baseline view receives matching gap metadata.
  - Pure deletion: rows for the deleted section are `(Baseline(b), Gap { after_row })`; the modified view receives matching gap metadata.
  - Collapsed modification: `Context Delete Delete Add Add Context` produces four aligned rows: one context row, two paired modification rows, and one context row.
  - Context inside a delete/add sequence resets the pairing window.
  - Multi-hunk file: alignment composes correctly across hunks separated by unchanged context.
  - Large diff (5,000 lines, 200 hunks): completes in under 50ms.

- `app/src/code/editor/side_by_side_bridge_tests.rs`:
  - Pane content for `DiffType::Update`: baseline view holds pre-diff content with delete decorations; modified view holds post-diff content with add decorations.
  - Pane content for `DiffType::Create`: baseline view is empty with gaps; modified view holds the new file content.
  - Pane content for `DiffType::Delete`: modified view is empty with gaps; baseline view holds the original content.
  - `apply_diffs_if_any` is not used when `DiffLayout::SideBySide` is active.
  - Removed lines render as selectable rows in the baseline view and gaps in the modified view.
  - Added lines render as editable-buffer rows in the modified view and gaps in the baseline view.
  - Gap rows have the same rendered height on both sides.

- `app/src/code/editor/side_by_side_interaction_tests.rs`:
  - The baseline child view is read-only and does not expose an edit cursor.
  - The modified child view is the only side registered with `FileModel`.
  - Baseline selection copies selectable content, including deleted-line ranges.
  - Cursor movement affects the modified side only.
  - Layout switch from `Inline` to `SideBySide` and back preserves scroll position to within one row.

- `app/src/code/editor/side_by_side_scroll_tests.rs`:
  - Wheel delta updates the bridge scroll anchor and reveals the corresponding row in both panes.
  - Cursor move on modified scrolls baseline to the corresponding row.
  - Cursor on a `(Gap { after_row }, Modified(m))` row (pure add) scrolls baseline to the next surrounding context line.
  - Recursive scroll updates are suppressed by the bridge.

### Integration tests

- `app/src/settings_view/code_page_tests.rs`:
  - The Code page renders `DiffLayoutWidget` when `SideBySideDiffLayout` is enabled.
  - The widget is hidden when the runtime flag is disabled.
  - Selecting "Side by side" writes `code.editor.diff_layout = "side_by_side"`.
  - The helper text names Code Review only.

- `app/src/code_review/code_review_view_tests.rs`:
  - Single-file diff in side-by-side renders both `CodeEditorView` children inside the bridge.
  - Multi-file diff: each file's editor honors the same layout.
  - Setting flip while open rebuilds every visible editor into the selected layout and preserves scroll.
  - Find-in-diff matches both child editor views.
  - Hunk navigation (`f` / `F`) advances the focused hunk on both panes simultaneously while cursor remains on the modified side.
  - Comment thread on a baseline-side line renders under the baseline pane's row; modified pane shows the gutter marker.
  - Hidden-line expansion updates both child views from the bridge's shared alignment model.

### Accessibility validation

- Snapshot tests assert the accessible labels "Original" and "Modified" for side-by-side panes inside the bridge.
- Keyboard tests cover `Tab` and any pane-switching shortcut selected during implementation.
- Tests assert baseline focus does not expose edit actions or an insertion cursor.
- Theme tests assert `diff.gap.background` meets 3:1 contrast against editor backgrounds in light and dark themes.
- Manual screen-reader smoke test on macOS VoiceOver reads a paired modification row as one logical original/modified row.

### Manual smoke test

- macOS, M1 MacBook Air, dogfood build with `SideBySideDiffLayout` enabled:
  - Open Settings -> Code. Change "Diff layout" from "Inline" to "Side by side" and confirm the visible Code Review diff refreshes within 200ms.
  - Open Code Review with a 200-file diff. Confirm the active diff renders in two bridged editor views.
  - Scroll wheel on each pane. Confirm both panes scroll together without jitter.
  - Drag-select on the baseline pane. Confirm selectable content copies, including deleted-line ranges.
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

Every Code Review site that assumes one editor state should be checked. Existing write, save, cursor, and accept/reject paths should keep targeting the modified view. Rendering, comments, find, selection, and accessibility call sites should route through the bridge when side-by-side is active.

## Open questions

1. State management shape: do we keep `CodeReviewEditorState` as one slice with `baseline_sub_state` and `modified_sub_state`, or split into two parallel `CodeReviewEditorState` slices? Single-slice keeps the existing Code Review reducer signature; split-slice cleanly separates the two views' lifecycles.
2. Diff result ownership: does the bridge wrapper own the diff state directly, or is the diff state hoisted into the parent Code Review container? Bridge-owned diff is encapsulated; container-owned diff is reusable by other Code Review consumers.
3. Find state UX: when "Next match" crosses panes, does focus jump match-by-match, or does it stay in one pane until all matches there are visited? Confirm with @kevinyang372 whether this is designed behavior or a bridge-internal detail.
4. Comment thread interaction with the opposite-pane gutter marker (Change 13) is non-interactive in this spec. Whether the marker should be clickable is a UX call for the Code Review SME and a candidate follow-up.
5. The segmented-control primitive used by `DiffLayoutWidget` needs SME confirmation. The current spec references existing settings segmented controls as precedent; the actual primitive name and import path should be confirmed during implementation review.
6. Resizable panel split is out of scope for the first ship per product Non-goals. Whether to revisit this later depends on telemetry and user feedback after the initial release.

## Revision notes

- v4 (this revision): narrowed v1 scope to Code Review only, deferred AI block-list and inline banner to v2, restored the two `CodeEditorView` plus bridge-wrapper architecture, clarified right-pane-only editing and cursor invariants, replaced ambiguous `line_index` with `RowIndex`, retargeted integration to `CodeReviewEditorState` / `LocalCodeEditorView`, and updated open questions for the confirmed bridge direction.
