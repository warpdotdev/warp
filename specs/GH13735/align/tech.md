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

   - **Parser-to-buffer contract: how the region boundaries reach the buffer.**
     The buffer stores the region as top-level `BufferText` markers (above), but
     those markers are *inserted during lowering* — the parser has to hand the
     boundaries to the lowering pass somehow. The lowering pass is a single
     stateful line-by-line loop over the `FormattedTextLine` stream: `edit`
     (`crates/editor/src/content/core.rs:567`, reached from `from_markdown` →
     `from_formatted_text` → `replace_with_formatted_text` →
     `edits_for_formatted_text` → the `Insert` action dispatched at
     `core.rs:447-457`) matches each `FormattedTextLine` variant and pushes
     `BufferText` for it, tracking `previous_block_type` across lines. That loop
     is the *only* place parser output becomes buffer content, so the boundaries
     must ride the `FormattedTextLine` stream to reach it.

     **Represent each boundary as a zero-content marker line in the parser
     stream**: two new content-less `FormattedTextLine` variants,
     `FormattedTextLine::AlignRegionStart(BlockAlignment)` and
     `FormattedTextLine::AlignRegionEnd`. The block detector (design question 3)
     emits `AlignRegionStart`, then the interior blocks parsed exactly as if
     standalone, then `AlignRegionEnd` — a flat run, no wrapper. The `edit` loop
     gains two arms that translate them directly into the paired top-level
     `BufferText::AlignStart { alignment }` / `BufferText::AlignEnd` markers,
     exactly as the `OrderedList` arm (`core.rs:781-825`) translates its line
     into a `BufferBlockStyle::OrderedList` marker. This mirrors the ordered-list
     precedent end to end: parser-side metadata (there `number`/`indent_level`;
     here `alignment`) originates in the parser
     (`crates/markdown_parser/src/markdown_parser.rs:740-759` builds
     `OrderedList`), rides as data on a `FormattedTextLine`, and is translated to
     a `BufferText` marker by the same `edit` loop.

     The reason boundaries are *their own* content-less variants rather than an
     `alignment` field on the existing line variants: a per-line field would
     have to be consumed at the point where `edit` produces a `BufferBlockStyle`
     for that line, which is precisely the "one `BufferBlockStyle` per character"
     collision this design rejects (and `FormattedIndentTextInline`'s reuse
     across `OrderedList`/`UnorderedList`, `lib.rs:315-319`, is a *block-style*
     attribute for exactly that reason). Content-less boundary lines sidestep it:
     they carry no `BufferBlockStyle`, translating only into the orthogonal
     top-level align markers, so "aligned **and** a code block" stays two
     independent facts. `LineBreak` and `HorizontalRule` (`lib.rs:161-162`) are
     the existing precedent for a content-less `FormattedTextLine` the `edit`
     loop handles without emitting a block style (`core.rs:570`, `:868`).

     Reverse serialization (`buffer.rs` ~`:6026`, the `BufferBlockStyle →
     FormattedTextLine` path) brackets the interior on the way out: on hitting an
     `AlignStart`/`AlignEnd` marker it emits the corresponding boundary line (or,
     equivalently, wraps the interior's re-serialized `FormattedTextLine`s in the
     `<div align>`/`<p align>` text form), symmetric with how the ordered-list
     block style is re-serialized there.

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
   recursively parse the raw content between them as ordinary Markdown). It reads
   the opening tag's `align`/`style` attributes through the single **Attribute-matching
   contract** below (the same contract the single-line detector uses), so both
   detector paths resolve `align=`/`style=`, the conflict winner, unrelated
   attributes, and malformed syntax identically. The
   detector's job is to emit a **`FormattedTextLine::AlignRegionStart(alignment)`
   boundary line, the interior blocks parsed exactly as if standalone, and a
   `FormattedTextLine::AlignRegionEnd` boundary line** (the parser-to-buffer
   contract under design question 1) — the interior is *not* wrapped in a new
   parser variant; it stays a flat run of ordinary `FormattedTextLine`s bracketed
   by the two content-less boundary lines. When the `edit` lowering loop
   (`core.rs:567`) reaches those boundary lines it inserts the paired top-level
   `BufferText` align markers (design question 1) around the interior blocks'
   `BufferText`. Recursion reuses the existing top-level parse function rather
   than inventing a second grammar.

   **Single-line `<p align="…">caption</p>` (product invariant 2, same-line
   shape).** The own-line detector above assumes the opening tag, interior
   content, and closing tag each occupy their own line(s) — but the product
   spec's common "centered caption" case, and the acceptance test case itself
   (`<p align="center">A centered caption line.</p>`), is a single *source*
   line containing open-tag, inline content, and close-tag together. This needs
   its own detection path because it doesn't have a separate own-line close tag
   for the own-line scanner to find.

   The parser is line-oriented at the top level (`parse_line`/
   `not_markdown_line_ending`, `markdown_parser.rs:955-958`, split the input
   into lines that each become one `FormattedTextLine`, e.g. `parse_markdown_line`
   producing a single `Line`). The single-line `<p align>` case is detected at
   that same grain: before falling through to ordinary paragraph/line parsing,
   check whether the line, once leading whitespace is stripped, matches
   `<p (align="…"|style="…")>...</p>` **entirely on that one line** — open tag,
   then content, then close tag, with nothing before the open tag and nothing
   after the close tag but trailing whitespace. On a match, extract the
   alignment (the **Attribute-matching contract** below, applied to this
   tag's attributes) and the inline content between the tags, then emit the
   *same three-part shape* the own-line detector emits, collapsed onto one
   source line: `AlignRegionStart(alignment)`, one interior `Line` built by
   running the extracted content through `parse_phrasing_content` (the existing
   inline-phrasing path, matching how the own-line `<p align>` case parses its
   content per design question 4), then `AlignRegionEnd`. This is not a fourth
   representation — it is the same `AlignRegionStart` / interior / `AlignRegionEnd`
   triple as the own-line and `<div>` cases, just produced by a detector that
   reads one line instead of scanning for a separate closing line. No wrapper
   variant, no special buffer-side handling: the `edit` loop and TUI renderer
   (design question 1, TUI surface disposition) see an ordinary
   `AlignRegionStart` / `Line` / `AlignRegionEnd` run regardless of which
   detector produced it.

   **Mixed same-line cases (text sharing the line with the tags) — literal
   fallback, deterministically.** If the line contains a same-line `<p align>`
   open+content+close *plus* additional non-whitespace text before the open tag
   or after the close tag (e.g. `Note: <p align="center">caption</p>` or
   `<p align="center">caption</p> — see above`), the single-line detector does
   **not** partially apply alignment to a fragment of the line. Consistent with
   invariant 7's malformed-input philosophy ("content the block detector can't
   safely group renders as the unaligned equivalent"), the simplest
   deterministic rule is chosen: **the whole line is rejected by this detector
   and falls through to ordinary paragraph/inline parsing**, exactly as it does
   today — the `<p align="…">`/`</p>` tags render as literal text (the existing
   fallthrough behavior for HTML tags without a dedicated block match, per the
   `p`-tag handling already established in `html_parser.rs`'s `_ =>` catch-all
   for the paste path), and no `AlignRegionStart`/`AlignRegionEnd` pair is
   emitted. This avoids inventing a partial-region concept (e.g. "align only
   the tag-bracketed substring, leave surrounding text unaligned on the same
   line") that has no product requirement and no rendering precedent in this
   codebase — `FormattedTextLine` has no sub-line alignment concept, only
   whole-line/whole-region. A test case should assert this literal-fallback
   behavior explicitly (see Testing and validation) so the boundary is
   documented and doesn't regress into silent partial-application.

   **Attribute-matching contract.** Both detector paths above — the own-line
   `<div>`/`<p>` scanner and the single-line `<p>` matcher — resolve a tag's
   alignment by reading its `align`/`style` attributes through *this one shared
   contract*, so they never diverge on how an attribute is matched. It is the
   single place both cite; the product spec's `style` micro-grammar (product
   invariant 3) is the value-parsing half of it, restated here once so the
   contract is self-contained:

   - **Attribute names are matched case-insensitively.** `align`, `ALIGN`,
     `Align` are the same attribute; `style`, `STYLE`, `Style` likewise. HTML
     attribute names are case-insensitive and this contract follows that.
   - **Values may be double-quoted, single-quoted, or unquoted.**
     `align="center"`, `align='center'`, and `align=center` are equivalent; the
     matched value is the characters up to the closing quote (for quoted forms)
     or up to the next whitespace or `>` (for the unquoted form). This mirrors
     the quoting latitude the sibling raw-HTML-subset specs accept.
   - **`align=` extraction.** The attribute value is matched
     **case-insensitively** against the three recognized literals `left`,
     `center`, `right` (product invariant 4). Any other value — `justify`,
     `bogus`, empty — is well-formed but unrecognized: the tag is still consumed
     as an align tag, it just yields no alignment (unaligned region), never an
     error. This is distinct from *malformed syntax* (below), where the tag
     falls back to literal text.
   - **`style=` extraction** applies the product-spec `style` micro-grammar
     (invariant 3) to the attribute value: split declarations on `;`; split each
     declaration into `property:value` on the **first** `:`; trim whitespace
     around the declaration, the property, and the value; ignore a trailing or
     empty (`;;`) declaration; match the property name **case-insensitively**
     against `text-align`; ignore declarations whose property isn't `text-align`
     (e.g. `color: red`) without invalidating the rest; when **multiple**
     `text-align` declarations appear, the **last** wins; match the value
     **case-insensitively** against `left`/`center`/`right`. It is a fixed
     literal-value matcher, not a CSS parser — `calc()`, custom properties,
     `!important`, comments, or any unrecognized `text-align` value make that
     declaration contribute no alignment.
   - **`align`-vs-`style` conflict winner: `style` wins** (product invariant 3,
     restated here once as the single source both detectors read). When both
     `align` and a recognized `text-align` `style` declaration are present and
     name different values, the `style` value is used. If `style` is present but
     contributes no recognized `text-align` value (unrecognized value, or no
     `text-align` declaration at all), it does **not** override — `align`'s value
     (if recognized) is used; if neither yields a recognized value the region is
     unaligned.
   - **Unrelated attributes are ignored.** Any attribute other than `align` and
     `style` — `class`, `id`, `onclick`, `data-*`, arbitrary others — is read
     past and discarded; its presence never blocks detection and never falls the
     tag back to literal text. Only `align` and `style` are consulted (see the
     Security section).
   - **Malformed attribute syntax → the whole tag is literal.** If the opening
     tag's attribute list can't be parsed as a well-formed sequence of
     `name`/`name=value` attributes (e.g. an unterminated quote, a stray `=`
     with no value, a `<` inside the attribute region), the detector does **not**
     guess a partial alignment: the tag is not recognized as an align tag, and
     it falls through to ordinary parsing where it renders as literal text —
     consistent with product invariant 7's "degrades deterministically, never to
     undefined behavior" and with the unterminated-region and mixed-same-line
     literal fallbacks already defined above. A recognized tag with an
     *unrecognized value* (well-formed syntax, but `align="justify"`) is the
     distinct, softer case: the tag is still consumed as an align tag but
     contributes no alignment (unaligned region), not literal text.

   Because both detectors funnel through this one contract, a test that fixes
   any single rule here (conflict winner, unrelated-attribute-ignoring,
   case-insensitivity, malformed fallback) fixes it for both the `<div>` group
   case and the `<p>` single-line case at once.

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
`FormattedTextLine` directly — it matches exhaustively on the variant stream
(`tui_markdown.rs:86-140`) — and implements its **own** alignment for tables,
separately from the GUI, in `crates/warp_tui/src/tui_markdown/table.rs`
(`aligned_cell_spans`, `:260`, padding per `TableAlignment`). Block-region
alignment must therefore have an explicit TUI disposition rather than assuming
the GUI `Align` element covers it (it does not — `warpui_core::Align` is not a
TUI concept).

**The TUI reads the region boundaries from the stream it already consumes — no
buffer/SumTree involvement.** Because the parser-to-buffer contract (design
question 1) represents region boundaries as `FormattedTextLine::AlignRegionStart`
/ `AlignRegionEnd` lines *in the `FormattedTextLine` stream*, the TUI receives
them by adding two arms to its existing exhaustive match at
`tui_markdown.rs:86-140` — the same stream position it already reads
`OrderedList`, `LineBreak`, etc. It never touches the buffer's `BufferText`
markers or the `SumTree` dimension; those are the GUI/buffer lowering of the same
boundaries. Both surfaces read the *same parser boundaries*, each in the form its
own pipeline consumes (the TUI the boundary lines, the buffer-backed GUI the
lowered `BufferText` markers).

**Scope decision:** on `AlignRegionStart`, the TUI renders the bracketed interior
lines with **best-effort horizontal positioning within the terminal width**
(compute each line's rendered width, pad leading columns per the alignment,
mirroring how `aligned_cell_spans` pads table cells), and **falls back to
left-aligned** where a line's width can't be determined or exceeds the pane. This
acknowledges the terminal's coarser layout model; it is an explicit scope
decision, not an oversight. (This mirrors bnavetta's #13345 framing that region
state is expressed once, at the parser boundary, so every surface can consult
it — the GUI via the lowered buffer markers, the TUI via the boundary lines in
the stream.)

7. **Feature gating.** No existing flag covers this (`MarkdownTables` is
   table-specific). Recommend a new flag, `FeatureFlag::MarkdownBlockAlign`, and
   gate it **in the parser crate**, at the same layer the detector lives in
   (design question 3), rather than only at the `buffer.rs` call site — the
   spec's own TUI disposition has `tui_markdown.rs` consuming
   `FormattedTextLine` directly, so a gate that only wraps the buffer's
   `from_markdown` selection would leave the TUI ungated and the two surfaces
   would diverge on whether alignment is live.

   The exact mechanism to mirror is `MarkdownTables`'s, verified end to end:
   `crates/editor/src/content/buffer.rs:850-855` doesn't gate table parsing
   itself — it selects between two *public parser-crate entry points*,
   `parse_markdown` vs. `parse_markdown_with_gfm_tables`
   (`crates/markdown_parser/src/markdown_parser.rs:111-117`), both of which
   delegate to one internal function, `parse_markdown_impl(markdown,
   parse_gfm_tables: bool)` (`markdown_parser.rs:119-134`) — the boolean is
   what actually reaches the table detector inside the parser. `buffer.rs` is
   just one of two call sites that make this choice today: `from_ipynb`
   (`buffer.rs:890-891`) checks the *same* `FeatureFlag::MarkdownTables` and
   threads the resulting bool into `ipynb_parser::ipynb_to_formatted_text`,
   which re-exports and re-checks against the same `parse_markdown` /
   `parse_markdown_with_gfm_tables` pair (`crates/ipynb_parser/src/lib.rs:16,
   124`). Every call site reads the same flag and funnels it to the same
   underlying boolean parameter — that's what keeps them from diverging, not
   any single call site being canonical.

   **The master mechanism only composes for one flag, so don't extend it as
   a second boolean entry point.** Today's two public entry points
   (`parse_markdown`, `parse_markdown_with_gfm_tables`) encode a single binary
   choice, and the call sites express it as an either/or —
   `if MarkdownTables { parse_markdown_with_gfm_tables } else { parse_markdown }`
   (`buffer.rs:850-855`). Adding a sibling `parse_markdown_with_block_align`
   the same way would leave the **both-enabled** state (GFM tables *and* block
   alignment on together) unrepresentable: there is no entry point for it and
   no `else` branch that selects it, so a caller with both flags set would have
   to pick one feature and silently drop the other. Two independent boolean
   features need a 2×2 of behaviors, which two mutually-exclusive entry points
   cannot express. So evolve the one shared internal function to take an
   **options struct** instead of accumulating positional booleans:

   ```rust
   #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
   pub struct MarkdownParseOptions {
       pub gfm_tables: bool,
       pub block_align: bool,
   }

   fn parse_markdown_impl(markdown: &str, options: MarkdownParseOptions) -> Result<FormattedText>
   ```

   `parse_markdown_internal` takes the same `options` and hands each field to
   the detector it gates — `options.gfm_tables` to the existing table detector,
   `options.block_align` to the new align-region detector (design question 3).
   All **four** combinations are well-defined by construction, because the two
   fields are independent inputs to independent detectors: neither on (plain
   Markdown), tables only, align only, or both on (tables and align regions
   both detected in the same pass; they don't interact — a table can appear
   inside an aligned region and is detected normally). `#[derive(Default)]`
   makes "neither" the zero value, so the plain path stays a one-liner.

   Keep the public entry points as **thin wrappers** over the one impl, so no
   existing caller signature breaks and the composable state is reachable:

   ```rust
   pub fn parse_markdown(markdown: &str) -> Result<FormattedText> {
       parse_markdown_impl(markdown, MarkdownParseOptions::default())
   }
   pub fn parse_markdown_with_gfm_tables(markdown: &str) -> Result<FormattedText> {
       parse_markdown_impl(markdown, MarkdownParseOptions { gfm_tables: true, ..Default::default() })
   }
   pub fn parse_markdown_with_options(markdown: &str, options: MarkdownParseOptions) -> Result<FormattedText> {
       parse_markdown_impl(markdown, options)
   }
   ```

   The existing two wrappers keep working unchanged (`parse_markdown_with_gfm_tables`
   is now just `gfm_tables: true`), and the new `parse_markdown_with_options`
   is the entry point for callers that populate **both** fields. There is no
   `parse_markdown_with_block_align` sibling — a third boolean wrapper would
   reintroduce the same non-composing pattern this replaces (it still couldn't
   express tables-and-align-together).

   Then have **each** call site that currently chooses between the two entry
   points build a `MarkdownParseOptions` from **both** `FeatureFlag` checks and
   route through `parse_markdown_with_options`:

   ```rust
   let options = MarkdownParseOptions {
       gfm_tables: FeatureFlag::MarkdownTables.is_enabled(),
       block_align: FeatureFlag::MarkdownBlockAlign.is_enabled(),
   };
   let parsed = parse_markdown_with_options(markdown, options)?;
   ```

   The call sites to convert are the same set that reads `MarkdownTables`
   today — `buffer.rs:850-855` (`from_markdown`), `buffer.rs:890-891` /
   `ipynb_parser::ipynb_to_formatted_text` (whose signature widens from a
   `gfm_tables: bool` parameter to a `MarkdownParseOptions` parameter,
   re-checking both flags the same way, `crates/ipynb_parser/src/lib.rs:16,
   124`), and any TUI call site that selects a parse entry point. Each reads
   **both** flags and funnels them to the same `MarkdownParseOptions`, so the
   surfaces stay in lockstep on *both* features — the same anti-divergence
   guarantee the single-boolean threading gives today, now extended to two
   independent flags. Because a detector only runs when its `options` field is
   true, the align-region-boundary variants (`AlignRegionStart`/`AlignRegionEnd`)
   are simply never emitted when `block_align` is false — every downstream
   consumer (the `edit` lowering loop, the TUI renderer, `find.rs`) can assume
   the variants don't exist in that mode without a separate runtime check,
   identical to how `Table` lines don't appear unless `gfm_tables` is on. This
   lets the feature ship dark and be enabled independently of unrelated Markdown
   work, with the GUI and TUI guaranteed to agree because they read the same
   `MarkdownParseOptions` rather than two independently-maintained checks.

## Security

Only `align` and `style="text-align:…"` are read, and only the three recognized
values are consulted; any other `style` property or attribute (`onclick`,
`class`, `id`, arbitrary CSS) is ignored — this is the "unrelated attributes
ignored" clause of the Attribute-matching contract (design question 3), the same
single contract both detector paths use, matching the pattern already
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
- `<p align="right">caption</p>` (single source line: open tag, content, and
  close tag together) → a `Right` aligned region bracketing exactly one
  interior `Line` (invariant 2, design question 3's single-line detection
  path).
- `Note: <p align="center">caption</p>` and `<p align="center">caption</p> —
  see above` (non-whitespace text sharing the line with the tags, before or
  after) → literal fallback: the whole line renders as today (tags as literal
  text via ordinary paragraph/inline parsing), no `AlignRegionStart`/
  `AlignRegionEnd` emitted (design question 3's mixed-same-line rule).
- `style="text-align: center"` → same result as `align="center"` (invariant 3).
- `align` and conflicting `style` both present → `style` wins (invariant 3).
- `style` micro-grammar cases (product invariant 3's subset): mixed case
  property/value (`style="Text-Align: CENTER"`); whitespace around `:`/`;`
  (`style="text-align : right ;"`); multiple declarations with an unrelated
  property ignored (`style="color: red; text-align: center"`); multiple
  `text-align` declarations where the last wins (`style="text-align: left;
  text-align: center"`); unrecognized `text-align` value (`style="text-align:
  justify"`) → unaligned, not an error.
- Unrecognized value (`align="justify"`, `align="bogus"`) → unaligned
  (`Left`/no wrapping), not an error (invariant 4).
- Attribute-matching contract, quoting variants: `align="center"`,
  `align='center'`, and `align=center` (double-quoted, single-quoted, unquoted)
  all resolve to `Center` (Attribute-matching contract, quoted/unquoted values).
- Attribute-matching contract, attribute-name case-insensitivity:
  `<div ALIGN="center">` / `<p Align="right">` detect identically to lowercase
  (Attribute-matching contract, case-insensitive names).
- Attribute-matching contract, unrelated attributes ignored:
  `<div class="hero" id="x" align="center">` still detects a `Center` region —
  the extra attributes neither block detection nor fall the tag to literal text
  (Attribute-matching contract, unrelated attributes ignored).
- Attribute-matching contract, malformed attribute syntax →
  whole tag literal: an unterminated quote (`<div align="center>`) or a stray
  `<` in the attribute region renders the tag as literal text, no
  `AlignRegionStart`/`AlignRegionEnd` emitted — distinct from the softer
  unrecognized-*value* case (`align="justify"`, which is consumed as an align
  tag but yields no alignment). Assert both to pin the boundary between the two
  fallbacks (Attribute-matching contract, malformed syntax rule).
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
  `AlignRegionStart`/`AlignRegionEnd` boundary lines from the `FormattedTextLine`
  stream it already consumes, not a GUI-only path.

### Integration / manual

Per CONTRIBUTING, before/after screenshots of the issue's exact test case
(centered `<div>` hero + centered `<p>` caption), plus a right-aligned variant
exercising the README "anchored project image" pattern from the issue
description (image-as-literal-text is acceptable until #13721 lands, but its
horizontal position must be correct).

## Risks and follow-ups

- **Marker-model blast radius is real but bounded, and shaped differently from
  the rejected wrapper.** The two content-less boundary variants
  (`FormattedTextLine::AlignRegionStart`/`AlignRegionEnd`) *do* add arms to
  exhaustive matches over `FormattedTextLine` — this is honest: the `edit`
  lowering loop (`core.rs:567`), the TUI renderer (`tui_markdown.rs:86-140`), and
  any other exhaustive consumer each gain arms. The difference from the rejected
  wrapper is not arm *count* but arm *cost*: these are marker lines like
  `LineBreak`/`HorizontalRule`, so most consumers handle them with a trivial or
  no-op arm (emit a marker, or ignore) rather than recursing into wrapped
  children, and — decisively — they never produce a `BufferBlockStyle`, so they
  never collide with the "one block style per character" constraint. On the
  buffer side the touched surface is: the `markdown_parser` block detector
  (`markdown_parser.rs`, plus `html_parser.rs` if the paste path is in scope),
  the two lowering arms in `core.rs`'s `edit` loop plus the `buffer.rs`
  serialize path (~`:6026`) that brackets on the way out, the paired top-level
  `BufferText::AlignStart`/`AlignEnd` variants, and the `SumTree`
  summary/dimension plumbing that answers "is this character inside an aligned
  region." Any code that exhaustively matches `BufferText` (e.g. `find.rs:338`,
  which already special-cases `BufferText::BlockMarker`) gains a new-variant arm
  — a bounded, compiler-enforced set.

  For contrast, the **rejected `FormattedTextLine::AlignedBlock(Vec<…>)`
  wrapper** (design question 1) added arms to the same exhaustive matches, but
  each arm was *heavier* — it wrapped a nested `Vec<FormattedTextLine>` every
  consumer had to recurse into and re-flatten — and it still had to become
  *some* `BufferBlockStyle` when lowered, failing bnavetta's composition
  constraint. That enum is referenced across **20 files** in `crates/`; the
  wrapper's recursion cost would have been paid not just in the obvious Markdown
  files (`markdown_parser` `lib.rs` / `markdown_parser.rs` / `html_parser.rs`;
  `editor` `content/{buffer,core,markdown,text}.rs`) but in two non-obvious ones
  worth calling out — the **Jupyter notebook parser**
  (`crates/ipynb_parser/src/lib.rs`) and the **TUI** (`crates/warp_tui/
  src/tui_markdown.rs` and `tui_plan_view.rs`) — plus the GUI element sink
  (`crates/warpui_core/src/elements/gui/formatted_text_element.rs`). The
  substantive reason to reject the wrapper is the composition failure, the
  recursion cost being secondary. The boundary-line design pays a smaller,
  flatter cost and composes.
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
