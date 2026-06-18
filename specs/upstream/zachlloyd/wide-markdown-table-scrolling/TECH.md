# Wide Markdown Table Scrolling — Tech Spec
Product spec: `specs/zachlloyd/wide-markdown-table-scrolling/PRODUCT.md`

## Context
The branch implements the behavior described in the sibling `PRODUCT.md`: wide Markdown tables scroll horizontally inside their own viewport, and selection / caret / hit-testing / copy stay attached to content while the table or any clipped surface is scrolled.

Investigation against `crates/markdown_parser/examples/table-demo/all_test_cases.md` surfaced two classes of bugs that both needed to be addressed:

- Editor-backed tables did not yet own a horizontal viewport, so wide tables simply overflowed the surrounding editor.
- Once a cell contained inline Markdown (`**Bold**`, `*Italic*`, links), source offsets and rendered offsets were mixed in selection and copy paths, producing drift even before the wide-table work.

Separately, the shared clipped-scrollable path could re-use stale screen-space coordinates after a scroll, which showed up as wrong word selection and wrong copied text.

### Relevant code
- `crates/editor/src/render/model/mod.rs` — `LaidOutTable` (horizontal scroll state, reveal logic, character bounds) and `RenderState::autoscroll` (table-aware reveal)
- `crates/editor/src/render/element/table.rs` — `RenderableTable` painting, clipping, and event dispatch for scroll/drag/hover
- `crates/editor/src/render/model/location.rs` — hit-testing that accounts for local table scroll
- `crates/editor/src/render/model/table_offset_map.rs` — `TableOffsetMap` (table-wide) and `TableCellOffsetMap` (per-cell source↔rendered)
- `crates/editor/src/render/layout.rs` — `TextLayout::with_container_scrolls_horizontally` carries the carve-out flag
- `crates/editor/src/content/text.rs` — `BufferBlockStyle::Table` variant, `TableCache` / `TableBlockCache`, and `table_cell_offset_maps`
- `crates/editor/src/content/edit.rs` — `layout_table_block`, `measure_table_cells`, per-cell size clamp
- `crates/editor/src/content/buffer.rs` — table-aware clipboard text extraction and partial-table HTML filtering
- `crates/warpui_core/src/elements/shared_scrollbar.rs` — shared `ScrollbarAppearance` / `ScrollbarGeometry` and scrollbar math
- `crates/warpui_core/src/elements/clipped_scrollable.rs` — selection anchor (`anchor_and_adjust_selection_for_scroll`)
- `crates/warpui_core/src/elements/new_scrollable/mod.rs` — dispatches selection APIs through the anchor helper
- `crates/warpui_core/src/elements/formatted_text_element.rs` — horizontal bounds check for smart selection
- `app/src/notebooks/editor/mod.rs` — notebook table appearance (scrollbar colors, etc.)

## Proposed changes

### Shared scrollbar primitives in `warpui_core`
`crates/warpui_core/src/elements/shared_scrollbar.rs` is the single source of truth for:
- `ScrollbarAppearance` / `ScrollbarGeometry` (overlay scrollbar geometry, thumb bounds, track bounds)
- Minimum thumb sizing (`MIN_SCROLLBAR_THUMB_LENGTH`)
- `compute_scrollbar_geometry(...)`
- `project_scroll_delta_by_sensitivity(...)` for axis projection of mixed-axis gestures
- `scroll_delta_for_pointer_movement(...)` for pointer-drag-to-scroll translation

Both the editor table renderer and the existing `new_scrollable` utilities call through these helpers. The goal is not to nest a `NewScrollable` inside tables but to share scrollbar math and mixed-axis resolution.

### Editor tables own a local horizontal viewport
`LaidOutTable` in `crates/editor/src/render/model/mod.rs` gains local horizontal scroll state:
- `scroll_left: Cell<Pixels>`
- `TableScrollbarInteractionState` (drag state, hovered)
- `viewport_width()`, `max_scroll_left()`, `set_scroll_left()`, `scroll_horizontally()`
- `reveal_offset()` for keyboard/caret autoscroll
- `horizontal_scroll_allowed` flag (see carve-out below)

`RenderableTable::paint` in `crates/editor/src/render/element/table.rs`:
- Computes a clipped viewport rectangle for the visible region.
- Translates painted content by `scroll_left`.
- Paints inside a clipped layer.
- Derives the thumb via `compute_scrollbar_geometry()` and paints it using notebook theme colors from `MarkdownTableAppearance` / `TableStyle`.

`RenderableTable::dispatch_event` handles `LeftMouseDown` (thumb + gutter), `LeftMouseDragged`, `LeftMouseUp`, `MouseMoved`, and `ScrollWheel`. Two event-propagation rules worth calling out:
- The `ScrollWheel` handler returns the boolean result of `scroll_horizontally()` rather than unconditionally consuming the event. When the table is pinned at an edge, the event falls through to the surrounding vertical scroller (PRODUCT invariant 11).
- The `MouseMoved` handler returns `false` regardless of whether the pointer is over the thumb; scrollbar hover state is still updated via `set_scrollbar_hovered()` and `ctx.notify()`, but the event continues to propagate to downstream handlers (PRODUCT invariant 12).

### Horizontally scrollable container carve-out
Table-local horizontal scrolling only applies when the surrounding surface doesn't own horizontal scroll. In code editors using `WidthSetting::InfiniteWidth`, wide tables render at full intrinsic width:
- `TextLayout` carries a `container_scrolls_horizontally` flag, set via `TextLayout::with_container_scrolls_horizontally(...)`.
- `RenderState::container_scrolls_horizontally()` returns `true` when `width_setting == WidthSetting::InfiniteWidth`; `RenderState::layout_context()` and `TextLayout::from_layout_context(...)` propagate the flag.
- `layout_table_block()` reads the flag and sets `horizontal_scroll_allowed` on the resulting `LaidOutTable` to its inverse.
- When `horizontal_scroll_allowed == false`: `viewport_width()` returns the full table width, `max_scroll_left()` returns zero, and `scroll_left()` / `scroll_horizontally()` / `reveal_offset()` / `set_scroll_left()` become no-ops. Paint widens the clip layer to the full table, `table_scrollbar()` returns `None`, and scrollbar drag/hover handling short-circuits. Wheel events fall through to the surrounding editor.

AI block list contexts keep the default `false`, preserving existing behavior.

### Per-cell source↔rendered offset mapping
`TableCellOffsetMap` in `crates/editor/src/render/model/table_offset_map.rs` tracks source↔rendered ranges per inline fragment. For each fragment it records rendered start/end, visible source start/end, and source end including Markdown markers. It supports `rendered_length()`, `source_length()`, `rendered_to_source()`, and `source_to_rendered()`.

`TableCellOffsetMap::from_inline_and_source(source, inline)` derives spans by walking the raw cell `source` character-by-character alongside each fragment's rendered text, rather than reconstructing marker lengths from style flags. For each fragment:
- Advance through source until the fragment's first rendered character is found. Intervening source characters are attributed to this fragment as markers.
- Consume the remaining rendered characters from source, treating `\<punct>` as a single-character escape (two source chars → one rendered char).

This replaces an earlier hardcoded `fragment_source_marker_lengths` helper and therefore:
- Handles backslash-escaped punctuation correctly (previously silently drifted).
- Stays correct if the Markdown parser ever changes marker syntax, because we walk actual source rather than regenerating it.
- Handles nested styles where adjacent fragments share outer markers (e.g. `**a *b* c**`), attributing each marker once.

`content::text::table_cell_offset_maps(table, source)` receives the parsed `FormattedTable` and the raw tab/newline-separated source, splits the source into rows/cells, and passes each cell's source through the builder. Synthetic cells added by `normalize_shape` get an empty source string and produce empty maps.

An in-code `TODO` above `impl TableCellOffsetMap` notes that moving cell/row boundaries into the `SumTree` with new `BufferText` marker types would let editable tables derive per-cell offsets by seeking to boundaries rather than reparsing. That refactor is deferred to the editable-tables workstream.

### Table layout is source-based, rendering stays rendered-text-based
`layout_table_block()` in `crates/editor/src/content/edit.rs`:
- Parses the table into formatted inline fragments (via the cache below when available).
- Builds `TableCellOffsetMap`s for every cell.
- Builds the table-wide `TableOffsetMap` from cell **source** lengths.
- Preserves `content_length` from the original text block source.

`LaidOutTable` stores the per-cell maps and uses them whenever a source offset touches rendered geometry (coordinate→offset conversion, link lookup, relative character bounds, selection highlight ranges, cursor placement).

The rule: editor-facing APIs stay source-based; text layout, frame widths, and painting stay rendered-text-based; conversions happen explicitly at the cell boundary.

### Hit-testing, selection, and caret geometry
- `crates/editor/src/render/model/location.rs` adds `scroll_left` before calling `coordinate_to_offset()`.
- `LaidOutTable::character_bounds()` subtracts `scroll_left` when returning screen-space bounds.
- `RenderState::autoscroll()` calls `reveal_autoscroll_offsets_in_tables()` so keyboard movement horizontally reveals the active caret/selection inside the table.
- `softwrap_point_to_offset()` for tables resolves to the first visible character in the row instead of using raw row starts.
- `crates/editor/src/render/element/mod.rs` hit-testing uses the editor element's own layer bounds rather than the clipped child layer bounds, so clicks inside a wide table are not rejected before block-level hit-testing runs.

### Per-cell maximum content width
`crates/editor/src/content/edit.rs` introduces:
- `MAX_TABLE_CELL_CONTENT_WIDTH_PX` = 500.0 — maximum content width (exclusive of cell padding).
- `maximum_table_cell_width(table_style)` helper, mirroring `minimum_table_cell_width(..)`.

In `measure_table_cells`, per-cell measured widths are clamped with both `.max(minimum_table_cell_width(..))` and `.min(maximum_table_cell_width(..))` before they fold into the shared column width. The second layout pass in `layout_table_block` lays each cell at `cell_content_width = column_width - cell_padding * 2.0`, and the existing text layout soft-wraps the clamped cells without further changes.

The cap is applied unconditionally — including when `horizontal_scroll_allowed == false` — so infinite-width containers still get readability benefits. Single very long unbreakable tokens (e.g. long URLs) may still render wider than the cap because soft-wrap can't break them; the column width stays clamped and the token visually overflows inside the clipped table region.

### Clipboard behavior is table-aware
`crates/editor/src/content/buffer.rs`:
- Plain-text copy walks selected block segments and routes table segments through `clipboard_table_text_in_range()`.
- Table copy rebuilds the formatted table (from cache when available), computes source-based cell ranges, converts the selected source span in each cell to rendered offsets, and slices visible cell text accordingly.
- Partial table selections return rendered plain text with tabs/newlines preserved.
- HTML export in `selected_text_as_html` filters only the ranges that contain a partial table selection, serializing the remaining clean ranges to HTML normally. Only when every range is a partial-table range does it return `None`.

### Clipped scrollables keep selections anchored to content
`crates/warpui_core/src/elements/clipped_scrollable.rs`:
- `ClippedScrollStateHandle` stores a `selection_scroll_anchor` (original selection + scroll position at the time it was observed).
- `anchor_and_adjust_selection_for_scroll(selection, axis)` either records the anchor (first time) or shifts the selection by the delta between current scroll and the anchored scroll. Doc comment spells out the three branches (None clears anchor; unmatched Selection installs a new anchor; matched Selection returns a scroll-compensated copy).
- `clear_selection_scroll_anchor()` resets that state when a fresh mouse-down starts a new interaction.

`NewScrollable` routes selection APIs through the anchor helper during paint, in `get_selection()`, and in `calculate_clickable_bounds()`. New mouse-down clears the anchor.

### Smart selection respects visible horizontal bounds
`FormattedTextElement::smart_select()` returns `None` when the click point is outside the visible horizontal bounds of the text frame (not just outside vertical bounds). This prevents double-click word selection from targeting text that is off-screen after horizontal scrolling.

### Lazy cache on the `BufferBlockStyle::Table` marker
Both the clipboard copy path and the layout path previously parsed each table block on every invocation. That parse is deterministic from the block's plain text plus `alignments`, so it is lifted onto the marker:

- `crates/editor/src/content/text.rs` introduces `TableBlockCache` (owns parsed `FormattedTable`, `Vec<Vec<TableCellOffsetMap>>`, and `TableOffsetMap`) and a `TableCache` newtype wrapping `Arc<OnceLock<TableBlockCache>>`.
- `BufferBlockStyle::Table` gains a `cache: TableCache` field. A `BufferBlockStyle::table(alignments)` helper constructs the variant with an empty cache.
- `TableCache` implements `PartialEq` / `Eq` / `Hash` as no-ops so `BufferBlockStyle` equality and hashing remain a function of `alignments`. `Clone` stays cheap because the field is a shared `Arc`.
- `TableCache::get_or_populate(text, alignments)` runs the parse once and returns `&TableBlockCache` on subsequent calls. Marker clones share the same `OnceLock`.
- `clipboard_table_text_in_range` and `layout_table_block` both route through the cache. A defensive fallback in `layout_table_block` builds an owned cache on the stack when the style isn't `Table`.

The staleness trade-off is the same one that already applies to `alignments`: if a cell were edited in place without replacing the block marker, the cache could go stale. This matters only when editable tables land; for read-only tables, marker replacement on edit is sufficient.

## Testing and validation

Each numbered invariant in `PRODUCT.md` maps to at least one test or verification step below.

### Automated tests
- PRODUCT invariants 2–3 (overflow rule), 9 (scrollbar interaction): `render::element::table::tests::table_scrollbar_uses_shared_overlay_geometry`, `table_scrollbar_drag_state_survives_renderable_recreation`, `table_scrollbar_pointer_movement_matches_drag_and_gutter_behavior`.
- PRODUCT invariant 4 (container carve-out): exercised via `horizontal_scroll_allowed == false` code paths in model tests; manual pass in a code editor surface configured with `WidthSetting::InfiniteWidth`.
- PRODUCT invariants 7–8 (per-cell max width): `content::edit::tests::test_layout_table_block_clamps_cell_width_to_max`.
- PRODUCT invariant 11 (wheel edge propagation): covered by the `ScrollWheel` handler returning `scroll_horizontally()`'s boolean; manual trackpad pass confirms fall-through.
- PRODUCT invariant 12 (`MouseMoved` not consumed): covered by the handler returning `false`; manual hover-link check confirms downstream handlers still fire.
- PRODUCT invariants 13–15 (selection and copy while scrolled): `content::buffer::tests::test_selected_table_copy_uses_visible_plain_text`, `test_partial_table_selection_does_not_export_html`, `test_partial_table_selection_still_exports_html_for_non_table_ranges`, `test_clipboard_table_copy_uses_source_offsets_for_later_formatted_cells`.
- PRODUCT invariants 13–14 (selection anchor stability): `warpui_core::elements::new_scrollable::scrollable_test` regressions for viewport-coordinate selection APIs, re-anchoring existing selections across horizontal scroll, and clearing the anchor on new mouse-down interactions.
- PRODUCT invariant 13 (double-click word selection across columns): `smart_select_returns_none_when_point_is_outside_horizontal_bounds` (formatted_text_element_tests).
- PRODUCT invariant 18 (caret reveal during keyboard movement): `render::model::location_tests` (hit-testing after horizontal scroll) and `render::model::mod_tests` (reveal/autoscroll for offsets inside tables).
- PRODUCT invariants 6 and 13–15 as applied to formatted cells (inline Markdown correctness): `render::model::table_offset_map::tests::test_table_cell_offset_map_handles_bold_and_links`, `test_table_cell_offset_map_handles_backslash_escaped_punctuation`, `test_table_cell_offset_map_handles_nested_styles`.

### Manual validation
- `crates/markdown_parser/examples/table-demo/all_test_cases.md` opened in a notebook for a visual pass covering PRODUCT invariants 2–3, 6, 7–8, 9–15, 19–21.
- AI block list responses containing a wide Markdown table — trackpad scroll, direct scrollbar interaction, double-click word selection across columns after scroll, partial-range text copy, and an existing selection that remains visually anchored across further horizontal scroll.
- Regression pass confirming vertical scrolling stays owned by the surrounding notebook or block list (PRODUCT invariant 5).
- Regression pass confirming narrow tables and non-table Markdown remain unchanged (PRODUCT invariant 22).

### Pre-merge gates
- `cargo fmt`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
- `cargo check -p warp_editor` and `cargo check -p warpui_core`
- Targeted `cargo nextest run --no-fail-fast --workspace ...` for the regressions listed above.

## Risks and mitigations

### Two offset spaces drift again
If any table path assumes source and rendered offsets are interchangeable, formatted cells will reintroduce selection/copy drift. Mitigation: keep `TableCellOffsetMap` as the single translation layer; build the table-wide `TableOffsetMap` from source lengths only; route hit-testing, cursor, selection, and copy through explicit source↔rendered conversions.

### Selection pinned to viewport coordinates
If clipped scrollables reuse raw selection rectangles after scroll changes, highlights and copied text target the wrong content. Mitigation: anchor selections via `ClippedScrollStateHandle`, clear the anchor on fresh mouse interactions, and route selection APIs through `anchor_and_adjust_selection_for_scroll()`.

### Scrollbar behavior diverges between surfaces
If the editor table path reintroduces bespoke thumb sizing or drag math, it will drift from the rest of WarpUI. Mitigation: keep geometry and pointer/scroll conversion in `shared_scrollbar.rs` and call the shared helpers from both the editor table renderer and the `new_scrollable` utilities.

### Clipped child layers interfere with editor hit-testing
If editor bounds checks use the clipped child layer instead of the editor element bounds, clicks can be rejected before block-level hit-testing runs. Mitigation: use the editor element's own layer bounds in `crates/editor/src/render/element/mod.rs`.

### Stale cache on in-place cell edits
`TableCache` is keyed by marker instance. If a future change edits a cell in place without replacing the block marker, the cache will be stale. Mitigation (current): marker replacement on edit is sufficient for read-only tables. Mitigation (longer-term): the SumTree follow-up below invalidates per-cell state naturally.

## Follow-ups
- If additional inline Markdown styles are supported in table cells, add a focused `TableCellOffsetMap::from_inline_and_source` regression alongside the parser change — the source-walking algorithm should handle them automatically, but a test pins the behavior.
- Other clipped horizontal surfaces adopting the same selection APIs inherit the `ClippedScrollStateHandle` behavior automatically, but they should get a manual regression pass when adopting new selection UX.
- When editable tables or other complex table operations land, evaluate moving cell/row boundaries into the `SumTree` with new `BufferText` marker types so per-cell offsets can be derived by seeking instead of reparsing. See the in-code `TODO` above `impl TableCellOffsetMap`.
