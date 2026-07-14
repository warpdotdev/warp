# PRODUCT.md — Markdown viewer: `<a href>`/`<a id>` anchor links

Issue: https://github.com/warpdotdev/warp/issues/13725

Split from: #13652 (bulk raw-HTML-subset request, closed in favor of per-feature issues).
Sibling specs in the same split: `<img>` sizing (#13721), raw HTML tables (#13726,
`specs/GH13652/tables/`), `<details>`/`<summary>` (#10259), `<br>` (#13732), `<kbd>`
(#13733), `<sub>`/`<sup>` (#13734), `align` (#13735), `<picture>`/`<source>` (#13736).

## Summary

Hand-built READMEs and docs commonly pair `<a id="section">` (or `name="section"`) anchor
targets with `[Jump to Section](#section)` or `<a href="#section">` links, either as a
manual table of contents or as inline cross-references. Warp's Markdown viewer supports
neither half of this pattern today: raw `<a href>`/`<a id>` tags render as literal text
(there is no HTML-tag token in the inline parser at all), and even the markdown-native
`[text](#fragment)` form — which **does** parse today as a link whose target is the literal
string `#fragment` — has nothing to resolve that fragment against, because no heading or
anchor in the document carries an id/slug of any kind.

This request is therefore two related but separable capabilities:

1. **Raw HTML anchor tags.** Recognize inline `<a href="…">…</a>` as a hyperlink (reusing
   the existing `Hyperlink` link styling) and `<a id="…">`/`<a name="…">` as a named anchor
   target attached to the surrounding content.
2. **Fragment resolution + scroll-to.** Give every heading an implicit anchor (a slug
   derived from its text, GitHub-style), let an explicit `<a id>` register an anchor too,
   and make any link whose target is `#fragment` — whether it came from `<a href>` or from
   markdown-native `[text](#fragment)` — scroll the viewer to the matching anchor instead
   of falling through to plain-URL handling.

Capability (2) is the harder half and is also what markdown-native links need — a
`[text](#fragment)` link already parses correctly and gets no benefit from HTML anchor
parsing at all. The issue explicitly frames working in-document anchor links as a
prerequisite for any future table-of-contents feature (#13083, #4720).

Figma: none provided.

## Goals / Non-goals

In scope:

- Parse an inline `<a href="…">link text</a>` as a hyperlink with the given text, styled
  and clickable exactly like a markdown `[link text](…)` — including external URLs
  (`https://…`) and in-page fragments (`#target`).
- Parse `<a id="…"></a>` and `<a name="…"></a>` (empty or self-closing, the common
  hand-authored form) as a named anchor target at that point in the document, with no
  visible rendering of its own (matches GitHub/browser behavior — an anchor tag with no
  text renders nothing).
- Give every heading (`#`…`######`) an **implicit** anchor slug derived from its rendered
  text, so `[Jump to Target Section](#target-section)` works against ordinary headings with
  zero authoring effort — the common case the issue's test document exercises.
- Resolve a `#fragment` hyperlink click (from either an `<a href>` tag or a markdown
  `[text](#fragment)` link) against the set of anchors in the current document — explicit
  `<a id>`/`<a name>` targets and implicit heading slugs — and scroll the viewer so the
  target is visible, instead of the current behavior (treated as an opaque URL).
- A `#fragment` link with no matching anchor in the document degrades gracefully: it
  remains a normally-styled, clickable-looking link, but clicking it is a no-op (no
  navigation, no error, no crash) rather than attempting to open `#fragment` as a URL.
- `<a href>` attributes beyond `href` — `title`, `target`, `rel`, `class`, etc. — are
  parsed-but-ignored: they don't break parsing, and they don't do anything (no `target`
  window semantics, no HTML tooltip from `title`).

Out of scope (explicit non-goals):

- **Cross-document/cross-tab fragment links** (e.g. `[text](other-file.md#section)`
  jumping into a different open document or tab). This spec covers same-document
  resolution only; the tech spec should note whether the chosen anchor-index design leaves
  room for that later.
- **Slug-collision policy beyond "first wins."** If two headings produce the same slug
  (e.g. two headings both literally titled "Overview"), only the first is addressable by
  that slug — matching common Markdown-renderer behavior (GitHub disambiguates duplicates
  by appending `-1`, `-2`, …; replicating that exact disambiguation scheme is left to the
  tech spec's judgment, not mandated here).
- **`<a>` tags with both `href` and `id`/`name` on the same element.** The issue's test
  case and the common real-world pattern always use them separately (`<a id>` as a bare
  target, `<a href>` as the link). Supporting both roles on one tag is not required.
- **Any other raw-HTML tag** (`<img>`, `<table>`, `<details>`, etc.) — each has its own
  spec in this split.
- **Editing/authoring affordances** — e.g. no "copy anchor link" UI, no auto-slug preview
  while typing a heading. This is a *rendering/navigation* feature only.
- **URL scheme validation or link-target security beyond what markdown links already do.**
  `<a href>` reuses the exact same `Hyperlink::Url` styling and click path as markdown
  links; it inherits that trust boundary as-is rather than introducing a new one.

## Behavior

1. `<a href="https://warp.dev">Visit Warp</a>` renders as a clickable link reading "Visit
   Warp", visually and behaviorally identical to the markdown link
   `[Visit Warp](https://warp.dev)`.

2. `<a href="#target-section">Jump to Target Section</a>` renders as a clickable link.
   Clicking it scrolls the viewer so the heading (or explicit anchor) matching
   `target-section` is visible. This is the HTML-tag half of the issue's test case.

3. The markdown-native equivalent, `[Jump to Target Section](#target-section)`, gets the
   **same** scroll-to-target click behavior as invariant 2 — it already parses as a link
   today; only fragment *resolution* is new. This is the contrasting case named explicitly
   in the issue ("resolves as a plain URL hyperlink" today).

4. A heading `## Target Section` is addressable by `#target-section` (its GitHub-style
   slug: lowercased, spaces to hyphens, punctuation stripped) with no authoring effort —
   no `<a id>` required. This is what makes invariants 2 and 3 work against the issue's
   test document out of the box.

5. `<a id="target-section"></a>` (or `<a name="target-section"></a>`) placed anywhere in
   the document — most commonly immediately before a heading, as a hand-authored anchor —
   registers `target-section` as a jump target and renders no visible content itself.

6. If both an explicit `<a id="x">` and a heading whose implicit slug is also `x` exist,
   the explicit `<a id>` is the effective target for `#x` (explicit authoring intent wins
   over the derived default) — the tech spec should confirm this is achievable without
   extra complexity; if not, document the fallback order chosen instead.

7. A `#fragment` link that matches nothing in the document remains a normal-looking,
   clickable link. Clicking it does nothing observable (no scroll, no error dialog, no
   attempt to open it as an external URL). This must not panic or freeze the viewer.

8. Non-`href`/`id`/`name` attributes on `<a>` (`title`, `target`, `rel`, `class`, inline
   `style`, …) are accepted without breaking the parse; they have no behavioral effect.

9. `<a href>` supports the same inline content markdown links do at minimum — plain text.
   Bold/italic/code *inside* the anchor text is a nice-to-have the tech spec may size
   separately; if infeasible in this slice, plain-text-only anchor content is an acceptable
   MVP as long as it's called out.

10. Malformed anchor tags (unterminated `<a`, missing closing `</a>`, `href`/`id` with no
    value) degrade to literal text for that tag, without swallowing the rest of the
    paragraph or document and without panicking.

## Suggested phasing

The two capabilities compound in value but are separately shippable:

- **Phase 1:** `<a href>` inline links (parsed via the existing `Hyperlink` link-styling
  machinery) **and** heading auto-anchors + fragment click resolution + scroll-to. This
  alone delivers the issue's headline case — an inline HTML link or a markdown
  `[text](#heading)` link jumping to a heading — and is the highest-value slice because it
  fixes markdown-native fragment links too, not just the new HTML tag.
- **Phase 2:** Arbitrary `<a id>`/`<a name>` targets (anchors not attached to a heading).
  Lower value on its own — most real documents anchor at headings — but completes the
  issue's hand-built-table-of-contents use case for authors who anchor mid-paragraph.

The tech spec should confirm or revise this split based on actual implementation cost.
