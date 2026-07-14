# TECH.md — Markdown viewer: `align` attribute on `<p>`/`<div>` blocks

Product spec: `specs/GH13735/align/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13735

## Context

Unlike the `<table>` slice (#13652 tables), where a shared `FormattedTable` model
and full layout/render/scroll/selection path already existed and only needed a
new *input path*, block alignment has **no existing content-model concept to
extend**. Verified:

- `FormattedTextLine` (`crates/markdown_parser/src/lib.rs:156-168`) is a flat enum —
  `Heading`, `Line`, `OrderedList`, `UnorderedList`, `CodeBlock`, `TaskList`,
  `LineBreak`, `HorizontalRule`, `Embedded`, `Image`, `Table`. Every variant is a
  single unit of content; **none wraps a sequence of child `FormattedTextLine`s**.
  There is no block-container variant at all.
- `FormattedTextStyles` (`lib.rs:545-552`) — `weight`, `italic`, `underline`,
  `strikethrough`, `inline_code`, `hyperlink` — is inline character styling. No
  alignment field, and it wouldn't fit here anyway since alignment is a
  block-level property, not a per-character one.
- `markdown_parser.rs` (the file-viewer's own Markdown grammar, distinct from the
  paste-oriented HTML importer) has **zero** references to `div`, `p`, or
  `center` — no block-level HTML detection of any kind beyond what already
  exists for images/tables in sibling specs.
- `html_parser.rs` (paste path): `div` is in `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP`
  (`:23-25`) — the tag is unwrapped and its children flattened, attributes
  discarded entirely. `p` is **not** in that skip list and has no explicit match
  arm either; it falls through to the `_ =>` catch-all (`:337-349`), which parses
  the node's children as pending inline/block content via
  `parse_pending_inline_nodes` — again, the `p` tag's own attributes are never
  read.
- The closest in-repo precedent for "alignment as part of the content model" is
  `TableAlignment` (`lib.rs:346-351`, `Left`/`Center`/`Right`, `#[default] Left`)
  — but it is scoped narrowly to GFM table **columns** (`FormattedTable.alignments:
  Vec<TableAlignment>`, one entry per column) and is consumed only by the table
  layout/render path. It is a useful naming/shape precedent, not a reusable
  mechanism — block alignment needs a per-*block* (or per-*block-group*)
  property, not a per-column one.
- `crates/warpui_core/src/elements/gui/align.rs` (`Align` element, `:12-132`) is a
  generic widget-layout primitive: wraps a `child: Box<dyn Element>`, positions
  it via a `Vector2F` alignment vector (`top_left`/`top_center`/…/`right`/`left`,
  `:28-66`) inside its own layout box, implements `SelectableElement` by
  delegating to the child. It is **not wired to Markdown rendering anywhere** —
  no reference to it exists in `crates/markdown_parser` or the Markdown render
  path. It is, however, exactly the shape of primitive block alignment needs at
  the paint layer, if the Markdown block renderer can be made to use
  `warpui_core` elements (needs confirmation — see open question below).

**Conclusion: this is a content-model addition, not an extension of an existing
field.** The shape most analogous in spirit is `TableAlignment`, but the actual
change is closer to introducing a new `FormattedTextLine` variant than to adding
a field to an existing one.

## Design questions this spec must answer

1. **Representation: new variant vs. wrapper field.**
   - **Option A (recommended): new `FormattedTextLine::AlignedBlock` variant**
     wrapping a `Vec<FormattedTextLine>` (the grouped child blocks) plus an
     alignment value:
     ```rust
     AlignedBlock(AlignedBlockContent),
     // where
     pub struct AlignedBlockContent {
         pub alignment: BlockAlignment, // new enum: Left / Center / Right
         pub lines: Vec<FormattedTextLine>,
     }
     ```
     This directly matches product invariant 1 (`<div align>` groups multiple
     blocks) and invariant 2 (`<p align>` is the single-block-content case,
     represented as an `AlignedBlock` with exactly one `Line` inside). It makes
     the container explicit and composable with every existing renderer for
     `FormattedTextLine` children, since the renderer only needs one new case
     that says "lay these children out, then align the group."
   - **Option B: alignment as a side-table keyed by block index/id.** Rejected —
     `FormattedTextLine` has no stable identity today, and this would need one
     invented purely for this feature, adding indirection with no offsetting
     benefit over Option A.
   - **Option C: alignment as a style flag threaded through every existing
     variant** (e.g. an `alignment: BlockAlignment` field added to `Heading`,
     `Line`, `Image`, etc. individually). Rejected — this is the "ripples through
     everything" version of Option A without the grouping capability
     `<div align>` needs (product invariant 1), and it would require touching
     every variant's construction site instead of one new one.

   Recommend **Option A**, flagged for maintainer review since (like the tables
   spec's cell-model choice) it changes the shared `FormattedTextLine` enum that
   every consumer matches on exhaustively — every existing `match` over
   `FormattedTextLine` (parser, layout, render, plain-text/export, selection)
   gains one new arm. Grep `FormattedTextLine::` across `crates/` before
   implementation to enumerate every match site and confirm none can silently
   compile with a wildcard `_ =>` that would swallow the new variant
   incorrectly.

2. **New enum: `BlockAlignment`.** Mirrors `TableAlignment`'s shape
   (`lib.rs:346-351`) for consistency: `Left` (default) / `Center` / `Right`.
   Kept as a distinct type from `TableAlignment` rather than reused, since the
   two are semantically different axes (table *column* alignment vs. Markdown
   *block* alignment) that happen to share three variant names — reusing one
   enum for both would conflate unrelated concerns the first time either needs
   to diverge (e.g. if `justify` is ever added to one but not the other).

3. **Grouping rule for `<div align>` (product invariant 1).** The issue's test
   case has the block content separated from the tag by blank lines — this
   matches the shape sibling specs (`<details>`/`<summary>`, HTML tables) use for
   own-line raw-HTML block detection. Recommend a block-level detector in
   `markdown_parser.rs` (same pattern as those siblings: scan for an own-line
   `<div align="…">` / `<p align="…">`, find the matching own-line closing tag,
   extract the raw content between them, recursively parse *that* content as
   ordinary Markdown to get `Vec<FormattedTextLine>`, then wrap in
   `AlignedBlock`). Recursion here reuses the existing top-level parse function
   rather than inventing a second grammar — the aligned region's content is
   parsed exactly as if it were a standalone document.

4. **`<p align>` vs `<div align>` — single block vs. group.** Both route through
   the same detector and `AlignedBlock` variant; the only difference is `<p>`'s
   content is inline phrasing (parsed via `parse_phrasing_content`, matching how
   `p` is treated elsewhere) and produces exactly one child `Line`, while `<div>`
   content is full block-level Markdown and can produce any number of children.
   This keeps one code path instead of two.

5. **Nested aligned blocks (product non-goal, but must not crash).** If a
   `<div align="center">` contains a nested `<p align="right">`, the recursive
   parse of the div's inner content will itself detect and emit a nested
   `AlignedBlock`. Recommend **innermost wins** for the nested block's own
   content (natural consequence of recursion — no special-casing needed) while
   the outer alignment still governs the group's overall horizontal position
   relative to the pane. This falls out of Option A's recursive design for free
   and needs no extra logic, only a test asserting it doesn't panic or infinite-
   loop.

6. **Render layer: how does an aligned group actually get positioned?**
   Needs confirmation from the render/layout owner, but two candidate
   approaches:
   - Reuse `warpui_core::elements::gui::align::Align` (`align.rs:12-132`) by
     wrapping the rendered block-group element in it — this is exactly the
     primitive's purpose (`left()`/`right()`/default-center via `Vector2F`
     alignment), just not currently wired to Markdown content. This is the
     natural reuse point the issue itself points at.
   - If the Markdown block renderer doesn't compose with arbitrary
     `dyn Element` wrapping (needs verification against
     `crates/editor/src/render/element/` and `render/model/mod.rs`, the same
     files the tables spec's render layer touches), the alternative is
     computing per-line x-offsets directly in the block layout pass, mirroring
     how table column widths are computed in `measure_table_cells`
     (`crates/editor/src/content/edit.rs:260-323`) rather than delegating to a
     generic widget. This is more invasive but avoids introducing a dependency
     from the Markdown render path onto a generic UI primitive if that boundary
     doesn't currently cross.
   The tech implementation should start by prototyping the `Align`-wrapping
   approach given it is purpose-built and already exists; fall back to manual
   offset computation only if element composition doesn't fit the render
   pipeline's structure.

7. **Feature gating.** No existing flag covers this (`MarkdownTables` is
   table-specific). Recommend a new flag, e.g. `FeatureFlag::MarkdownBlockAlign`,
   following the same gating pattern as `buffer.rs:850-855`, so the feature can
   ship dark and be enabled independently of unrelated Markdown work.

## Security

Only `align` and `style="text-align:…"` are read, and only the three recognized
values are consulted; any other `style` property or attribute (`onclick`,
`class`, `id`, arbitrary CSS) is ignored, matching the pattern already
established for the tables spec (invariant 10 there). No script/event-handler
surface. Nested content inherits the viewer's existing trust boundary — an
`AlignedBlock`'s children are ordinary `FormattedTextLine`s parsed the same way
as unaligned content, so no new content-injection surface is introduced beyond
"this content is now positioned differently."

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<div align="center">` wrapping a heading + blank line + text → `AlignedBlock`
  with `Center` and the expected child `Vec<FormattedTextLine>` (invariant 1).
- `<p align="right">caption</p>` → `AlignedBlock` with `Right` and exactly one
  `Line` child (invariant 2).
- `style="text-align: center"` → same result as `align="center"` (invariant 3).
- `align` and conflicting `style` both present → `style` wins (invariant 3).
- Unrecognized value (`align="justify"`, `align="bogus"`) → unaligned
  (`Left`/no wrapping), not an error (invariant 4).
- Nested content inside an aligned block still parses with normal semantics
  (heading stays `Heading`, not flattened to `Line`) (invariant 5).
- Nested `<div align><p align></div>` → innermost governs its own content,
  no panic, no infinite recursion (design question 5).
- Unterminated `<div align="center">` (no matching close) → falls back to
  literal-text or unaligned rendering, rest of document unaffected
  (invariant 7).

### Round-trip (`crates/markdown_parser` + `crates/editor/src/content/text_tests.rs`)

- Aligned block → internal/export format → re-parsed, alignment preserved
  (invariant 6). Tech spec's chosen serialization (likely re-emitting the
  `<div align="…">`/`<p align="…">` wrapper around the re-serialized children)
  needs its own round-trip test distinct from the tables spec's tab-delimited
  internal format, since this content is block-structured, not tabular.

### Layout / render tests

- Centered/right/left group renders with the expected horizontal offset
  relative to pane width, at multiple pane widths (confirms it's dynamic
  positioning, not a fixed pixel offset).
- A heading inside an aligned group still measures/wraps/paints as a heading
  (font size, weight) — alignment doesn't downgrade block semantics.
- An aligned group containing unsupported content (e.g. `<img>` before #13721
  lands, rendering as literal text) still positions that literal text per the
  group's alignment — the two features are independently testable.

### Integration / manual

Per CONTRIBUTING, before/after screenshots of the issue's exact test case
(centered `<div>` hero + centered `<p>` caption), plus a right-aligned variant
exercising the README "anchored project image" pattern from the issue
description (image-as-literal-text is acceptable until #13721 lands, but its
horizontal position must be correct).

## Risks and follow-ups

- **No existing content-model hook, unlike the tables slice.** This is a new
  `FormattedTextLine` variant, meaning every exhaustive match over the enum
  across `crates/` gains a required arm. Enumerate those match sites early (see
  design question 1) — this is the main risk of scope creep if a match site is
  missed and silently mishandled via a wildcard arm.
- **Render-layer approach is unconfirmed** (design question 6). Whether the
  `warpui_core::Align` element composes cleanly with the Markdown block
  renderer needs a spike before committing to that path; the manual-offset
  fallback is more invasive and should only be taken if composition doesn't
  fit.
- **Float/wrap (`<img align="right">` interleaving prose) is explicitly
  deferred** (product non-goal) but is the more visually striking half of the
  "right-anchored README image" pattern the issue motivates. This spec commits
  to correct block *positioning* only; if maintainers want the wrap behavior
  too, it is a substantially larger follow-up (needs float-equivalent layout)
  tracked against #13721.
- **Feature flag choice** (design question 7) should be confirmed with the
  team rather than assumed — it's possible this should ride an existing
  general "raw HTML subset" flag if one gets introduced for the sibling specs
  in this split (tables, kbd, sub/sup, br), rather than a bespoke one.
