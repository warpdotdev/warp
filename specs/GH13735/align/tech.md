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
  generic **GUI widget-layout primitive**: wraps a `child: Box<dyn Element>`,
  positions it via a `Vector2F` alignment vector (`top_left`/`top_center`/…/
  `right`/`left`, `:28-66`) inside its own layout box, implements
  `SelectableElement` by delegating to the child. It is **not wired to Markdown
  rendering anywhere** — no reference to it exists in `crates/markdown_parser` or
  the Markdown render path. It is a **GUI-only paint strategy, not part of the
  content model**: it is a candidate for *how* the GUI surface draws an aligned
  group, but it says nothing about how alignment is *represented* in the buffer
  so that copy/export, the TUI, and selection all agree. See §6.
- The editor's own content model already has the vocabulary this feature needs.
  `BufferText` (`crates/editor/src/content/text.rs:541-570`) is the per-character
  buffer element enum, and it already distinguishes **two kinds of marker**:
  paired inline-style markers `BufferText::Marker { marker_type, dir }`
  (`:549-554`, Start/End) and block-level `BufferText::BlockMarker { marker_type:
  BufferBlockStyle }` (`:564-566`). `BufferBlockStyle`
  (`crates/editor/src/content/text.rs:867-893`) is the per-block style enum:
  `PlainText`, `Header`, `UnorderedList`, `OrderedList { number, indent_level }`,
  `TaskList`, `CodeBlock`, `Table`. Ordered lists carry their own per-block
  metadata (`number`, `indent_level`) in the marker and are tracked through the
  buffer's `SumTree`. `FormattedTextLine` (`crates/markdown_parser/src/lib.rs`)
  is the *parser output* enum that `buffer.rs` maps **to and from** this content
  model (`crates/editor/src/content/buffer.rs`: `from_markdown` at `:843`,
  `BufferBlockStyle → FormattedTextLine` serialization at `:5989+`).

**Conclusion: this is a content-model addition. The right layer to model it is the
editor buffer's block-marker + `SumTree` mechanism (the same layer ordered lists
live in), not a wrapper variant in the `markdown_parser` `FormattedTextLine` enum.**
The reasoning — and the precedent that decides it — is in design question 1.

## Design questions this spec must answer

1. **Representation: where does alignment live?** This is the load-bearing
   decision, and there is direct maintainer precedent for it. On the sibling
   spec PR **#13345** (`<details>`/`<summary>`, [PR](https://github.com/warpdotdev/warp/pull/13345)),
   maintainer **bnavetta** ruled twice against bolting region state onto an
   adjacent mechanism, and those rulings apply verbatim to alignment:

   - Against replicating the `Table` mechanism:
     > "The approach taken by `Table` is very much an intermediate
     > implementation… We shouldn't try to replicate it here… Given that you're
     > supporting nested details items, I'd look at lists for inspiration
     > instead (specifically ordered lists)… That approach should also make it
     > fairly doable to track nested details depth in the `SumTree`."
     ([review comment](https://github.com/warpdotdev/warp/pull/13345#discussion_r3545165725))

   - On why a region must **compose** with its body's own block style rather
     than replace it:
     > "We assume that there's only one `BufferBlockStyle` active for a given
     > character, so we'll need a way to express that something is a code block
     > within a details section, for example — that seems easier if details are
     > not themselves block styles… details start/end markers should be new
     > top-level variants of `BufferText`… we'd then be able to consult the
     > `SumTree` to check both whether or not it's part of a details section and
     > what its specific style is."
     ([review comment](https://github.com/warpdotdev/warp/pull/13345#discussion_r3552861589))

   Alignment is the same class of problem as a details region: it is a property
   of a *span of blocks*, and the span's blocks keep their own styles (a code
   block inside `<div align="center">` must still be a code block). bnavetta's
   own analogy is instructive — he notes this is "equivalent to how link
   start/end markers don't use quite the same modeling as data-less markers for
   other inline styles like bold and italic," and the buffer already reflects
   that: `BufferText::Link(LinkMarker)` and `BufferText::Marker { dir }` are
   distinct variants (`text.rs:549-555`).

   - **Recommended: alignment as start/end block markers + a `SumTree`
     dimension.** Introduce paired `BufferText` markers — conceptually
     `BufferText::AlignStart { alignment }` / `BufferText::AlignEnd` (top-level
     `BufferText` variants, *not* a new `BufferBlockStyle`), plus a `SumTree`
     dimension so that at render time the buffer can be asked "is this character
     inside an aligned region, and if so, with what alignment?" independently of
     its per-block style. This is the exact shape bnavetta steered the #13345
     author toward (ordered-list precedent: `BufferBlockStyle::OrderedList {
     number, indent_level }` carries per-block metadata in the marker and is
     tracked through the `SumTree`, `text.rs:882-885` / `:918-923`). It composes
     by construction: "aligned + code block" is representable because alignment
     and block-style are two orthogonal `SumTree` queries, not one mutually
     exclusive `BufferBlockStyle` slot. It also generalizes cleanly to the
     `<div align>` **group** case (product invariant 1) — the start/end markers
     bracket any number of interior blocks — and the single-paragraph
     `<p align>` case (invariant 2) is just a region of length one.

     Representation is **surface-agnostic**: the markers and the `SumTree`
     dimension are the content model. *How* each surface paints an aligned
     region is a separate, per-surface concern (GUI may wrap in
     `warpui_core::Align`; TUI computes horizontal offsets against terminal
     width — see §6 and the TUI scope decision). The content model does not
     name `warpui_core::Align`.

   - **Rejected alternative — `FormattedTextLine::AlignedBlock` wrapper
     variant.** An earlier draft recommended a new `markdown_parser`
     `FormattedTextLine::AlignedBlock(Vec<FormattedTextLine>)` wrapper carrying
     an alignment value. This is rejected on bnavetta's #13345 grounds. Two
     problems:

     1. It is the parser-layer analogue of making alignment a block style: when
        `buffer.rs` maps `FormattedTextLine` into the editor content model, an
        `AlignedBlock` wrapper has to become *some* `BufferBlockStyle`, which
        walks straight into "only one `BufferBlockStyle` active for a given
        character." "Aligned **and** a code block" cannot be expressed if
        alignment is modeled as (or collapses into) a block style — precisely
        the composition failure bnavetta called out. The marker approach avoids
        this because alignment never occupies the block-style slot.
     2. It changes the shared `FormattedTextLine` enum every consumer matches on
        exhaustively (see the Risks section for the concrete blast radius), for
        a *parser-side* grouping that then has to be *un*-grouped again when
        lowering into the flat buffer content model. The marker approach keeps
        the parser output flat and expresses the region in the layer that
        actually stores and renders it.

   - **Rejected alternative — alignment as a side-table keyed by block
     index/id.** `FormattedTextLine` has no stable identity today; this would
     invent one purely for this feature, adding indirection the `SumTree`
     dimension already provides for free.

   - **Rejected alternative — a per-block `alignment` flag on every existing
     variant.** Threading `alignment: BlockAlignment` through `Heading`, `Line`,
     `Image`, etc. individually both loses the `<div align>` grouping
     (invariant 1) and duplicates a property the region markers express once for
     the whole span.

2. **New alignment enum.** A small `Left` (default) / `Center` / `Right` enum
   carried by the start marker (and, at the GUI paint layer, mapped to a
   `Vector2F` for `warpui_core::Align`). It mirrors the *shape* of
   `TableAlignment` (`lib.rs:346-351`) but is a distinct type: table *column*
   alignment and *block-region* alignment are different axes that merely share
   three variant names, and conflating them would couple unrelated concerns the
   first time either needs to diverge (e.g. `justify` added to one but not the
   other).

3. **Grouping rule for `<div align>` (product invariant 1).** The issue's test
   case has the block content separated from the tag by blank lines — this
   matches the shape sibling specs (`<details>`/`<summary>`, HTML tables) use for
   own-line raw-HTML block detection. Recommend a block-level detector in
   `markdown_parser.rs` (same pattern as those siblings: scan for an own-line
   `<div align="…">` / `<p align="…">`, find the matching own-line closing tag,
   recursively parse the raw content between them as ordinary Markdown). The
   detector's job is to emit an **alignment-start signal, the interior blocks
   parsed exactly as if standalone, and an alignment-end signal** — the interior
   is *not* wrapped in a new parser variant; it stays a flat run of ordinary
   `FormattedTextLine`s bracketed by the region boundary. When `buffer.rs` lowers
   this into the content model it inserts the paired `BufferText` align markers
   (design question 1) around the interior blocks' `BufferText`. Recursion reuses
   the existing top-level parse function rather than inventing a second grammar.

4. **`<p align>` vs `<div align>` — single block vs. group.** Both route through
   the same detector and emit the same paired region markers; the only difference
   is `<p>`'s content is inline phrasing (parsed via `parse_phrasing_content`,
   matching how `p` is treated elsewhere) and produces exactly one interior
   `Line`, while `<div>` content is full block-level Markdown and can produce any
   number of interior blocks. A `<p align>` region is just an aligned region of
   length one — no separate code path.

5. **Nested aligned blocks (product non-goal, but must not crash).** If a
   `<div align="center">` contains a nested `<p align="right">`, the recursive
   parse of the div's inner content detects the inner region and emits its own
   paired markers *inside* the outer pair. Because alignment is a `SumTree`
   dimension rather than a single mutually-exclusive slot, the render layer can
   see both regions; recommend **innermost wins** for the nested block's own
   content (the nearest enclosing align marker governs), while the outer
   alignment still governs blocks that are only inside the outer region. This is
   a query over the marker stack, not special-cased recursion logic — the only
   requirement is a test asserting it doesn't panic or infinite-loop.

6. **Render layer: how does each surface paint an aligned region?** This is a
   **per-surface paint concern, downstream of the content model** — the markers
   and `SumTree` dimension (design question 1) are surface-agnostic; each
   renderer decides how to turn "this run of blocks is centered" into pixels or
   cells.

   - **GUI.** `warpui_core::elements::gui::align::Align` (`align.rs:12-132`) is
     the natural paint primitive: wrap the rendered block-region element in it
     (`left()`/`right()`/default-center via `Vector2F`). **This is a GUI-only
     paint strategy, not the content model** — the buffer stores the region via
     markers regardless of whether GUI happens to use `Align`. If the Markdown
     block renderer doesn't compose with arbitrary `dyn Element` wrapping (verify
     against `crates/editor/src/render/element/` and `render/model/mod.rs`), the
     GUI fallback is per-line x-offsets computed in the block layout pass; either
     way it is a GUI implementation detail behind the same content model.
   - **TUI.** See the TUI surface disposition below — the terminal surface reads
     the same markers but positions within terminal width, with a defined
     fallback. The content model does not privilege either surface.

   Because alignment is not `warpui_core::Align`-specific at the model layer, the
   GUI paint choice can be prototyped (start with `Align`-wrapping, given it is
   purpose-built) without blocking the TUI or the serialization work.

### TUI surface disposition

Master's TUI Markdown renderer (`crates/warp_tui/src/tui_markdown.rs`) consumes
`FormattedTextLine` directly and implements its **own** alignment for tables,
separately from the GUI, in `crates/warp_tui/src/tui_markdown/table.rs`
(`aligned_cell_spans`, `:260`, padding per `TableAlignment`). Block-region
alignment must therefore have an explicit TUI disposition rather than assuming
the GUI `Align` element covers it (it does not — `warpui_core::Align` is not a
TUI concept).

**Scope decision:** the TUI renders an aligned region with **best-effort
horizontal positioning within the terminal width** (compute the region's
rendered width, pad leading columns per the alignment, mirroring how
`aligned_cell_spans` pads table cells), and **falls back to left-aligned** where
the region's width can't be determined or exceeds the pane. This keeps the TUI
consuming the same content-model markers as the GUI while acknowledging the
terminal's coarser layout model; it is an explicit scope decision, not an
oversight. (This mirrors bnavetta's #13345 framing that region state lives in the
buffer so every surface can consult it — the TUI reads the same `SumTree`
dimension the GUI does.)

7. **Feature gating.** No existing flag covers this (`MarkdownTables` is
   table-specific). Recommend a new flag, e.g. `FeatureFlag::MarkdownBlockAlign`,
   following the same gating pattern used for `MarkdownTables` in
   `crates/editor/src/content/buffer.rs:850-855` (the `from_markdown` parse-fn
   switch), so the feature can ship dark and be enabled independently of
   unrelated Markdown work.

## Security

Only `align` and `style="text-align:…"` are read, and only the three recognized
values are consulted; any other `style` property or attribute (`onclick`,
`class`, `id`, arbitrary CSS) is ignored, matching the pattern already
established for the tables spec (invariant 10 there). No script/event-handler
surface. Nested content inherits the viewer's existing trust boundary — an
aligned region's interior blocks are ordinary `FormattedTextLine`s parsed the same
way as unaligned content, and the region itself is expressed only by content-model
markers, so no new content-injection surface is introduced beyond "this content is
now positioned differently."

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<div align="center">` wrapping a heading + blank line + text → a `Center`
  aligned region bracketing the expected interior `Vec<FormattedTextLine>`
  (assert the region markers are emitted around the interior, and the interior
  blocks parse as normal — invariant 1).
- `<p align="right">caption</p>` → a `Right` aligned region bracketing exactly
  one interior `Line` (invariant 2).
- `style="text-align: center"` → same result as `align="center"` (invariant 3).
- `align` and conflicting `style` both present → `style` wins (invariant 3).
- Unrecognized value (`align="justify"`, `align="bogus"`) → unaligned
  (`Left`/no wrapping), not an error (invariant 4).
- Nested content inside an aligned block still parses with normal semantics
  (heading stays `Heading`, not flattened to `Line`) (invariant 5).
- Nested `<div align><p align></div>` → innermost governs its own content,
  no panic, no infinite recursion (design question 5).
- Unterminated `<div align="center">` (no matching close) → renders as
  literal text, rest of document unaffected (invariant 7).
- `align`/`text-align` on content the block detector can't safely group →
  renders as the unaligned equivalent, normal Markdown semantics preserved
  (invariant 7).

### Round-trip (`crates/markdown_parser` + `crates/editor/src/content/text_tests.rs`)

- Aligned region → internal/export format → re-parsed, alignment preserved
  (invariant 6). Serialization re-emits a `<div align="…">`/`<p align="…">`
  wrapper around the re-serialized interior blocks (the `BufferBlockStyle →
  FormattedTextLine` path at `buffer.rs:5989+` reads the region markers and
  brackets the interior). Per bnavetta on #13345, exact source-Markdown
  preservation is explicitly **not** a goal — "Warp's rich-text pipeline doesn't
  attempt to guarantee exact preservation of the source Markdown; I think it's
  fine to continue that here"
  ([review comment](https://github.com/warpdotdev/warp/pull/13345#discussion_r3545145887)).
  The round-trip test therefore asserts *alignment survives* (semantic
  round-trip), not byte-identical re-emission, and is distinct from the tables
  spec's tab-delimited internal-format test since this content is
  block-structured, not tabular.

### Layout / render tests

- Centered/right/left group renders with the expected horizontal offset
  relative to pane width, at multiple pane widths (confirms it's dynamic
  positioning, not a fixed pixel offset).
- A heading inside an aligned group still measures/wraps/paints as a heading
  (font size, weight) — alignment doesn't downgrade block semantics.
- An aligned group containing unsupported content (e.g. `<img>` before #13721
  lands, rendering as literal text via `FormattedTextElement`
  (`crates/warpui_core/src/elements/gui/formatted_text_element.rs`), the same
  sink every other `FormattedTextLine` variant renders through) still
  positions that literal text per the group's alignment — the two features
  are independently testable.
- **TUI** (`crates/warp_tui/src/tui_markdown_tests.rs`): an aligned region
  renders with best-effort horizontal positioning within a fixed terminal
  width, and falls back to left-aligned when the region width exceeds the pane
  (the explicit TUI scope decision) — asserting the terminal surface reads the
  same content-model markers as the GUI, not a GUI-only path.

### Integration / manual

Per CONTRIBUTING, before/after screenshots of the issue's exact test case
(centered `<div>` hero + centered `<p>` caption), plus a right-aligned variant
exercising the README "anchored project image" pattern from the issue
description (image-as-literal-text is acceptable until #13721 lands, but its
horizontal position must be correct).

## Risks and follow-ups

- **Marker-model blast radius is narrower than the rejected wrapper, but real.**
  Because alignment rides existing channels — new `BufferText` variants and a
  `SumTree` dimension, following the ordered-list precedent — it does **not**
  add an arm to every `match` over `FormattedTextLine`. The touched surface is:
  the `markdown_parser` block detector (`markdown_parser.rs`, plus
  `html_parser.rs` if the paste path is in scope), the `buffer.rs` lower/serialize
  bridge (insert markers around interior blocks on parse; bracket on serialize),
  and the `SumTree` summary/dimension plumbing that answers "is this character
  inside an aligned region." Any code that exhaustively matches `BufferText`
  (e.g. `find.rs:338`, which already special-cases `BufferText::BlockMarker`)
  gains a new-variant arm — a bounded, compiler-enforced set, not the full
  `FormattedTextLine` fan-out.

  For contrast, the **rejected `FormattedTextLine::AlignedBlock` wrapper** (design
  question 1) *would* have added a required arm to every exhaustive match over
  `FormattedTextLine`. That enum is referenced across **20 files** in `crates/`;
  the production match sites that would have needed a new arm span not just the
  obvious Markdown files (`markdown_parser` `lib.rs` / `markdown_parser.rs` /
  `html_parser.rs`; `editor` `content/{buffer,core,markdown,text}.rs`) but two
  non-obvious ones worth calling out explicitly: the **Jupyter notebook parser**
  (`crates/ipynb_parser/src/lib.rs`) and the **TUI** (`crates/warp_tui/
  src/tui_markdown.rs` and `tui_plan_view.rs`), plus the GUI element sink
  (`crates/warpui_core/src/elements/gui/formatted_text_element.rs`). The wrapper's
  cost was paid across all of those *and* still failed bnavetta's composition
  constraint — which is the substantive reason to reject it, the arm-count being
  secondary. The marker design pays a smaller, differently-shaped cost and
  composes.
- **Render-layer approach is per-surface and partly unconfirmed** (design
  question 6). GUI: whether `warpui_core::Align` composes cleanly with the
  Markdown block renderer needs a spike; the manual per-line-offset fallback is
  more invasive and only taken if element composition doesn't fit. TUI: the
  best-effort-within-terminal-width behavior with left-aligned fallback is a
  deliberate scope decision (see the TUI surface disposition), not full parity
  with the GUI. Neither surface's paint choice affects the content model.
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
