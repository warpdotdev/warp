# TECH.md — Markdown viewer: raw-HTML `<table>` support

Product spec: `specs/GH13726/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13726
Split out of #13652 (bulk raw-HTML-subset request, closed in favor of per-feature
issues). Preceding specs from that chain: `specs/GH13652/` (`<img>`, PR #13656),
`specs/GH13652/details-summary/`. Related: `<br>`-handling has its own issue, #13732;
this spec still owns `<br>`-in-cell (item 1 below) since it's core to the table's
value proposition, but see #13732 for `<br>` outside of tables.

This spec covers **raw HTML table markup only**. GFM pipe-syntax tables
(`| a | b |`) already work today via `parse_table`, gated behind
`FeatureFlag::MarkdownTables` (`buffer.rs:850`) — unaffected by this work.

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
  `align`/`text-align` for `TableAlignment`, and assembles a `FormattedTable`.
  **Correction to an earlier draft of this spec:** `<br>` is *not* already handled inside
  cell content. `"br" => FormattedTextLine::LineBreak` at `:335` is a top-level block-match
  arm — it never fires for a `<br>` nested inside `parse_phrasing_content` (`:410-457`),
  whose own `NodeData::Element` match (`:439-446`) has no `"br"` case today: an unhandled
  element falls to the generic styling no-op and recurses into its (empty, for a void
  element) children, so a `<br>` inside phrasing content is currently **silently dropped**,
  not converted to a break. This spec must add a `"br"` arm to `parse_phrasing_content`
  itself (pushing the new intra-cell break construct from item 1, not
  `FormattedTextLine::LineBreak` — a cell can't hold a `FormattedTextLine`, per item 1's
  model change) — this is new code, not a reuse of `:335`, and reuses the DOM + helpers
  already there.

  **Escape ambiguity (author intent vs. literal text) — resolved by construction, and
  the reason must be stated because it does not generalize from `specs/GH13732/tech.md`.**
  A cell author must be able to write a literal `<br>` as visible text (e.g. documenting
  HTML syntax inside a table) without it becoming a hard break, by escaping it as
  `&lt;br&gt;`. Because this reader operates on an `html5ever`-parsed DOM (not the raw
  Markdown inline tokenizer `specs/GH13732/tech.md` modifies for GFM cells), entity
  decoding and tag recognition are **not two competing parsers over the same character
  span** — they happen at different DOM-construction stages that cannot collide:

  - **Parse direction.** `html5ever`'s tokenizer decodes character references (`&lt;`,
    `&amp;`, `&#60;`, …) while building `NodeData::Text` nodes, and recognizes `<br>` as
    a `NodeData::Element` with `name == "br"` as a structurally separate tokenization
    decision — an element boundary, not a character sequence inside a text node's
    content. A source cell containing `&lt;br&gt;` therefore *always* produces a single
    `NodeData::Text` node whose decoded content is the four characters `<br>`; that text
    can never be reclassified as a `NodeData::Element("br")` later, because element
    recognition already happened (or didn't) during tokenization, before decoded text
    content exists to be reinterpreted. A source cell containing a literal, unescaped
    `<br>` produces a `NodeData::Element("br")` node instead — the fork happens once, at
    tokenization, never at any later pass this reader runs. Concretely: authored break —
    `<td>a<br>b</td>` → DOM: text "a", element `br`, text "b" → cell holds 2 lines.
    Escaped literal — `<td>a&lt;br&gt;b</td>` → DOM: one text node "a\<br\>b" (already
    decoded) → cell holds 1 line containing the literal string `<br>`. There is no
    parser-ordering decision for this spec to make (unlike GH13732's raw-tokenizer case,
    where `<br>`-token recognition and entity decoding are both stages of the *same*
    linear scan and their relative order is a real design choice) — this is a
    consequence of using `parse_phrasing_content`/DOM parsing for table cells rather
    than the GFM inline tokenizer, and should be called out explicitly so a reader
    doesn't assume the two specs share a hazard they don't.
  - **Serialize direction (`to_internal_format` / GFM `to_plain_text` export, per item
    1 below).** An authored break must serialize as literal `<br>` text (per
    `specs/GH13732/tech.md`'s recommendation, reused here for consistency between the
    two specs' cell-break encoding). A cell whose content is the *literal string* `<br>`
    (from an escaped source, not a break) must serialize as `&lt;br&gt;`, i.e.
    serialization must escape any literal `<`/`>` characters in cell text through the
    existing HTML-entity-escaping path (the same one `specs/GH13721/tech.md` §5 uses for
    attribute-value escaping) *before* emitting the break-token `<br>` for actual breaks
    — the break token is inserted as a raw, unescaped `<br>` at the break's position,
    never itself subject to the cell-text escaping pass, so the two are never
    indistinguishable in the serialized form either. A **double-escaped** source cell
    (`&amp;lt;br&amp;gt;`) decodes at parse time to the literal text `&lt;br&gt;`
    (`&amp;` → `&`, and the resulting `&lt;br&gt;` is *not* re-decoded — HTML entity
    decoding is single-pass, per `html5ever`, matching this repo's own single-pass
    `parse_html_entity`, `markdown_parser.rs:1903-1959`), which then re-escapes on
    serialization to `&amp;lt;br&amp;gt;`, round-tripping exactly.
- Because `html_parser.rs` is currently paste-oriented (whole-document), the file-viewer
  path needs the block Markdown grammar to *detect* a raw `<table>` block and route its text
  through this reader. Add a block-level detector in `markdown_parser.rs` (near the image
  block branch) that recognizes an own-line `<table ...>` … `</table>` region, extracts the
  raw HTML, and calls the table reader — emitting `FormattedTextLine::Table(FormattedTable)`.
  This mirrors how the `<img>` and `<details>` specs add own-line raw-HTML block detectors.
  **The detector's opening-tag match must tolerate attributes and whitespace, not just the
  bare `<table>` literal** — a table opened as `<table class="data" id="results">` is a
  block-level `<table>` for every purpose this spec cares about (product invariant 10 already
  says `class`/`id`/etc. are read-and-ignored, which presupposes the tag was recognized in
  the first place). Concretely, the detector's opening-tag grammar is: `<table`
  (case-insensitive), then a sequence of `name="value"`/`name='value'`/bare attributes
  tolerant of arbitrary whitespace, terminated by `>` (a self-closing `/>` on `<table>` is not
  meaningful HTML and is treated as a non-match, falling to invariant 8's literal-text
  fallback same as any other malformed tag) — the same attribute-scanning shape
  `specs/GH13721/tech.md` §2 uses for its `parse_html_image` attribute loop (hand-written,
  quoted/unquoted values, ends at the first unquoted `>`, unrecognized attributes consumed
  and discarded). A bare `<table>` with no attributes is simply the zero-attribute case of
  this same grammar, not a separate path. The closing `</table>` match is unaffected (closing
  tags carry no attributes in valid HTML; a `</table foo>`-shaped malformed closer is not a
  match and falls to invariant 8 like any other unparseable region).

Header determination (product invariant 2) — `FormattedTable` has exactly one `headers`
slot, so the reader must pick exactly one source row for it, by precedence:

1. `<thead>`'s first `<tr>` → `headers`. Any further rows inside `<thead>` are appended to
   `rows` in document order (as plain data rows — no header styling carried over), not
   dropped.
2. Else, if the table's first `<tr>` is `<th>`-majority → that row becomes `headers`. Any
   *other* `<tr>` in the table (outside `<thead>`) that is also `<th>`-majority is appended
   to `rows` as a plain data row, same as case 1 — never dropped, never merged into
   `headers`.
3. Else, the table's first `<tr>`, of whatever cell tag composition, becomes `headers`
   (the model always has a header row).

Implementation-wise: the reader walks rows in document order, classifies each as
`<thead>`-first / `<th>`-majority / other using the rules above to find the *one* row that
wins `headers`, and every other row — regardless of its own `<th>`/`<td>` tag mix — is
pushed onto `rows` using the ordinary cell reader (tag is not consulted again once a row is
routed to `rows`; a demoted `<th>` row's cells parse the same as `<td>` cells). No cell
data is discarded by this step; only header *styling* is not preserved for demoted rows,
since `FormattedTable` has no per-row header flag beyond the single `headers` slot.

Alignment resolution (product invariant 5) — `alignments: Vec<TableAlignment>` is one slot
per column, so per-cell `align`/`text-align` must collapse to a single value per column
before construction:

- For each column index, scan `align`/`text-align` in this order: the resolved header
  cell's value (if any) wins outright. If the header specifies none, take the first body
  cell in that column (in row order) that specifies one.
- If no cell in the column specifies an alignment, default to `TableAlignment::Left`
  (matching the type's `#[default]`, `lib.rs:347`).
- This resolution happens once, after the header/rows split above, so "the header cell" is
  well-defined even when case 2/3 promotes a non-`<thead>` row to header.
- Disagreeing cells are not tracked or surfaced anywhere post-resolution — the model has no
  per-cell alignment override, matching how GFM tables already work (the separator row sets
  one alignment per column with no per-cell escape hatch either).

`colspan`/`rowspan` (product invariant 7): read the attributes only to **ignore** them —
each `<td>`/`<th>` occupies exactly one grid slot regardless of span. Ragged rows are fixed
by the existing `FormattedTable::normalize_shape` (`lib.rs:414-429`) (invariant 6).

Fallback (invariant 8): if the region has no `</table>` or can't form a grid, the block
detector fails deterministically and the raw region is handed back to ordinary Markdown
parsing, landing as `FormattedTextLine::Line` — the same plain-text sink (rendered via
`FormattedTextElement`) that any other unrecognized text renders through. Never an
unspecified/undefined outcome.

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
- `<thead>` with **two** rows → first row is `headers`, second row appended to `rows` as a
  plain data row, not dropped, not merged (invariant 2).
- No `<thead>`, first `<tr>` is `<th>`-majority, and a **later** `<tr>` elsewhere in the
  table is also `<th>`-majority → first row is `headers`, the later `<th>`-majority row is
  demoted to a plain data row (invariant 2).
- Inline formatting inside cells (bold/link/`code`/inline image) → parsed fragments
  (invariant 3).
- `<br>` in a cell → multi-line cell (invariant 4); assert the cell holds ≥2 lines under
  the chosen cell model.
- **Escape-ambiguity matrix (invariant 4's escape rule):**
  - Authored break: `<td>a<br>b</td>` → cell holds 2 lines, `["a", "b"]`.
  - Escaped literal: `<td>a&lt;br&gt;b</td>` → cell holds 1 line, text `a<br>b` (the
    literal 4 characters `<br>` visible, not a break).
  - Mixed cell: `<td>line1<br>literal: &lt;br&gt;<br>line3</td>` → cell holds 3 lines,
    the middle line containing the literal text `literal: <br>`, unambiguous from the
    two real breaks around it.
  - Double-escape: `<td>&amp;lt;br&amp;gt;</td>` → cell holds 1 line, text `&lt;br&gt;`
    (single-pass entity decode: `&amp;` → `&`, the resulting `&lt;br&gt;` is not
    re-decoded), never a break, never the raw text `<br>`.
- `align`/`text-align` on cells → `TableAlignment` (invariant 5).
- Header cell and a body cell in the same column disagree on alignment → header wins
  (invariant 5).
- Header cell specifies no alignment, two body cells in the same column disagree → first
  body cell (in row order) that specifies one wins (invariant 5).
- No cell in a column specifies alignment → defaults to left (invariant 5).
- Ragged rows → normalized to uniform columns (invariant 6).
- `<td colspan="2">` / `rowspan` → single ordinary cell, span ignored, grid rectangular
  (invariant 7).
- Unterminated `<table>` / non-grid content → literal-text fallback, document below intact
  (invariant 8).
- `<table></table>` / empty `<tr>` → no panic (invariant 9).
- Ignored attributes (`onclick`, `class`) → not consulted (invariant 10).
- `<table class="data" id="results">` (attributed opening tag, whitespace-tolerant,
  single/double-quoted values) → still **recognized** by the block detector and produces
  a `FormattedTable` identical to the bare-`<table>` case, not literal-text fallback
  (invariant 1, invariant 10 — detector-grammar/attribute-allowlist alignment).
- `<table/>` (self-closing, meaningless for a container element) → not a detector match,
  falls to literal-text fallback (invariant 8), same as any other malformed tag.

### Round-trip (`crates/markdown_parser` + `crates/editor/src/content/text_tests.rs`)

- HTML table without `<br>` → internal format → back, content preserved; canonicalizes to
  GFM where it fits (invariant 11).
- HTML table **with** `<br>` in a cell → round-trips preserving the line break (encoded as
  `<br>` in the internal/GFM forms), not collapsed and not turned into a new row
  (invariant 11 + 4).
- HTML table with a cell containing the **literal text** `<br>` (from an escaped
  `&lt;br&gt;` source) → round-trips to a cell whose serialized form re-escapes it as
  `&lt;br&gt;`, not a raw `<br>` that would misparse as a break on the next read
  (invariant 11 + 4's escape rule).
- Double-escaped source (`&amp;lt;br&amp;gt;`) → round-trips to the same double-escaped
  serialized form, confirming single-pass decode/re-escape symmetry.

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
- **Issue-linkage: resolved, tracked for follow-through.** #13726 cited `colspan`/`rowspan`
  as a motivating requirement this PR does not deliver; per product.md's "Issue linkage
  (resolved)" note and the Non-goals acceptance criteria, the resolution is option (b) —
  `colspan`/`rowspan` is split to its own follow-up issue, #13953, and this PR keeps
  "Closes #13726" for the un-spanned subset. What remains before merge is mechanical, not
  a decision: issue #13726's own body must be narrowed to match (not left describing spans
  as in-scope), per product.md's Non-goals acceptance criteria.
- **Interaction with the other tier-zero specs:** inline images inside cells depend on the
  `<img>` spec's inline-image support; an HTML table inside a `<details>` body should work
  under that spec's Option-A model since the table is an ordinary top-level block. Verify
  once the chain lands.
