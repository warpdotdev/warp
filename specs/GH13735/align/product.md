# PRODUCT.md — Markdown viewer: `align` attribute on `<p>`/`<div>` blocks

Issue: https://github.com/warpdotdev/warp/issues/13735

Split from: #13652 (bulk raw-HTML-subset request). Sibling splits: #13721 (`<img>`
sizing), #13725 (anchor links), #13726 (raw HTML tables), #10259 (`<details>`/
`<summary>`), #13732 (`<br>`), #13733 (`<kbd>`), #13734 (`<sub>`/`<sup>`), #13736
(`<picture>`/`<source>`).

## Summary

Markdown itself has no syntax for horizontal alignment outside GFM table columns.
Authors who want a centered README "hero" (logo, title, badge row) or a
right-anchored project image reach for raw HTML: `<div align="center">…</div>` or
`<p align="right">…</p>`. GitHub, and every other major renderer, honors the
`align` attribute on these two block tags. Warp's Markdown viewer currently drops
it silently — the block renders, but always left-aligned, with the attribute
ignored.

This spec covers recognizing `align="left|center|right"` (and the `text-align`
CSS-style equivalent) on a `<div>` or `<p>` block and rendering that block's
content horizontally aligned within the viewer pane, while everything nested
inside continues to render as normal Markdown (headings, images, text, badge
rows, etc.), exactly as it does today when the attribute is absent.

Figma: none provided.

## Goals / Non-goals

In scope:

- Recognize `align="center"`, `align="right"`, and `align="left"` (and
  `style="text-align: …"` with the same three values) on a `<div>` or `<p>`
  block that appears on its own line(s) in a Markdown document.
- Render the block's content horizontally aligned per the attribute: each line
  the block produces is centered / right-aligned / left-aligned within the
  viewer's content width, matching the common README "hero header" and
  "right-anchored image" patterns.
- Continue to render everything nested inside the block as normal Markdown —
  headings render as headings, inline images render as images (subject to
  `<img>` support landing separately, #13721), bold/links/etc. render normally.
  Alignment is purely a horizontal-position change; it must not suppress or
  alter nested block semantics.
- Support a `<div align="…">` that wraps **multiple** block-level children
  (e.g. a heading, a blank line, an image) as a single aligned group — the
  common "hero" shape from the issue's test case.
- Support a single-paragraph `<p align="…">` as the other common shape (a
  centered caption line).
- Default (no `align`/`text-align`, or an unrecognized value) renders exactly as
  today: left-aligned, attribute ignored.

Out of scope (explicit non-goals for this slice):

- **Float / inline text-wrap.** GitHub's `<img align="right">` pattern — where
  prose interleaves alongside a right-anchored image instead of stacking below
  it — requires CSS float-equivalent layout, which Warp's block-stacked render
  model does not have. This spec only guarantees the image (or block) itself
  is horizontally positioned at the right edge; it stacks above/below
  surrounding prose rather than wrapping beside it. True float/wrap is future
  work, tracked against `<img align>` in #13721 once block alignment lands.
- Replicating GitHub's own inline-flow quirk, where `<div align="center">` can
  break surrounding inline flow while `<p align="center">` does not — this is
  a rendering artifact of GitHub's specific engine, not a behavior to match.
  Warp should render both tags' alignment consistently and predictably.
- Alignment values beyond `left`/`center`/`right` (e.g. `justify`) — not
  requested and not part of the common README pattern.
- Alignment on any tag other than `<div>`/`<p>` (e.g. `<table>`, `<h1 align>`)
  — out of scope for this slice; may be considered later if requested.
- Vertical alignment.
- Nesting an aligned block inside another aligned block (e.g.
  `<div align="center"><p align="right">…</p></div>`) — the tech spec should
  define a safe, non-panicking fallback (e.g. innermost wins, or outermost
  wins) but exotic nesting is not a design goal to optimize for.
- **Full GUI/TUI render parity.** Alignment is stored once in the content model,
  but the terminal (TUI) surface renders it with best-effort horizontal
  positioning within terminal width and falls back to left-aligned where the
  region can't be laid out (e.g. it exceeds the pane) — see the tech spec's TUI
  surface disposition. Pixel-for-pixel equivalence between the GUI and the TUI
  is an explicit non-goal; both surfaces read the same stored alignment, but the
  terminal's coarser layout model means its result is an approximation.

## Behavior

1. A `<div align="center">` / `<div align="right">` / `<div align="left">`
   block, delimited by its own opening/closing tags with block content in
   between (matching the issue's test-case shape — blank line after the
   opening tag, one or more Markdown blocks, blank line before the closing
   tag), renders all of its contained blocks horizontally aligned per the
   attribute. Each contained block continues to render with its normal
   semantics (a `#` heading inside still renders as a heading, just
   horizontally repositioned).

2. A `<p align="center">…</p>` (single paragraph, inline content only) renders
   that paragraph's line(s) horizontally aligned per the attribute. This is
   the common "centered caption" pattern.

3. `text-align: center|right|left` expressed via `style="…"` on either tag is
   honored identically to the `align` attribute. If both are present and
   conflict, `style` wins (matching CSS cascade expectations, since `style` is
   the more specific/modern mechanism).

   Warp does not embed a CSS engine, so only a small literal subset of
   `style` is recognized — exactly-N-literals matching, the same style used
   for other raw-HTML-attribute subsets in this split, not general CSS
   parsing:

   - The `style` value is split into declarations on `;`. Each declaration is
     split into `property:value` on the **first** `:`. Leading/trailing
     whitespace around the whole declaration, around the property, and around
     the value is ignored (`style="text-align : center ;"` is equivalent to
     `style="text-align:center"`). A trailing `;` (or an empty declaration
     from consecutive `;;`) is ignored, not an error.
   - The property name is matched **case-insensitively** against `text-align`
     (`Text-Align`, `TEXT-ALIGN`, etc. all match). Declarations whose property
     isn't `text-align` (any other CSS property, e.g. `color: red`) are
     **ignored** — their presence doesn't invalidate the rest of the `style`
     value or fall back to literal text.
   - If **multiple** `text-align` declarations appear in the same `style`
     value (e.g. `style="text-align: left; text-align: center"`), the
     **last** one wins, matching CSS cascade order within a single
     declaration block (later declarations override earlier ones for the
     same property).
   - The value is matched **case-insensitively** against the three
     recognized literals: `left`, `center`, `right`. No unit conversion, no
     shorthand (`text-align: initial`/`inherit`/`justify`/anything else) is
     recognized.
   - An unrecognized `text-align` value, or a `style` attribute with no
     `text-align` declaration at all, is treated as if `style` were absent
     for alignment purposes — falls through to invariant 4 (unaligned,
     normal Markdown semantics), consistent with "never an error or panic."
   - This is a fixed literal-value matcher, not a CSS parser: no
     `calc()`, custom properties, `!important`, or comments (`/* … */`)
     inside `style` are supported. Any of these appearing in the
     `text-align` declaration's value makes that declaration unrecognized
     (falls through per the point above), rather than being partially
     interpreted.

4. An unrecognized or missing alignment value renders identically to today:
   left-aligned, attribute ignored — never an error or panic.

5. Alignment affects horizontal position only. It never changes what a nested
   block renders as (a heading stays a heading, an image stays an image once
   `<img>` support exists), never affects vertical stacking order, and never
   truncates or reflows content differently than the equivalent unaligned
   block would.

6. Copy / export of a document containing an aligned block preserves the
   alignment: round-tripping through Warp (copy out, paste back in) does not
   silently drop `align`/`text-align`. Preservation is *semantic*, not
   byte-exact — consistent with the maintainer's ruling on the sibling
   `<details>` spec (#13345) that "Warp's rich-text pipeline doesn't attempt to
   guarantee exact preservation of the source Markdown… it's fine to continue
   that here." The tech spec defines the exact serialization.

7. Malformed input degrades deterministically, never to undefined behavior:
   an unterminated `<div>`/`<p>` (no matching close tag found) renders as
   literal text — the opening tag and everything after it up to the next
   recognized block boundary is treated as unparsed source. An
   `align`/`text-align` value the parser can't safely group (e.g. content
   the block detector rejects as a groupable unit) renders as the unaligned
   equivalent — the content still renders with normal Markdown semantics,
   just without the alignment applied. Neither case ever panics or corrupts
   the rest of the document.

## Test case (from the issue, used as the acceptance check)

```markdown
<div align="center">

# Centered Title

<img src="https://placehold.co/300x150/png" alt="Centered image">

</div>

<p align="center">A centered caption line.</p>
```

Expected: the heading and image line render centered as a group; the caption
paragraph renders centered on its own. (The nested `<img>` itself renders as an
image only once #13721 lands; until then it may still render as literal
text/unsupported content, but that text should itself be horizontally
positioned per the enclosing alignment — the two features are independent and
should not block each other.)
