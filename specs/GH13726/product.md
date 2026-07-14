# PRODUCT.md — Markdown viewer: raw-HTML `<table>` support

Issue: https://github.com/warpdotdev/warp/issues/13726
Split out of #13652 (bulk raw-HTML-subset request, closed in favor of per-feature
issues). This spec previously targeted #13652 (as `specs/GH13652/tables/`) and has
been retargeted to the focused #13726 without changing its scope.
Related: `<br>`-in-cell now has its own dedicated issue, #13732
(`specs/GH13732/` if/when authored) — this spec still implements `<br>`-in-cell
directly (invariant 4) since it's part of what makes HTML tables worth having over
GFM pipe tables, but #13732 is the place to look for any standalone `<br>` handling
outside of tables (e.g. `<br>` in ordinary paragraph text).

## Summary

Warp's Markdown viewer already renders GFM pipe-tables (behind the `MarkdownTables`
feature flag), but drops raw-HTML `<table>` markup. The issue asks for HTML tables
specifically for the two things GFM pipe-table syntax **cannot** express: cells that
need a hard line break (`<br>`), and cells that span multiple columns or rows
(`colspan`/`rowspan`).

This spec covers teaching the Markdown viewer to render a block-level HTML
`<table>` (`<thead>`/`<tbody>`/`<tr>`/`<th>`/`<td>`) by mapping it onto Warp's existing
table model and render path, and to honor `<br>` inside cells. It scopes
`colspan`/`rowspan` as an **explicit non-goal for this slice**, because those fight the
existing flat rectangular-grid model at every layer and warrant their own effort. It
covers raw HTML table markup only — GFM pipe-syntax tables (`| a | b |`) already work
today, gated behind `FeatureFlag::MarkdownTables`, and are out of scope here.

A deliberate framing note for reviewers: a *simple* HTML table (no `<br>`, no spans) maps
onto the existing `FormattedTable` and renders with zero editor changes — but it delivers
little beyond GFM pipe tables. The value HTML tables add is `<br>`-in-cell and spans;
this slice targets the first (`<br>`) and honestly defers the second (spans).

The repo's own paste-path test, `test_unsupported_html_types`
(`crates/markdown_parser/src/html_parser_tests.rs:191-222`, TODO-marked), documents
today's non-support and should be updated as part of this work.

Figma: none provided.

## Goals / Non-goals

In scope:

- Recognize a block-level HTML `<table>` in a Markdown document (on its own lines) and
  render it through Warp's existing table layout/render path.
- Support `<thead>`/`<tbody>` grouping, `<tr>` rows, `<th>` header cells, and `<td>` data
  cells. A `<th>` row (or a `<thead>` row) becomes the table header; `<td>` rows become
  data rows.
- Parse inline content inside cells (bold, italic, code, links, and inline images per the
  `<img>` spec) using the viewer's existing inline parsing.
- Honor `<br>` inside a cell as a hard line break, producing a genuinely multi-line cell
  (the primary capability GFM pipe tables lack).
- Honor per-column alignment when expressed via `align="left|center|right"` on `<th>`/
  `<td>` (or the equivalent `style="text-align:…"`), defaulting to left.
- Degrade gracefully: a malformed/unterminated `<table>`, or a ragged table with
  inconsistent cell counts, renders without panicking — either as a best-effort normalized
  grid (padded to a uniform shape, as the existing model already does via
  `normalize_shape`) or, if unparseable, as literal text.

Out of scope (explicit non-goals):

- **`colspan` / `rowspan`.** Spanning cells require a non-rectangular grid model that the
  current data model, layout, render, and selection code do not support. A `<td colspan>`
  or `<td rowspan>` is handled as a **degraded** single-cell (the span attribute is
  ignored and the cell occupies one slot), never a panic or corrupt layout. Full span
  support is a documented follow-up.
- `<caption>`, `<colgroup>`/`<col>`, and nested tables inside a cell.
- Inline `<table>` mixed with other text on the same line.
- Arbitrary CSS beyond the discrete `align` / `text-align` used for alignment.
- Any change to GFM pipe-table behavior. HTML tables are an additional input path that
  produces the same internal table representation.
- Script execution / event handlers / navigation from table markup.

## Behavior

1. A Markdown document region delimited by `<table>` … `</table>` on their own lines
   renders as a table in the viewer, using Warp's existing table appearance (borders,
   dividers, alignment, horizontal scrolling for wide tables), identical to how a GFM
   pipe-table of the same content renders.

2. A row of `<th>` cells — whether inside `<thead>` or as the first `<tr>` — becomes the
   table's header row. Subsequent `<tr>` rows of `<td>` cells become data rows. A table
   with no `<th>`/`<thead>` renders its first `<tr>` as the header (matching the existing
   model, which always has a header row), unless the design chooses to synthesize an empty
   header; the tech spec picks one and documents it.

3. Cell content renders with the viewer's inline formatting: bold, italic, inline code,
   strikethrough, links, and inline images (per the `<img>` spec) all render inside a cell
   exactly as they do in a GFM table cell.

4. A `<br>` inside a cell renders as a hard line break, so the cell's content occupies
   multiple lines within its row. The row's height grows to fit the tallest cell. This is
   distinct from the automatic word-wrapping the viewer already does for long cell content
   — `<br>` is an author-specified break that is honored regardless of column width.

5. Column alignment follows `align`/`text-align` on the header (or per-cell) where present,
   defaulting to left. Alignment renders identically to GFM table alignment.

6. A ragged HTML table (rows with differing cell counts) is normalized to a uniform column
   count (short rows padded with empty cells), matching the existing table model's
   `normalize_shape` behavior. It must not panic or misalign columns.

7. A `<td colspan="N">` or `<td rowspan="N">` is rendered as a single ordinary cell (the
   span attribute is ignored for this slice). The table remains a well-formed rectangular
   grid; no cell visually merges. This is a known limitation (non-goal), surfaced so the
   behavior is predictable rather than a crash or corrupt grid.

8. A malformed or unterminated `<table>` (missing `</table>`, or content the parser can't
   form into a grid) falls back deterministically to the same plain-text sink ordinary
   Markdown text renders through (`FormattedTextLine::Line`, rendered via
   `FormattedTextElement`) rather than panicking, silently dropping content, or swallowing
   the rest of the document. The unparseable region renders as literal (escaped) source
   text, not undefined behavior.

9. Empty structures degrade cleanly: `<table></table>` renders as an empty (or
   single-empty-row) table or nothing, per the tech spec's chosen convention, never a
   panic; a `<tr></tr>` with no cells is dropped or padded, not a crash.

10. Only structural tags (`table`/`thead`/`tbody`/`tr`/`th`/`td`) and the `align`/
    `text-align`/`colspan`/`rowspan` attributes are read (the last two only to decide the
    degraded single-cell behavior of invariant 7). All other attributes (`onclick`,
    `style` beyond text-align, `class`, `id`, …) are ignored. No attribute is executed or
    navigated to.

11. Copy / export of a document containing an HTML table preserves the tabular content.
    Because the internal representation is the shared table model, export may canonicalize
    to GFM pipe-table syntax where the content fits it; content that GFM cannot express
    (a `<br>`-bearing multi-line cell) must round-trip in a form that preserves the line
    break (the tech spec defines the serialization). Span attributes, being ignored, are
    not preserved.
