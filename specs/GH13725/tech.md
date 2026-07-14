# TECH.md — Markdown viewer: `<a href>`/`<a id>` anchor links

Product spec: `specs/GH13725/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13725
Sibling specs in the same split: `specs/GH13652/tables/` (raw HTML tables), and specs to
follow for `<img>` sizing, `<details>`/`<summary>`, `<br>`, `<kbd>`, `<sub>`/`<sup>`,
`align`, `<picture>`/`<source>`.

## Context

This is two mostly-independent pieces of work bolted together by the issue, and they should
be evaluated separately because their feasibility differs enormously:

- **`<a href>` as a hyperlink tag: small.** Warp's inline parser has no HTML-tag concept at
  all — `InlineToken` (`crates/markdown_parser/src/markdown_parser.rs:1674-1694`) covers
  `Delimiter`, `Text`, `BackslashEscape`, `HtmlEntity`, `CodeSpan`, `AutoLink`, `LinkEnd`,
  `UnderlineEnd`. The **only** literal HTML tag special-cased anywhere in the inline grammar
  is `<u>`/`</u>` (`parse_inline_token_underline_start`/`_end`, :1628-1645), which is handled
  exactly like a markdown delimiter pair (push a `Delimiter`/`UnderlineEnd` token, no
  attribute parsing). `<a href="…">…</a>` is a direct structural analog: a start delimiter
  carrying one piece of data (the `href` value) and a fixed end tag. The existing markdown
  link path, `parse_link` (:1116-1179) + `parse_link_target` (:1183-1271), already builds
  exactly the styling this needs — `styles.hyperlink = Some(Hyperlink::Url(url))`
  (:1149) — so an HTML `<a>` reader can reuse `Hyperlink::Url` as its output type; it only
  needs a new *front door* into that styling, not a new link model.
- **Fragment resolution + scroll-to: medium, and this is where the real work is.** A
  markdown link `[text](#fragment)` **already parses today** — `parse_link_target` has no
  opinion on what a link destination looks like beyond balanced parens/brackets, so
  `#fragment` is accepted as a URL string exactly like `https://…` would be (confirmed at
  `markdown_parser_tests.rs:1646-1653`, "Example 501": `[link](#fragment)` →
  `Hyperlink::Url("#fragment")`). The gap is entirely downstream of parsing: nothing in the
  content model or render/click path treats a `#`-prefixed `Hyperlink::Url` differently from
  an external URL.
  - `FormattedTextHeader` (`crates/markdown_parser/src/lib.rs:303-306`) is
    `{ heading_size: usize, text: FormattedTextInline }` — no id/slug field. No heading
    carries any addressable identity today.
  - `grep -rn "slug\|anchor" crates/markdown_parser/ crates/editor/` (excluding this spec's
    own additions) turns up nothing — there is no slug-generation code anywhere in the repo
    to reuse.
  - The hyperlink click path is `FormattedTextElement::register_default_click_handlers`
    (`crates/warpui_core/src/elements/gui/formatted_text_element.rs:334-365`): for each
    `Hyperlink::Url(url)` found via `line.hyperlinks(false)`, a registered callback receives
    `HyperlinkUrl { url }` on click (:358-359). Today every consumer of this callback treats
    `url` as an opener target (external browser / new tab) — there is no branch anywhere
    that inspects the string for a leading `#` and no code path back into the editor's own
    scroll state from here.
  - **The scroll primitive this needs already exists**, which is the good news:
    `EditorRenderState::request_autoscroll_to_exact_vertical(character_offset: CharOffset,
    pixel_delta: Pixels)` (`crates/editor/src/render/model/mod.rs:3064-3074`) submits
    `LayoutAction::Autoscroll { mode: AutoScrollMode::ScrollToExactVertical { .. } }`, handled
    at :3694-3700+, and there's a simpler `scroll_to(ScrollPositionSnapshot)` /
    `LayoutAction::ScrollTo` (:2951-2952, handled :3145-3148) for a raw scroll-top target.
    Both already work in terms of a **character offset into the document**, which is exactly
    what a resolved anchor needs to produce. The missing piece isn't "how do we scroll" — a
    character-offset-based autoscroll API is already used elsewhere (selection-follow) — it's
    "how do we go from `#target-section` to a character offset."

This framing matters for scoping: the visible, "does it look done" work (parsing `<a href>`
text into a blue underlined link) is the cheap part. The part that makes the issue's test
case actually pass — `[Jump to Target Section](#target-section)` scrolling to the heading
below it — requires building an anchor index and wiring a new click branch, neither of which
exist in any form today.

## Feasibility summary

- **(i) `<a href>` inline parsing → `Hyperlink::Url`: SMALL.** New `InlineToken` variant(s)
  (or extend delimiter handling, mirroring `<u>`) plus an attribute-extraction step for
  `href`. Reuses 100% of the existing link styling/click/render path once the token exists.
- **(ii) `<a id>`/`<a name>` as an invisible anchor token: SMALL–MEDIUM.** Needs a new
  concept — a zero-width marker attached to a document position — since nothing in
  `FormattedTextFragment`/`FormattedTextLine` represents "renders nothing, but is
  addressable here." Smaller than (iii) because it's purely additive (a new fragment/marker
  kind), not a change to an existing shared struct.
  - **The `<a id>` half of this feature genuinely doesn't fit the current content model in
    ANY existing type**, unlike table `<br>` (`specs/GH13652/tables/tech.md` item 1), which
    at least had an existing multi-line cell rendering path to extend. This is closer to net
    new plumbing: an anchor is not text, not a delimiter, not a link — it's an id-to-position
    binding that must survive from parse time through to click-resolution time.
- **(iii) Heading auto-slugs + anchor index + fragment click resolution: MEDIUM–LARGE.**
  Three sub-parts, each real but bounded:
  - Slug generation from heading text (GitHub-style: lowercase, spaces→hyphens, strip
    punctuation, dedupe via `-1`/`-2` suffixes). Pure function, no existing code to build on
    (product non-goal notes dedupe policy is flexible), straightforward to unit test in
    isolation.
  - An **anchor index**: id → position, built once per document (or incrementally per edit)
    from (a) every heading's slug and (b) every `<a id>`/`<a name>` marker found during
    parse. This needs a place to live — likely alongside or inside the existing document/
    buffer model that already tracks headings for other purposes (e.g. whatever powers
    "jump to heading" style navigation, if Warp has one — **needs verification**; if no such
    index exists yet, this is genuinely new state, not a variant of something already
    tracked).
  - Click-time resolution: extend (or add a sibling to)
    `register_default_click_handlers`'s callback so a `Hyperlink::Url` starting with `#` is
    looked up in the anchor index instead of being handed to the URL-open callback, then
    calls `request_autoscroll_to_exact_vertical`/`scroll_to` with the resolved offset. A
    miss (product invariant 7) simply does nothing — no fallback to "open `#fragment` as a
    URL" (that would be actively wrong; today's behavior of doing that is precisely the bug
    being fixed).

This spec recommends implementing **(i) + (iii)** as phase 1 (matches the product spec's
phasing — this is the slice that fixes the issue's headline test case and also repairs
markdown-native `[text](#heading)` links, which get zero benefit from (ii) alone), and
**(ii)** as phase 2.

## Proposed changes

### 1. `<a href="…">text</a>` inline token (phase 1)

Add HTML-anchor delimiter tokens to `InlineToken`
(`crates/markdown_parser/src/markdown_parser.rs:1674-1694`), following the `<u>` precedent
exactly in shape but carrying data:

- A start token that captures the `href` attribute value — unlike `<u>`'s zero-data
  `Delimiter { kind: UnderlineStart, count: 1 }`, this needs the URL string threaded through
  to close-time, so it likely needs its own variant rather than reusing `Delimiter` verbatim
  (e.g. `InlineToken::HtmlAnchorStart(String)` for the href, paired with a `HtmlAnchorEnd`
  token on `</a>`), or the URL could live in a small ad hoc attribute parser called at
  `<a` and stashed on the delimiter-stack entry (mirroring how `parse_link` stashes
  `link_start.node_index` today at :1141).
- Closing `</a>` applies `styles.hyperlink = Some(Hyperlink::Url(href))` to the fragments
  between start and end — the exact same `backtrack_styles` call `parse_link` makes at
  :1148-1150, just triggered by `</a>` instead of `]`+`(url)`.
- A minimal attribute parser for the opening tag: extract `href="…"` (single/double-quoted),
  tolerate and discard any other attributes (`title`, `target`, `rel`, `class`, …) per
  product invariant 8 — this can be a small nom combinator scanning `key="value"` pairs
  without needing a general HTML tokenizer (the existing paste-path parser in
  `html_parser.rs` already depends on `html5ever`, but pulling that into the inline-token
  grammar for a single-tag case is likely overkill; a purpose-built attribute scanner is
  more consistent with how `<u>` is handled today — call this choice out for maintainer
  review, since `html5ever` is an available and more robust alternative if the ad hoc parser
  proves fragile against real-world `<a>` markup).
- Malformed input (unterminated `<a`, `href` with no value, no closing `</a>`) falls back to
  literal text for the tag, matching how `parse_link` falls back to a literal `]` on failure
  (:1170-1177) — product invariant 10.

### 2. Heading slugs (phase 1)

Add a `slug: String` (or `Option<String>` if collision/empty-text edge cases need to opt
out) field to `FormattedTextHeader` (`crates/markdown_parser/src/lib.rs:303-306`), computed
at parse time from the heading's rendered text. This is a **shared-struct change** like the
table `<br>` cell-type change in the sibling spec — audit callers of
`FormattedTextHeader { .. }` construction and pattern matches before landing (`grep -rn
"FormattedTextHeader" crates/`) to size the ripple; expect this to be small since the field
is purely additive.

Slug algorithm (GitHub-compatible, since that's the ecosystem convention the product spec
points to): lowercase, strip characters outside `[a-z0-9 -]`, collapse/trim spaces, replace
spaces with `-`. Deduplicate across the document by appending `-1`, `-2`, … to repeats,
processed in document order — this requires slug generation to happen as a document-wide
pass (or with access to prior headings' slugs), not purely per-heading, so it likely belongs
as a post-process over the parsed `FormattedText.lines` rather than inline in the heading
parser itself.

### 3. Anchor index (phase 1 for headings, phase 2 extends it for `<a id>`)

Build a document-scoped `HashMap<String, CharOffset>` (id → character offset of the target)
by walking `FormattedText.lines` after parsing:

- Every `FormattedTextLine::Header(h)` contributes `h.slug → <offset of this line>`.
- (Phase 2) every `<a id>`/`<a name>` marker contributes `id → <offset of the marker>`.
- Product invariant 6 (explicit `<a id>` wins over a same-named implicit heading slug):
  since phase 2 markers are indexed after or alongside headings, insert order (or an
  explicit "explicit beats implicit" rule in the insert step) resolves the collision — flag
  for implementation whether `HashMap::insert` overwrite order alone is sufficient or needs
  an explicit priority check.

Where this index lives and how it's invalidated on edit is the main open question — needs
verification against however Warp already tracks per-document derived state (if anything
comparable exists for, say, syntax-highlighting spans or the table offset maps referenced in
the tables spec, follow that pattern; if nothing comparable exists, this is new
per-document cached state that must be recomputed on edit, which the tech implementer should
scope against the render/relayout lifecycle in `render/model/mod.rs`).

### 4. Fragment-aware click resolution (phase 1)

`register_default_click_handlers` (`formatted_text_element.rs:334-365`) currently maps every
`Hyperlink::Url(url)` to the same `HyperlinkUrl { url }` callback unconditionally (:358-359).
Add a branch: if `url` starts with `#`, resolve `url[1..]` against the anchor index instead
of invoking the URL-open callback.

- **Hit:** call `request_autoscroll_to_exact_vertical(character_offset, Pixels::zero())` (or
  `scroll_to` with a `ScrollPositionSnapshot` built from the offset, whichever the
  surrounding `EventContext`/`AppContext` at the click site can most directly construct —
  needs verification of which of the two APIs is reachable from
  `formatted_text_element.rs`'s click callback without new plumbing).
- **Miss:** do nothing (product invariant 7) — explicitly *not* falling through to the
  URL-open callback with `#fragment` as a literal URL, which is today's bug.
- This requires `register_default_click_handlers` (or its caller in the Markdown-viewer
  wiring, not yet located — **needs verification** of exactly where the viewer instantiates
  `FormattedTextElement` and supplies the click callback) to have access to the anchor index
  from (3) at click time.

### 5. `<a id>`/`<a name>` anchor markers (phase 2)

Represent a bare anchor as a new zero-width construct — the cleanest option is a
`FormattedTextFragment` with empty `text` and a new style/marker field (e.g.
`styles.anchor_id: Option<String>`), since fragments are already the unit `hyperlinks()`
and similar traversal helpers walk over; a marker-as-empty-fragment reuses that traversal
for free. An alternative is a dedicated `FormattedTextLine` variant, but that's a heavier
change (every line-level consumer would need a new match arm) for something that's
conceptually inline, not block-level — recommend the fragment-marker approach unless review
surfaces a reason `FormattedTextLine` fits better.

`<a id="x">visible text</a>` (both an id and content in one tag) is out of scope per product
non-goal — the phase-2 reader only needs to handle the empty/self-closing form
(`<a id="x"></a>` or `<a id="x" />`), simplifying the parser considerably (no need to also
apply link/text styling in this path).

### 6. Feature gating

Recommend a **new** feature flag (e.g. `FeatureFlag::MarkdownAnchorLinks`) rather than
riding an existing one — unlike the tables spec, there's no existing "structural HTML"
flag this naturally extends, and gating separately lets phase 1 (headings + `<a href>`) ship
independently of phase 2 (`<a id>`) if their cost estimates diverge during implementation.

### 7. Security

`<a href>` reuses `Hyperlink::Url` verbatim — no new trust boundary, no script/event-handler
attributes are read (only `href`; all others parsed-but-discarded per product invariant 8).
Fragment resolution never leaves the document (no network, no file access) — a `#fragment`
click either scrolls within the current buffer or is a no-op. `<a id>`/`<a name>` values are
used only as HashMap keys for in-document lookup, never interpolated into a URL, path, or
shell context.

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<a href="https://warp.dev">Visit Warp</a>` → `Hyperlink::Url` fragment identical in
  shape to the equivalent markdown link (invariant 1).
- `<a href="#target">Jump</a>` → `Hyperlink::Url("#target")` fragment (invariant 2) —
  parsing only; resolution is tested separately below.
- Attributes beyond `href` (`title`, `target="_blank"`, `class="x"`) parsed-but-ignored, no
  effect on output (invariant 8).
- Unterminated `<a href="…">` / missing closing `</a>` → literal text fallback, rest of
  paragraph intact (invariant 10).
- Heading slug generation: plain text, text with punctuation/mixed case, duplicate headings
  → `-1`/`-2` suffixes (invariant 4, product non-goal on exact dedupe scheme — assert the
  chosen behavior, not GitHub's exact algorithm unless matched intentionally).
- (Phase 2) `<a id="x"></a>` / `<a name="x"></a>` → zero-width anchor marker, no visible
  text emitted (invariant 5).
- (Phase 2) `<a id="x">text</a>` (both id and content on one tag) — confirm documented
  behavior (out of scope; assert it doesn't panic, even if unspecified which role wins).

### Anchor index / resolution tests (`crates/editor/` — exact module TBD per item 3)

- Document with a heading and a `[text](#slug)` link → click resolves to the heading's
  character offset (invariants 2, 3, 4).
- `<a href="#slug">` and markdown `[text](#slug)` targeting the same heading → identical
  resolved offset (invariant 3 — the two syntaxes must be equivalent post-resolution).
- `#fragment` with no matching anchor → click is a no-op, no panic, link still renders
  normally (invariant 7).
- (Phase 2) Explicit `<a id="x">` colocated with a heading whose implicit slug is also `x`
  → explicit anchor wins per invariant 6, or documented fallback if not achievable.

### Integration / manual

Per CONTRIBUTING, before/after screenshots plus a short recording reproducing the issue's
motivating test document verbatim: the raw-HTML `<a href="#target-section">` jump, the
markdown-native `[Jump to Target Section](#target-section)` jump (contrast case — today
resolves as a plain URL), the external `<a href="https://warp.dev">` link, and the `<a
id="target-section"></a>` marker preceding the heading. Confirm scroll lands the heading at
or near the top of the viewport (exact positioning behavior — top-align vs. some offset — is
an implementation choice the manual pass should sanity-check against `request_autoscroll_to_
exact_vertical`'s existing `pixel_delta` semantics).

## Risks and follow-ups

- **The anchor index's storage/invalidation lifecycle is the single biggest unknown.**
  Everything else in this spec (slug generation, click resolution, the `<a id>` marker
  itself) is bounded, ordinary parser/render work. Where a per-document id→offset map lives,
  how it's kept in sync with edits, and whether an existing analogous cache already exists to
  extend — none of this is verified yet and should be the first thing implementation
  confirms, since it could shift (iii) from MEDIUM to LARGE if no such lifecycle hook exists.
- **`<a href>`'s attribute parser (item 1) needs a maintainer call between a purpose-built
  scanner and reusing `html5ever`.** The purpose-built path matches the `<u>` precedent's
  spirit (minimal, inline-grammar-native) but is more exposed to malformed real-world HTML
  than a real parser; `html5ever` is already a dependency of the crate (via
  `html_parser.rs`) so there's no new-dependency cost either way.
- **Interaction with the HTML-table spec:** an `<a href>`/`<a id>` inside a table cell should
  work automatically once cell inline content is parsed via the same `parse_phrasing_content`
  path the tables spec already plans to reuse — verify once both land, no explicit design
  change anticipated here.
- **Cross-document fragment links (explicit non-goal) are the natural next step** once
  same-document resolution ships, and the anchor-index design should be sanity-checked
  against not painting that in a corner (e.g. keying the index by document id in addition to
  fragment id would future-proof it cheaply) — not required for this slice, but worth a
  design glance before implementation locks in the index's shape.
