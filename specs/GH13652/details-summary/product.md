# PRODUCT.md — Markdown viewer: `<details>/<summary>` collapsible sections

Issue: https://github.com/warpdotdev/warp/issues/13652
Preceded by: `<img>` sizing spec (`specs/GH13652/`, PR #13656)

## Summary

Warp's Markdown viewer drops raw inline HTML, so `<details>/<summary>` collapsible
sections — extremely common in READMEs, changelogs, and issue templates — render as
either literal tag text or dropped content. This spec covers teaching the Markdown
viewer to recognize a `<details>` block with an optional `<summary>`, render the summary
as a clickable disclosure row with a caret, and show/hide the body on click.

This is tier-zero tag #2 of issue #13652 (after `<img>` sizing). It is a **larger and
more architecturally involved** change than `<img>` sizing: unlike an image (a single
leaf block), a `<details>` owns an arbitrary run of body content, and the collapsed
state must persist across re-layout. The tech spec documents both a pragmatic MVP that
reuses existing hidden-line infrastructure and the constraints that MVP imposes.

Figma: none provided.

## Goals / Non-goals

In scope:

- Recognize a block-level `<details> … </details>` region in a Markdown document,
  with an optional leading `<summary> … </summary>`.
- Render the summary as a single clickable disclosure row prefixed with a caret
  (right-pointing when collapsed, down-pointing when expanded). When no `<summary>` is
  present, render a default label (e.g. "Details").
- Render the body content between `</summary>` and `</details>` as normal Markdown
  blocks (paragraphs, lists, code blocks, images, tables, etc.), shown only when the
  section is expanded.
- Toggle expanded/collapsed on clicking the summary row, re-laying-out the document so
  the body appears/disappears.
- Persist the expanded/collapsed state of each `<details>` across re-layout and edits
  within a viewing session (state keyed to the section's document position).
- Default state on open: collapsed, matching the HTML default (a `<details>` without the
  `open` attribute is collapsed). Honor an `open` attribute by defaulting to expanded.
- Degrade gracefully: a malformed or unterminated `<details>` (no closing tag, nested
  in a way the parser can't handle) falls back to rendering its raw text without
  panicking or losing content.

Out of scope (not changed by this spec):

- Inline `<details>` mixed with other text on the same line.
- Nested `<details>` inside a `<details>` body. (Called out as a known hard case in the
  tech spec; the MVP may render an inner `<details>` as literal text rather than a
  second collapsible. This is an explicit limitation, not a silent failure.)
- Arbitrary non-Markdown HTML inside the body beyond what the other tier-zero specs
  cover (`<img>`, and later `<table>`). Body content is parsed as ordinary Markdown
  blocks; unsupported raw HTML inside the body degrades the same way it does elsewhere.
- Editing/authoring affordances (inserting a `<details>` via a toolbar, etc.). This is
  viewer rendering only.
- Animated expand/collapse transitions. Show/hide is acceptable without animation for
  the MVP.
- Remembering expand state across app restarts / document reopen. Session-scoped
  persistence is sufficient.

## Behavior

1. A Markdown document region delimited by `<details>` … `</details>` on their own
   lines renders as a single collapsible section in the viewer: a clickable summary row
   plus a hidden-by-default body.

2. When a `<summary>…</summary>` appears as the first child of the `<details>`, its
   inline content is the label of the disclosure row (rendered with inline Markdown /
   HTML-inline support consistent with the rest of the viewer). When absent, the row
   shows a default label ("Details").

3. The disclosure row is prefixed with a caret affordance that points right (▶) when
   collapsed and down (▼) when expanded, matching the disclosure-triangle convention.

4. Clicking anywhere on the summary row toggles the section between collapsed and
   expanded, and the document re-lays-out so the body content appears (expanded) or is
   removed from view (collapsed). The rest of the document reflows accordingly.

5. When expanded, the body — everything between `</summary>` and `</details>` — renders
   as ordinary Markdown blocks: paragraphs, lists, code blocks, block images (per the
   `<img>` spec), horizontal rules, headings, and (once the tables spec lands) tables.
   Each body block renders identically to how it would render at top level.

6. On first render of a document, each `<details>` is collapsed unless it carries the
   HTML `open` attribute (`<details open>`), in which case it starts expanded
   (invariant matches HTML semantics).

7. A section's expanded/collapsed state persists across re-layouts caused by unrelated
   edits or viewport changes elsewhere in the document. Toggling section A does not
   change the state of section B. State is keyed to the section's position so it
   survives edits above it (state tracks the section, not a raw line number).

8. A `<details>` with no `<summary>` still renders as a collapsible section with the
   default label and its body hidden/shown on toggle (invariant 2 + 4).

9. An empty `<details></details>` (no summary, no body) renders as a collapsible row
   with the default label whose expanded state shows nothing. It must not render a
   zero-height ghost or panic.

10. A `<details>` whose closing `</details>` is missing (unterminated) falls back to
    rendering the raw `<details>`/`<summary>` tag text and the following content as
    ordinary Markdown, rather than swallowing the rest of the document or panicking.
    (The exact fallback boundary is an implementation choice in the tech spec, but the
    document's remaining content must remain visible and correctly rendered.)

11. Attribute handling on `<details>` is limited: only `open` is honored (invariant 6).
    All other attributes are ignored. No attribute value is executed or navigated to.

12. Nested `<details>` inside a `<details>` body is a known limitation: the inner
    `<details>` MAY render as literal text or as a non-interactive block, but MUST NOT
    corrupt the outer section's boundaries or the document below. (This invariant exists
    to bound the MVP honestly; a later iteration may add true nesting.)

13. Copy / export of a document containing a `<details>` preserves the section:
    round-tripping reproduces a `<details>`/`<summary>` region with the same summary and
    body content. (Whether the serialized form preserves the live expanded/collapsed
    state or always emits the source `open`/closed default is an implementation decision
    in the tech spec.)
