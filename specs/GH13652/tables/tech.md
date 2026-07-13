# TECH.md — Markdown viewer: raw-HTML `<table>` support

Product spec: `specs/GH13652/tables/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13652
Preceding specs in the chain: `specs/GH13652/` (`<img>`, PR #13656),
`specs/GH13652/details-summary/`.

## Context

Warp **already has a table model and render path**; the work here is a new *input path*
(HTML `<table>`) into it, plus one model extension (`<br>`-in-cell).

The shared model — `crates/markdown_parser/src/lib.rs:354-359`:

```rust
pub struct FormattedTable {
    pub headers: Vec<FormattedTextInline>,
    pub alignments: Vec<TableAlignment>,   // Left / Center / Right (:344-351)
    pub rows: Vec<Vec<FormattedTextInline>>,
}
```

A flat, rectangular headers+rows grid. A cell is `FormattedTextInline = Vec<FormattedTextFragment>`
(`:501`) — a run of styled inline fragments, **structurally single-line**: `parse_table_cell`
rejects `\n`/`\r` (`markdown_parser.rs:596-597`), and no code path produces a line break
inside a cell. There is **no colspan/rowspan/caption** field anywhere (`grep colspan|rowspan`
across `crates/editor/` is empty).

GFM tables enter via `parse_table` (`markdown_parser.rs:535-674`), gated by
`FeatureFlag::MarkdownTables` in `crates/editor/src/content/buffer.rs:850-855`
(`parse_markdown_with_gfm_tables` vs `parse_markdown`). There is also a fenced-code
internal round-trip format `warp-markdown-table` (`markdown_parser.rs:39`,
`FormattedTable::from_internal_format`/`to_internal_format`, `lib.rs:363-411`) that is
**strictly tab-separated flat cells, newline-separated rows** — it cannot represent an
intra-cell newline (a `\n` reads as a new row) and has no slot for spans.

Render path (all already built, reused as-is for the simple case):

- Layout: `layout_table_block` / `measure_table_cells` (`crates/editor/src/content/edit.rs:1197-1339`,
  `:260-323`), column widths clamped `[MIN_TABLE_CELL_CONTENT_WIDTH_EMS=1.0 (:132),
  MAX_TABLE_CELL_CONTENT_WIDTH_PX=500.0 (:133)]`; cells already **wrap** to multiple visual
  lines when content exceeds the clamp (pass-2 re-layout at fixed width, `:1250-1289`).
- Render model: `TableBlockConfig` (`render/model/mod.rs:1478-1482`), `LaidOutTable`
  (`:1623-1644`), and crucially `CellLayout { line_heights, line_y_offsets, line_char_ranges,
  line_widths, line_caret_positions }` (`:1487-1493`) — **multi-line cells are already a
  first-class rendering concept**, sourced from wrapped text frames (`from_text_frame`,
  `:1496-1533`).
- Element: `crates/editor/src/render/element/table.rs` (`RenderableTable`) paints a uniform
  grid + per-table horizontal scroll.
- Selection/copy correctness across the horizontal viewport relies on per-cell source↔
  rendered offset maps (`table_offset_map.rs`, `content/text.rs` `table_cell_offset_maps`),
  introduced by `specs/zachlloyd/wide-markdown-table-scrolling/`.

The HTML paste parser (`crates/markdown_parser/src/html_parser.rs`) currently **skips**
`<table>` (`TOP_LEVEL_ELEMENT_TAGS_TO_SKIP` at `:23-25`) and flattens `<tr>/<td>/<th>` into
paragraph lines (they fall to the `_ =>` arm at `:337-347`) — so it produces **no**
`FormattedTable` from HTML today. But it already has the reusable pieces: an `html5ever`
DOM (`parse_document`, `:126`), attribute helpers (`get_attribute` `:107`), and inline
phrasing parsing (`parse_phrasing_content` `:410`). `html5ever` is already a dependency of
`crates/markdown_parser`.

### Constraints from existing table specs (must respect)

- `specs/zachlloyd/markdown-table-consistency/` — table chrome must come from the shared
  `TableStyle`/appearance helper; "keep layout, selection, cursor, and alignment code
  unchanged." A new input path must produce a normal `FormattedTable` and not fork the
  render chrome.
- `specs/zachlloyd/wide-markdown-table-scrolling/` — per-cell source↔rendered offset maps
  and the `MAX_TABLE_CELL_CONTENT_WIDTH_PX` wrap clamp must stay correct. Any change to
  cell content structure (the `<br>` line-break work) must keep these offset maps correct.

## Feasibility summary (three slices, honestly sized)

- **(i) Simple HTML `<table>` → `FormattedTable`: SMALL.** A simple HTML table *is* a
  rectangular headers+rows grid of inline cells — exactly `FormattedTable`. Build one from
  the DOM and emit `FormattedTextLine::Table`; the entire existing layout/render/scroll/
  selection path works unchanged. But this alone barely beats GFM pipe tables.
- **(ii) `<br>` in cells: MEDIUM.** The renderer already does multi-line cells (via wrap),
  but the cell *type* can't represent an authored hard break, and the internal round-trip
  format uses `\n` as the row delimiter. Needs a cell-type/serialization change, not new
  rendering. This is the capability that makes HTML tables worth having.
- **(iii) `colspan`/`rowspan`: LARGE — explicit non-goal here.** The flat rectangular grid
  (one width per column, one height per row, `[row][col]` indexing) is baked into
  `FormattedTable`, `measure_table_cells`, `LaidOutTable`, the painter, and every offset
  map. Spans need a non-rectangular grid model across all of them. Deferred to a follow-up.

This spec implements **(i) + (ii)** and degrades **(iii)** to single-cell (product
invariant 7).

## Proposed changes

### 1. Cell model: allow an authored line break (`<br>`)

The minimal, least-disruptive change is to make a cell a sequence of lines rather than a
single inline run. Two options:

- **Option A (recommended): change the cell type to `Vec<FormattedTextInline>`** (a list of
  lines) in `FormattedTable.headers`/`rows`. This is explicit and makes multi-line cells
  first-class end to end. It ripples through `from/to_internal_format`, `normalize_shape`,
  `to_plain_text`, and every editor consumer of `FormattedTable` cells.
- **Option B (smaller, hackier): keep `FormattedTextInline` but introduce a line-break
  sentinel fragment** (e.g. a `FormattedTextFragment` flagged as a hard break) that the
  layout inserts as a forced newline. Less type churn, but every consumer must know to
  treat the sentinel specially, and it's easy to miss a site.

Recommend Option A for correctness, but call the choice out for maintainer review since it
touches the shared `FormattedTable` type. Whichever is chosen:

- Layout (`measure_table_cells` / pass-2 in `edit.rs:260-323,1250-1289`) inserts the
  authored break so the cell lays out to ≥2 lines; the existing multi-line `CellLayout`
  machinery (`render/model/mod.rs:1487-1533`) then handles heights/selection.
- Serialization: `to_internal_format` (`lib.rs:396-411`) must escape an intra-cell break
  (it can't be a literal `\n`); encode it as `<br>` (or an escape marker) within the
  tab-separated cell, and decode symmetrically in `from_internal_format`. `to_plain_text`
  (GFM export) likewise encodes the break as `<br>` since GFM has no intra-cell newline.
- Keep the per-cell offset maps (`table_cell_offset_maps`) correct across the added break.

### 2. HTML table reader in `markdown_parser`

Add a `<table>` reader that produces a `FormattedTable`. Two placement options:

- Extend `html_parser.rs`: remove `"table"` from `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP` (`:23-25`)
  and add explicit handling that walks `<thead>/<tbody>/<tr>/<th>/<td>`, builds rows of
  cells via the existing `parse_phrasing_content` (`:410`) for inline content, reads
  `align`/`text-align` for `TableAlignment`, maps `<br>` (already `→ LineBreak` at `:335`)
  into the new intra-cell break, and assembles a `FormattedTable`. Reuses the DOM + helpers
  already there.
- Because `html_parser.rs` is currently paste-oriented (whole-document), the file-viewer
  path needs the block Markdown grammar to *detect* a raw `<table>` block and route its text
  through this reader. Add a block-level detector in `markdown_parser.rs` (near the image
  block branch) that recognizes an own-line `<table>` … `</table>` region, extracts the raw
  HTML, and calls the table reader — emitting `FormattedTextLine::Table(FormattedTable)`.
  This mirrors how the `<img>` and `<details>` specs add own-line raw-HTML block detectors.

Header determination (product invariant 2): a `<thead>` row, or a `<tr>` whose cells are
`<th>`, becomes `headers`; remaining `<tr>` become `rows`. If there is no header row,
promote the first `<tr>` to the header (the model always has a header) — documented
behavior.

`colspan`/`rowspan` (product invariant 7): read the attributes only to **ignore** them —
each `<td>`/`<th>` occupies exactly one grid slot regardless of span. Ragged rows are fixed
by the existing `FormattedTable::normalize_shape` (`lib.rs:414-429`) (invariant 6).

Fallback (invariant 8): if the region has no `</table>` or can't form a grid, the block
detector fails and the text is parsed as ordinary Markdown/literal text.

### 3. Feature gating

HTML tables should ride the **existing `FeatureFlag::MarkdownTables`** gate
(`buffer.rs:850-855`) so they light up exactly where GFM tables do, and stay dark where
tables are disabled. No new flag. The `<br>`-in-cell model change (item 1) is behind the
same table code paths, so it only affects tables.

### 4. Security

Only structural tags and `align`/`text-align` (plus `colspan`/`rowspan` read solely to
ignore) are consulted; all other attributes are dropped (invariant 10). Cell content is
parsed as inline Markdown/phrasing content and inherits the viewer's existing trust
boundary. Inline images inside cells resolve through the same asset-source resolver as the
`<img>` spec — no new source path. No script/event-handler surface.

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/html_parser_tests.rs`, `markdown_parser_tests.rs`)

- Simple `<table>` with `<thead>`/`<tbody>` → `FormattedTable` with expected headers/rows
  (invariants 1, 2).
- `<th>`-first-row table with no `<thead>` → first row is header (invariant 2).
- Inline formatting inside cells (bold/link/`code`/inline image) → parsed fragments
  (invariant 3).
- `<br>` in a cell → multi-line cell (invariant 4); assert the cell holds ≥2 lines under
  the chosen cell model.
- `align`/`text-align` on cells → `TableAlignment` (invariant 5).
- Ragged rows → normalized to uniform columns (invariant 6).
- `<td colspan="2">` / `rowspan` → single ordinary cell, span ignored, grid rectangular
  (invariant 7).
- Unterminated `<table>` / non-grid content → literal-text fallback, document below intact
  (invariant 8).
- `<table></table>` / empty `<tr>` → no panic (invariant 9).
- Ignored attributes (`onclick`, `class`) → not consulted (invariant 10).

### Round-trip (`crates/markdown_parser` + `crates/editor/src/content/text_tests.rs`)

- HTML table without `<br>` → internal format → back, content preserved; canonicalizes to
  GFM where it fits (invariant 11).
- HTML table **with** `<br>` in a cell → round-trips preserving the line break (encoded as
  `<br>` in the internal/GFM forms), not collapsed and not turned into a new row
  (invariant 11 + 4).

### Layout / render tests (`crates/editor/src/render/model/mod_tests.rs`)

- A `<br>`-bearing cell increases its row height to fit the extra line; neighbor columns
  unaffected.
- Column widths still honor the `[MIN, MAX]` clamp; wide HTML tables still scroll
  horizontally (no regression to the wide-table viewport).
- Selection/copy across a multi-line cell stays correct (offset maps).

### Integration / manual

Per CONTRIBUTING, before/after screenshots + a short recording rendering the issue's
motivating case — an HTML table whose cell contains a `<br>`-separated multi-line value —
alongside a GFM table for comparison, and a `colspan` table showing the documented
degraded (span-ignored) rendering. Add `crates/integration/` coverage for opening a
Markdown file containing an HTML table if exercisable there.

## Risks and follow-ups

- **The valuable capability is the medium-cost one.** A simple HTML table (small) barely
  improves on GFM pipe tables; the reason to do HTML tables at all is `<br>`-in-cell (and,
  later, spans). This slice therefore commits to the `<br>` cell-model change rather than
  shipping only the near-free simple-table reader. If maintainers would rather ship the
  simple reader first and defer `<br>`, the cell-model change (item 1) can be split into its
  own follow-up — noted as an option.
- **Cell-type change touches the shared `FormattedTable`.** Whether Option A (`Vec<lines>`)
  or Option B (sentinel fragment), this ripples through parser round-trip, editor layout,
  and offset maps. It's the main risk surface; the tests above target each site. If it
  starts to sprawl, that's the signal to split simple-table and `<br>` into two PRs.
- **`colspan`/`rowspan` is a genuine model change** (non-rectangular grid) and is an
  explicit non-goal here (invariant 7 degrades it). It deserves its own spec/PR — likely
  the largest single piece of the whole #13652 effort — and should be scoped separately
  once simple + `<br>` tables land.
- **Interaction with the other tier-zero specs:** inline images inside cells depend on the
  `<img>` spec's inline-image support; an HTML table inside a `<details>` body should work
  under that spec's Option-A model since the table is an ordinary top-level block. Verify
  once the chain lands.
