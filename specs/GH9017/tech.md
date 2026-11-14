# Word wrap (soft wrap) in the file editor and code review diff view — Tech Spec

Product spec: [`specs/GH9017/product.md`](product.md)

GitHub issue: https://github.com/warpdotdev/warp/issues/9017

Code references inspected at commit: `21f413b7971709f5345fdaa03bdbda30875db979`

## Context

The render engine already supports soft wrap; the file editor deliberately opts out of it, and the gutter's line index is derived from a counter that advances per *visual* row. Those two facts together are the whole problem: flipping the width setting alone produces wrapping with wrong line numbers and misaligned diff decorations. Two prior attempts stalled on exactly this — the POC #13069 (closed 2026-07-29) introduced a parallel `LogicalLineCount` dimension, and #15401 (closed 2026-08-24) recomputed the number from the buffer at the call site; #13192 (closed 2026-07-07) shipped wrap behind a flag and explicitly deferred the gutter issue as a known limitation.

Current system:

- [`crates/editor/src/render/model/mod.rs:179-183`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L179-L183) — `WidthSetting { FitViewport, InfiniteWidth }`. `FitViewport` is the existing soft-wrap mode and is `Default`.
- [`crates/editor/src/render/model/mod.rs:3403-3417`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L3403-L3417) — `RenderState::layout_context` is the single place the setting is consumed: `FitViewport` passes `self.viewport.width()` as the layout width, `InfiniteWidth` passes `f32::MAX`. Wrapping is therefore already implemented in text layout; nothing new is needed in the layout engine.
- [`crates/editor/src/render/model/mod.rs:2684-2687`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L2684-L2687) — `with_width_setting` is builder-only; there is no runtime setter, so the value cannot change after construction.
- [`app/src/code/editor/model.rs:341-350`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/model.rs#L341-L350) — `CodeEditorModel::new` constructs its `RenderState` `.with_width_setting(WidthSetting::InfiniteWidth)`. This one line is why no file editor or diff pane ever wraps.
- [`crates/editor/src/render/model/mod.rs:2694-2696`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L2694-L2696) — `container_scrolls_horizontally()` is `matches!(width_setting, InfiniteWidth)`; it is fed into `TextLayout` and governs whether wide blocks (e.g. Markdown tables) render at full intrinsic width. It must keep tracking the width setting, so tables keep their own horizontal scroll when the editor wraps.
- [`crates/editor/src/render/model/mod.rs:3826-3829`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L3826-L3829) — horizontal autoscroll is already suppressed under `FitViewport`, so no change is needed for behavior 6's cursor half.
- [`crates/editor/src/render/model/mod.rs:2999-3013`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L2999-L3013) — `set_viewport_size` emits `RenderEvent::NeedsResize` when `size_info.needs_layout`. This is the existing hook for behavior 5 (re-wrap on resize).
- [`crates/editor/src/render/model/mod.rs:4194-4198`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L4194-L4198) — `start_line_index` returns `block_at_offset(offset)?.start_line`.
- [`crates/editor/src/render/model/mod.rs:4924-4948`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L4924-L4948) and [`:5440-5458`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod.rs#L5440-L5458) — **the root cause.** `start_line` is accumulated with `*line_acc += LineCount(1)` **per laid-out line of a paragraph's `TextFrame`**, and per paragraph with `+= paragraph.lines()`. Under `InfiniteWidth` one logical line always produces one frame line, so `start_line` coincides with the source line number. Under `FitViewport` a wrapped logical line produces N frame lines, so `start_line` becomes a *visual row index* and every following line's number is inflated.
- [`app/src/code/editor/element.rs:590-624`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/element.rs#L590-L624) — `gutter_elements` takes `model.start_line_index(&**block)` as `line_count` and uses it for **three** distinct purposes: the rendered gutter number (`absolute_line_number` / `display_line_number`), the diff-hunk lookup `self.diff_status.diff_hunk(line_count, …)`, and removal-hunk range lookup `removed_diff_range(line_count)`. All three break together when the value becomes visual.
- [`app/src/code/editor/element.rs:730-742`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/element.rs#L730-L742) — hidden-section blocks read `model.line_range(&**block)`, which is `start_line_index` plus the block's own line count.
- [`app/src/code/editor/diff.rs:188-226`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/diff.rs#L188-L226) — `diff_hunk` and `removed_diff_range` key `deletion_mapping` / `change_mapping` by **source** line number. These maps are produced by the diff, independent of layout, which is why a visual `line_count` silently mismatches them.
- [`app/src/code/editor/element.rs:353-370`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/element.rs#L353-L370) — `LineNumberConfig::absolute_line_number` / `display_line_number` convert a `LineCount` to the printed number, including relative mode.
- [`app/src/code/editor/view.rs:1256-1262`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/view.rs#L1256-L1262) — `active_cursor_line_for_line_numbers` **already** derives its `LineCount` from the buffer (`selection.head.to_buffer_point(buffer)`, row minus one for the zero-based convention), not from the render model. This is the existing precedent for the fix below, and it means relative mode's anchor is already wrap-independent.
- [`crates/editor/src/content/buffer.rs:6164-6178`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/content/buffer.rs#L6164-L6178) — `ToBufferPoint for CharOffset` is the O(log n) sumtree conversion that precedent uses.
- [`app/src/code/editor/model.rs:2098-2100`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/model.rs#L2098-L2100) and [`:2028-2048`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/model.rs#L2028-L2048) — go-to-line already works in buffer `Point` space (`line_count` from `buffer.max_point().row`, `jump_to_line_column` via `Point::new(line, col).to_buffer_char_offset`), so behavior 11 needs no change.
- [`app/src/code/editor/view.rs:212-256`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/view.rs#L212-L256) — `CodeEditorRenderOptions` is the per-surface construction knob (`lazy_layout`, `line_height_override`, providers), and [`:305-315`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/view.rs#L305-L315) shows `CodeEditorView` already subscribing to `AppEditorSettings` and re-notifying on change.
- [`app/src/settings/code.rs`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/settings/code.rs) — `define_settings_group!(CodeSettings, …)` already owns editor-scoped booleans with `toml_path: "code.editor.*"` (`format_on_save`, `auto_save`, `show_project_explorer`).
- [`app/src/settings_view/code_editor_review_page.rs:282-330`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/settings_view/code_editor_review_page.rs#L282-L330) — `init_actions_from_parent_view` registers each toggle as a `ToggleSettingActionPair` bound to a `flags::*` context flag, and [`:386-400`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/settings_view/code_editor_review_page.rs#L386-L400) shows the `render_body_item` + switch pattern for the row itself. `flags::*` constants live in [`app/src/settings_view/mod.rs:493-505`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/settings_view/mod.rs#L493-L505).
- [`app/src/notebooks/file/mod.rs:1115-1141`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/notebooks/file/mod.rs#L1115-L1141) — the Markdown viewer's **Raw** mode is not a separate renderer: it emits `PaneEvent::ReplaceWithCodePane`, i.e. it *is* the code editor. Fixing the code editor fixes Raw Markdown with no notebook-side change.
- [`crates/editor/src/render/model/mod_tests.rs`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/crates/editor/src/render/model/mod_tests.rs), [`app/src/code/editor/element_tests.rs:20-35`](https://github.com/warpdotdev/warp/blob/21f413b7971709f5345fdaa03bdbda30875db979/app/src/code/editor/element_tests.rs#L20-L35) — existing sibling-file test homes (`script/check_no_inline_test_modules` forbids inline `mod tests`).

No new renderer, layout algorithm, persistence format, or network API is required.

## Proposed changes

### 1. Make the gutter's line index logical, independent of the wrap setting — first, and on its own

This is the load-bearing change and the one both prior attempts had to invent. Land it **before** any wrap is enabled, so it is verifiable as a no-op refactor under today's `InfiniteWidth` behavior.

Add a private helper on `EditorWrapper` in `app/src/code/editor/element.rs` that converts a block's buffer offset to a zero-based logical line index, mirroring the existing precedent in `active_cursor_line_for_line_numbers`:

```rust
/// The 0-indexed logical (source) line containing `offset`, from the buffer's own
/// line structure. Deliberately independent of `RenderState`'s `start_line`, which
/// counts wrapped *visual* rows: with word wrap on, one source line spans several
/// rows and would otherwise inflate every following gutter number. Diff hunks
/// (`DiffStatus`) are keyed by source line numbers too, so this keeps gutter
/// numbering and hunk matching correct in both wrap modes.
fn logical_line_number(&self, offset: CharOffset, app: &AppContext) -> LineCount
```

In `gutter_elements`, replace the `model.start_line_index(&**block)` lookup with this helper over `block.viewport_item().block_offset`. Note the control-flow change: `start_line_index` returns `Option` and the loop `continue`s on `None`, while the buffer conversion is total. Blocks whose offset no longer resolves in the render model (the `block_at_offset` invalidation path) must keep being skipped, so retain a skip for blocks with no resolvable viewport item rather than silently numbering a stale block.

For hidden sections at `element.rs:730-742`, keep taking the *length* from `model.line_range(&**block)` (hidden blocks render no text, so their length in lines is wrap-invariant) but rebase the range's start onto the logical number: `logical..logical + (render_range.end - render_range.start)`.

Rejected alternative: a parallel `LogicalLineCount` sumtree dimension threaded through the render model (the #13069 POC). It is strictly more machinery for the same answer the buffer already holds in O(log n), and it adds a second source of truth for "what line is this" that can drift from `DiffStatus`'s keys. Deriving from the buffer keeps exactly one.

Also rejected: leaving the gutter alone and documenting the jump as a known limitation (#13192). Behavior 7 and 9 are the reason this issue's prior attempts did not land; shipping wrap with wrong line numbers and misaligned hunks converts a readability win into a code-review correctness bug.

Note what the diff view is **not**: it is a single unified column. `compute_unified_diff` (`app/src/code/editor/diff.rs:92-130`) produces a git-style unified diff, and removed lines are rendered as inline `TemporaryBlock`s in the same render tree as the new lines — there is no split side-by-side layout and therefore no left/right row-pairing to preserve under wrap. Wrapped rows in a hunk stay contiguous because every row of the hunk, including the temporary removal blocks, wraps against the same pane width in the same column. Temporary blocks enter the gutter loop like any other block (`element.rs:590-624`), so change 1's logical numbering covers them without a separate mechanism. This is why the tech spec needs no diff-specific layout work at all; the only diff-side requirement is that hunk decorations resolve against logical lines (change 1 + test 4).

### 2. Add a runtime width-setting setter to `RenderState`

In `crates/editor/src/render/model/mod.rs`:

- Derive `Debug, Clone, Copy, PartialEq, Eq` on `WidthSetting` (currently only `Default`) so it can be compared and carried in view state.
- Add `pub fn set_width_setting(&mut self, new: WidthSetting) -> bool` returning whether the value changed. It only assigns; because the render model does not own the content model, **the caller is responsible for re-laying out** the affected range when it returns `true`.

Keep `with_width_setting` for construction and keep `container_scrolls_horizontally()` defined in terms of the setting, so wide tables continue to opt out of viewport-fitting when the editor is unwrapped and gain the container's scroll when it is not.

### 3. Give `CodeEditorModel` a soft-wrap toggle

In `app/src/code/editor/model.rs`:

- Add `pub fn set_soft_wrap(&mut self, enabled: bool, ctx: &mut ModelContext<Self>)` that maps `true → FitViewport`, `false → InfiniteWidth`, calls `RenderState::set_width_setting`, and on `true` (changed) triggers a full relayout over the buffer range via the existing `layout_edit_delta` path, then notifies.
- Keep the constructor's default at `InfiniteWidth` so every existing surface (comment editor, inline AI blocks, find-references cards — product-spec behavior 16) is unaffected unless a caller opts in.

Add an opt-in on the per-surface construction knob rather than a global: `CodeEditorRenderOptions::with_soft_wrap_setting()` (or an equivalent builder method) marking a surface as *governed by the user setting*. Only `app/src/code/view.rs`'s file-editor construction sites and the code-review/diff construction sites set it. This is what confines the feature to the two surfaces in the product spec and keeps behavior 16 true by construction rather than by review vigilance.

### 4. Persist the preference and react to changes

Add to `define_settings_group!(CodeSettings, …)` in `app/src/settings/code.rs`, alongside `format_on_save` / `auto_save`:

```
word_wrap: WordWrap {
    type: bool,
    default: false,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "code.editor.word_wrap",
    description: "Whether long lines wrap to the editor width instead of scrolling horizontally.",
}
```

The setting must reach an editor through **two** paths, not one:

- **At construction.** `CodeEditorView::new` (`view.rs:291-320`) reads the current `CodeSettings` when the surface is governed by the setting and passes the initial width setting into `CodeEditorModel` construction, so a pane opened or restored with wrap already on renders wrapped from its first frame. A subscription-only wiring would show every restored pane unwrapped until the next setting change.
- **On change.** `CodeEditorView` already subscribes to `AppEditorSettings` (`view.rs:305-315`); add the analogous `CodeSettings` subscription for governed surfaces, and on change call `model.set_soft_wrap(...)`.

Behavior 5 (re-wrap on resize) is served by the existing `RenderEvent::NeedsResize` emission from `set_viewport_size`; ensure the governed surfaces relayout on that event when wrap is active, and keep the existing debounce so a drag-resize does not relayout per frame.

### 5. Settings row and keyless editable action

- Add a **Word wrap long lines** row to the Editor and Code Review page using the existing `render_body_item` + switch pattern, with a `flags::WORD_WRAP_FLAG` context flag and a `ToggleSettingActionPair` registration in `init_actions_from_parent_view`, matching the surrounding rows exactly.
- Register one payload-free editable action `"Toggle word wrap"` with **no** default key binding (no `with_*_key_binding` call), which automatically surfaces it in Settings → Keyboard Shortcuts and the Command Palette. Users bind Alt+Z themselves; we do not claim a shortcut that conflicts on any platform.

### 6. Explicit non-changes

Go-to-line (`model.rs:2028-2048`, `2098-2100`) and relative-mode's cursor anchor (`view.rs:1256-1262`) already operate in buffer `Point` space and require no edit — but both are covered by tests below, because change 1 makes them share a definition of "line" with the gutter, and a future regression there would be silent.

## Testing and validation

### Unit tests

1. `crates/editor/src/render/model/mod_tests.rs` — `set_width_setting` returns `true` only on a real change and is idempotent in both directions (guards the relayout trigger in change 3 from firing spuriously or not at all).
2. `app/src/code/editor/element_tests.rs` — table-driven `logical_line_number` coverage on a buffer with (a) no long lines, (b) one line wrapping to several rows, (c) several consecutive wrapping lines, asserting the number for every following line is identical in both wrap modes. Directly pins invariants 7 and 8. The wrapped case must use a line long enough to wrap at the test viewport width *derived from that width*, and an independent literal expectation for the resulting numbers, so the test cannot pass by construction if wrapping silently stops happening.
3. `app/src/code/editor/element_tests.rs` — gutter numbers with `starting_line_number` set (lens/embedded editors) are unchanged by wrap, covering the existing offset path.
4. `app/src/code/editor/diff` tests — `diff_hunk(logical_line)` and `removed_diff_range(logical_line)` resolve to the same hunks with wrap on as with wrap off, for an addition, a deletion, and a replacement hunk where at least one line wraps. Pins invariant 9.
5. Construction-path test: a governed editor constructed with the persisted setting on lays out `FitViewport` from the first frame (no unwrap-then-rewrap flash); one constructed with it off lays out `InfiniteWidth`. Pins the initialization half of change 4.

6. Buffer-identity test: toggling `set_soft_wrap` on and off around an unmodified buffer leaves the buffer contents and version byte-identical, and a save after wrapping produces identical bytes. Pins invariant 4.
7. Hidden-section test: a collapsed section preceded by a wrapping line reports the same line range in both modes. Pins invariant 14.

### Integration tests

7. A GUI integration test that opens a file with long lines, asserts a horizontal scrollbar is present, toggles the setting, and asserts the scrollbar is gone and content height increased while the gutter's last number is unchanged (invariants 2, 3, 6, 7).
8. Resize test: with wrap on, shrinking the pane increases rendered row count and re-wraps rather than preserving stale break points (invariant 5).
9. Cursor navigation: pressing Down from a wrapped visual row that is not the last row of its logical line moves within the same logical line (invariant 12).

### Manual testing (per CONTRIBUTING)

Before/after screenshots for: a long-lined Markdown file in Raw mode, the same file with wrap on showing unchanged gutter numbers, a diff with a wrapping changed line showing correctly aligned hunk decoration, and the Settings row. A narrated recording covering the toggle action from the Command Palette, a live pane resize, and go-to-line landing on the same content with wrap on and off.

## Risks

- **Relayout cost on large files.** Turning wrap on forces a full relayout. Mitigate by reusing the existing lazy-layout and `NeedsResize` debounce paths rather than adding a new eager relayout; measure on a multi-MB file before merge and, if needed, keep the pre-existing lazy-layout behavior in charge of when work happens.
- **Wide Markdown tables in the editor.** `container_scrolls_horizontally()` flips with the width setting, changing how tables lay out inside a wrapped editor. Verify a table-containing Markdown file in Raw mode explicitly; this is the interaction most likely to look wrong while every line-number test passes.
- **Surface creep.** The opt-in in change 3 is what keeps comment editors and inline AI blocks out. A future caller adding the opt-in without reading the product spec would silently widen the feature; the constructor default being `InfiniteWidth` keeps that from happening by accident.

## Follow-ups (out of scope)

- Rendered Markdown preview wrap width (#10527).
- Terminal output wrapping (#8106, #4094).
- The TUI editor, which shares `CodeEditorModel` via `new_tui` but has its own char-cell layout path and no settings surface here.
