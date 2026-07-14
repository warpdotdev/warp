# PRODUCT.md — Markdown viewer: raw-HTML `<sub>`/`<sup>` support

Issue: https://github.com/warpdotdev/warp/issues/13734

Split from: #13652 (bulk raw-HTML-subset request). Sibling splits from the same issue:
#13721 (`<img>` sizing), #13725 (anchor links), #13726 (raw HTML tables), #10259
(`<details>`/`<summary>`), #13732 (`<br>`), #13733 (`<kbd>`), #13735 (`align` on
`<p>`/`<div>`), #13736 (`<picture>`/`<source>`).

## Summary

Markdown has no native syntax for subscript or superscript. The only way an author
expresses either is raw HTML — `<sub>2</sub>`, `<sup>1</sup>` — and Warp's Markdown
viewer currently renders both tags and their contents as **literal baseline text**,
tags included (`<sub>2</sub>` shows up as the literal string `<sub>2</sub>`, not even a
plain "2"). This is common enough to be noticeable: subscript for chemical formulas and
variable notation, superscript for footnote markers and exponents.

This spec covers recognizing `<sub>`/`<sup>` as inline phrasing tags and rendering their
contents with a visible vertical offset from the surrounding baseline (and, where
feasible, a reduced font size — the conventional treatment). Unlike the sibling `<table>`
and `<img>` splits, this one turns out to need a genuine **capability that does not exist
in the text-rendering stack today**: everything from the shared style struct down to the
platform text shaper assumes one font size and one baseline per line of text. The tech
spec investigates how deep that gap goes and proposes a phased path, including an MVP that
ships something correct-looking without the deepest layout-engine work.

Figma: none provided.

## Goals / Non-goals

In scope:

- Recognize `<sub>…</sub>` and `<sup>…</sup>` as inline (phrasing) HTML tags, wherever
  inline HTML is currently accepted (paste path, and the Markdown-file viewer once the
  block/inline grammar routes to it — tech spec confirms exact entry points).
- Render `<sub>` content visually **below** the surrounding text baseline, and `<sup>`
  content visually **above** it, offset enough to read unambiguously as sub/superscript
  rather than a font glitch.
- Content inside `<sub>`/`<sup>` keeps its own inline formatting (bold, italic, code,
  links) composing normally with the sub/sup treatment, the same way `<u>` composes with
  other inline styles today.
- Nesting: `<sup>` inside `<sub>` (or vice versa) does not need to compound the offset in
  this slice — the tech spec picks a sane degraded behavior (e.g. innermost wins) and
  documents it, as long as it doesn't panic or produce garbled output.
- Copy/export canonically re-serializes the semantic markup (emits `<sub>`/`<sup>` HTML, or
  an equivalent internal representation) — a user pasting or exporting content with
  sub/superscript should not silently lose that information. This is canonical
  re-serialization, not byte-exact source preservation: the *rendered* glyph size, and the
  original source's exact formatting, are not guaranteed to round-trip unchanged.
- Degrade gracefully: an unterminated `<sub>`/`<sup>`, or one nested pathologically deep,
  renders without panicking — falling back to plain unstyled text for the unparseable
  portion rather than corrupting the rest of the document.

Out of scope (explicit non-goals):

- **True typographic subscript/superscript glyphs** (font-native alternate glyphs, e.g.
  OpenType `subs`/`sups` features) — this spec targets a baseline-shift + optional
  font-scale rendering of the *existing* glyphs, not font-feature substitution.
- Any MathML-style layout (nested fractions, radicals, etc.) — `<sub>`/`<sup>` are
  handled as simple inline phrasing spans, not a math-layout system.
- Changing GFM Markdown syntax to add native sub/sup shorthand (e.g. `H~2~O` /
  `x^2^`) — this spec is scoped to the raw-HTML tags only, matching the issue.
- Compounding nested `<sub><sup>` offsets into a deeper stack (see nesting note above) —
  deferred if it turns out to need more than the MVP data model supports.
- Script execution / event handlers / navigation from `<sub>`/`<sup>` markup (no
  attributes on these tags carry meaning here).

## Behavior

1. `<sub>2</sub>` in Markdown/pasted content renders "2" visually below the surrounding
   text's baseline. `<sup>1</sup>` renders "1" visually above it. Both are legible against
   the surrounding text — not clipped, not overlapping adjacent lines.

2. Water written as `Water is H<sub>2</sub>O` renders as "Water is H₂O" with the "2"
   subscripted, not as literal `H<sub>2</sub>O` text (today's behavior).

3. A footnote-style marker written as `This claim needs a citation<sup>1</sup>.` renders
   the "1" superscripted immediately after "citation", not as literal `<sup>1</sup>` text.

4. Content inside `<sub>`/`<sup>` may itself carry other inline formatting (e.g.
   `<sub>*n*</sub>`) — the italic (or bold/code/link) styling applies together with the
   vertical offset, the same way `<u>` already composes with emphasis today.

5. If a reduced font size for sub/superscript content is feasible at the chosen
   implementation tier (tech spec determines this), it's applied; if not feasible in this
   slice, full-size baseline-shifted text is an acceptable MVP fallback — the vertical
   offset alone is enough to read as sub/superscript, size reduction is a refinement, not
   a launch blocker. The tech spec states explicitly which is shipped.

6. An unterminated `<sub>` or `<sup>` (missing closing tag) falls back to treating the
   opening tag as literal text (matching how other unpaired inline delimiters degrade
   today) rather than swallowing the rest of the document or panicking.

7. `<sup>` nested inside `<sub>` (or vice versa) does not panic or produce garbled/
   overlapping glyphs. The tech spec documents the exact degraded behavior chosen (e.g.
   only the innermost tag's offset applies), since compounding true nested offsets is
   out of scope for this slice.

8. Copy and export of content containing `<sub>`/`<sup>` canonically re-serializes the tag
   semantics into the copied/exported representation — the semantic markup survives a
   round trip through Warp without silently collapsing subscripted or superscripted text
   to plain baseline text, though the re-serialized output is not guaranteed to be
   byte-identical to the original source.

9. Only the `<sub>`/`<sup>` tags themselves carry meaning; any attributes on them
   (`class`, `style`, `id`, event handlers) are ignored, matching how other inline HTML
   tags are handled today.
