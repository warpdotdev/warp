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
little beyond GFM pipe tables. The value HTML tables add is `<br>`-in-cell and spans.
**This slice delivers the un-spanned raw-HTML table subset — simple tables plus
`<br>`-in-cell — and spans remain a documented follow-up, not a partial implementation
of them.** Concretely: a `<td colspan="N">`/`<td rowspan="N">` in the input renders
today as a single ordinary cell, deterministically (invariant 7) — never a crash, never
a silently-merged cell, never left unspecified. Because #13726 cites `colspan`/`rowspan`
as one of its two motivating requirements, this PR does **not** fully close that issue on
its own; see the note on issue linkage below.

**Issue linkage (resolved):** option (b) was chosen — `colspan`/`rowspan` support is
tracked as its own follow-up issue, #13953, and this PR keeps "Closes #13726" for the
un-spanned raw-HTML table subset (the deliverable this issue's title describes). Spanned
tables degrade deterministically per the malformed-input rules below until #13953 lands.
This linkage is not just a PR-comment decision — it is binding on this spec and on issue
#13726 itself: see the Non-goals section's acceptance criteria for what must be true
before "Closes #13726" is valid, and #13726's own body must be narrowed to match (not
left describing spans as in-scope) as part of landing this PR.

The repo's own paste-path test, `test_unsupported_html_types`
(`crates/markdown_parser/src/html_parser_tests.rs:191-222`, TODO-marked), documents
today's non-support and should be updated as part of this work.

Figma: none provided.

## Goals / Non-goals

In scope:

- Recognize a block-level HTML `<table>` in a Markdown document (on its own lines) and
  render it through Warp's existing table layout/render path.
- Support `<thead>`/`<tbody>` grouping, `<tr>` rows, `<th>` header cells, and `<td>` data
  cells. Exactly one row becomes the table's header, matching the model's single-header-row
  shape; see invariant 2 for the deterministic rule when a table has more header-like rows
  than that (multiple `<thead>` rows, or stray `<th>` rows outside `<thead>`).
- Parse inline content inside cells (bold, italic, code, links, and inline images per the
  `<img>` spec) using the viewer's existing inline parsing.
- Honor `<br>` inside a cell as a hard line break, producing a genuinely multi-line cell
  (the primary capability GFM pipe tables lack).
- Honor per-column alignment when expressed via `align="left|center|right"` on `<th>`/
  `<td>` (or the equivalent `style="text-align:…"`), defaulting to left. Because the
  underlying model stores one alignment per column (not per cell), the **header cell's**
  `align`/`text-align` is authoritative for its column; see invariant 5 for the full rule,
  including what happens when body cells disagree with the header or with each other.
- Degrade gracefully: a malformed/unterminated `<table>`, or a ragged table with
  inconsistent cell counts, renders without panicking — either as a best-effort normalized
  grid (padded to a uniform shape, as the existing model already does via
  `normalize_shape`) or, if unparseable, as literal text.

Out of scope (explicit non-goals):

- **`colspan` / `rowspan` — owned by follow-up issue #13953, not by this spec.**
  Spanning cells require a non-rectangular grid model that the current data model,
  layout, render, and selection code do not support. A `<td colspan>` or
  `<td rowspan>` is handled as a **degraded** single-cell (the span attribute is
  ignored and the cell occupies one slot), never a panic or corrupt layout — that
  degraded behavior is this spec's entire contribution to spans; anything beyond it is
  explicitly deferred. **Acceptance criteria for the split (both must hold for this PR
  to close #13726):** (1) this spec/PR delivers the un-spanned subset — simple tables
  and `<br>`-in-cell — with the degraded single-cell behavior above for any
  `colspan`/`rowspan` input, and (2) issue #13726 itself is narrowed (see "Issue
  linkage" below) so its remaining acceptance criteria describe only the un-spanned
  subset, with `colspan`/`rowspan` cross-linked to #13953 as the issue that owns them.
  If either half is missing — the degraded behavior isn't implemented, or #13726's body
  still reads as promising full span support — this PR must not close #13726.
- `<caption>`, `<colgroup>`/`<col>`, and nested tables inside a cell.
- Inline `<table>` mixed with other text on the same line.
- Arbitrary CSS beyond the discrete `align` / `text-align` used for alignment.
- Any change to GFM pipe-table behavior. HTML tables are an additional input path that
  produces the same internal table representation.
- Script execution / event handlers / navigation from table markup.

## Behavior

1. A Markdown document region delimited by `<table ...>` … `</table>` on their own lines
   renders as a table in the viewer, using Warp's existing table appearance (borders,
   dividers, alignment, horizontal scrolling for wide tables), identical to how a GFM
   pipe-table of the same content renders. The opening tag is recognized whether or not
   it carries attributes (`<table>`, `<table class="data">`, `<table id="x" class="y">`,
   …) — invariant 10's read-and-ignore rule for `class`/`id`/etc. only makes sense if the
   tag was recognized in the first place, so a `<table>` with ignored attributes is a
   detector match, never literal-text fallback.

2. Exactly one row becomes the table's header, chosen by this precedence (first rule that
   matches wins):

   - **The first `<tr>` inside `<thead>`.** If `<thead>` contains additional rows beyond
     the first, they are **not** dropped — they become ordinary data rows (rendered as
     plain body rows, no bold/header styling), preserved in document order ahead of
     `<tbody>`'s rows. This keeps all authored content visible rather than silently
     discarding rows the author clearly intended to show.
   - **If there is no `<thead>`, the first `<tr>` in the table, if it is composed of `<th>`
     cells** (majority or all `<th>`, matching common hand-authored HTML). Any *other*
     `<tr>` of `<th>` cells appearing later in the document (outside `<thead>`) is treated
     the same as an extra `<thead>` row above: demoted to an ordinary data row, not
     dropped, not concatenated into the header.
   - **If neither applies** (no `<thead>`, first row is not `<th>`-majority), the first
     `<tr>` in the table becomes the header regardless of cell tag, matching the existing
     model's invariant that every table has a header row.

   Data rows (from `<tbody>`, or any row demoted per the rules above) render as ordinary
   `<td>`-style rows even if their cells were tagged `<th>` in the source — the model has
   no per-row "this row is a header" flag beyond the single `headers` slot, so a demoted
   header row cannot retain header styling without a model change, which is out of scope
   for this slice.

3. Cell content renders with the viewer's inline formatting: bold, italic, inline code,
   strikethrough, links, and inline images (per the `<img>` spec) all render inside a cell
   exactly as they do in a GFM table cell.

4. A `<br>` inside a cell renders as a hard line break, so the cell's content occupies
   multiple lines within its row. The row's height grows to fit the tallest cell. This is
   distinct from the automatic word-wrapping the viewer already does for long cell content
   — `<br>` is an author-specified break that is honored regardless of column width. An
   author who wants the literal text `<br>` to appear in a cell (rather than a break) writes
   the escaped form, `&lt;br&gt;`, which renders as the visible characters `<br>` on a single
   line, never as a break — the escaped and unescaped forms are unambiguous and never
   collide, in either direction: exporting a cell containing the literal text `<br>` must
   re-escape it, and a cell round-tripped through copy/export must not turn an authored
   break into literal text or vice versa (see invariant 11 for the general round-trip rule;
   the tech spec defines the escaping mechanism).

5. Column alignment follows `align`/`text-align` where present, defaulting to left.
   Because `FormattedTable` stores one alignment **per column**, not per cell, disagreement
   within a column is resolved deterministically: the **header cell's** `align`/
   `text-align` for that column wins. If the header cell specifies no alignment (or there
   is no header, per invariant 2's fallback), the **first body cell in that column that
   specifies an alignment** wins. Any cell (header or body) that disagrees with the
   column's resolved alignment is **not** re-aligned individually — it renders at the
   column's alignment, matching the model's one-alignment-per-column shape. A column
   where no cell specifies an alignment defaults to left. Alignment renders identically to
   GFM table alignment.

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

9. Empty structures degrade cleanly, never a panic. `<table></table>` (no rows at all) is
   a well-formed empty table, not a fallback-to-text case (invariant 8 is for content the
   parser can't form into a grid; an empty table trivially is one) — it renders as a
   single-column table with one empty header cell and zero body rows, per the tech spec's
   "Empty structures" rule (tech.md §2), which is the same `normalize_shape` normalization
   every ragged table already goes through (invariant 6), not a separate empty-table code
   path. A `<tr></tr>` with no cells is **padded** to the
   table's column count with empty cells, not dropped — consistent with invariant 2's
   principle of never silently discarding an authored row.

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
