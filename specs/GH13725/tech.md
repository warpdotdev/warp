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
  all — `InlineToken` (`crates/markdown_parser/src/markdown_parser.rs:1684`) covers
  `Delimiter`, `Text`, `BackslashEscape`, `HtmlEntity`, `CodeSpan`, `AutoLink`, `LinkEnd`,
  `UnderlineEnd`. The **only** literal HTML tag special-cased anywhere in the inline grammar
  is `<u>`/`</u>` (`parse_inline_token_underline_start`/`_end`, :1635/:1648), which is handled
  exactly like a markdown delimiter pair (push a `Delimiter`/`UnderlineEnd` token, no
  attribute parsing). `<a href="…">…</a>` is a direct structural analog: a start delimiter
  carrying one piece of data (the `href` value) and a fixed end tag. The existing markdown
  link path, `parse_link` (:1125) + `parse_link_target` (:1192), already builds
  exactly the styling this needs — `styles.hyperlink = Some(Hyperlink::Url(url))`
  (:1158) — so an HTML `<a>` reader can reuse `Hyperlink::Url` as its output type; it only
  needs a new *front door* into that styling, not a new link model.
- **Fragment resolution + scroll-to: small, because the scaffolding already exists.** A
  markdown link `[text](#fragment)` **already parses today** — `parse_link_target` has no
  opinion on what a link destination looks like beyond balanced parens/brackets, so
  `#fragment` is accepted as a URL string exactly like `https://…` would be (confirmed at
  `markdown_parser_tests.rs:1644-1647`, "Example 501": `[link](#fragment)` →
  `Hyperlink::Url("#fragment")`). And the click path **already** treats a `#`-prefixed
  `Hyperlink::Url` differently from an external URL: `maybe_open_url`
  (`app/src/notebooks/editor/view.rs:1971`) routes it to `scroll_to_matching_header` rather
  than the URL opener. The gap is narrow and specific: the matcher compares the fragment
  against the heading's exact lowercased text instead of a GitHub-style slug, so a hyphenated
  fragment misses a spaced heading. Fixing that one comparison is the core of the resolution
  work — no new content-model field and no new click branch are required for headings.
  - `FormattedTextHeader` (`crates/markdown_parser/src/lib.rs:303`) is
    `{ heading_size: usize, text: FormattedTextInline }` — no id/slug field. But this no
    longer matters for phase 1: heading matching happens in the editor at click time via
    `find_matching_header`, which re-reads each heading's text out of the live buffer
    (`content.text_in_range(...)`) on every click — it never consulted a parse-time slug
    field, so none needs to be added to `FormattedTextHeader`.
  - `grep -rn "slug" crates/markdown_parser/ crates/editor/` turns up no slug-*generation*
    helper — the normalization function has to be written — but the *matching loop* it plugs
    into already exists (`find_matching_header`, `app/src/notebooks/editor/model.rs:1351`),
    so this is a small localized change, not new plumbing.
  - **A `#`-fragment click path already exists in the Markdown viewer** — this is the key
    fact that reshapes the scoping below. The viewer's click entry point is
    `NotebookEditorView::maybe_open_url`
    (`app/src/notebooks/editor/view.rs:1955`), and it **already branches on a leading `#`**:
    at :1971-1983, `if url.starts_with('#')` it calls
    `model.scroll_to_matching_header(&url, ctx)` and returns early on a hit, only falling
    through to ordinary URL-open handling on a miss. So the fragment-scroll wiring the earlier
    draft of this spec treated as net-new does not need to be built — it is live on master.
    (The general `FormattedTextElement::register_default_click_handlers` helper at
    `crates/warpui_core/src/elements/gui/formatted_text_element.rs:336` is used by many other
    surfaces — settings pages, banners, modals, the changelog, AI views — but **not** by the
    notebook Markdown viewer, and none of those callers has a `#`-fragment branch. It is not
    the fix site; do not target it.)
  - **The gap is the matching rule, not the scroll or the click branch.**
    `scroll_to_matching_header` (`app/src/notebooks/editor/model.rs:1335`) delegates to
    `find_matching_header` (:1351), which walks `content.outline_blocks()`, filters to
    `BlockType::Text(BufferBlockStyle::Header { .. })`, and compares the fragment against
    `heading.trim().to_lowercase()` (:1374) — i.e. the **exact lowercased heading text**, not
    a GitHub-style slug. So `#target-section` (hyphenated) misses a heading titled
    "Target Section" (spaced), which is precisely the issue's failing case. It does already
    strip the `#` prefix and `urlencoding::decode` the fragment (:1352-1356), so URL-escaped
    fragments are handled; only the text-vs-slug comparison is wrong.
  - **The scroll primitive is already wired end-to-end.** On a match,
    `scroll_to_matching_header` calls
    `render_state.request_autoscroll_to(AutoScrollMode::PositionOffsetInViewportCenter(range.start))`
    (`app/src/notebooks/editor/model.rs:1346`), backed by
    `EditorRenderState::request_autoscroll_to` (`crates/editor/src/render/model/mod.rs:3054`,
    with `PositionOffsetInViewportCenter` handled at :3661/:3712). Nothing new is needed on the
    scroll side — a resolved heading range already scrolls today.

This framing matters for scoping: the visible, "does it look done" work (parsing `<a href>`
text into a blue underlined link) is one piece. The part that makes the issue's headline test
case pass — `[Jump to Target Section](#target-section)` scrolling to the heading below it — is
**far smaller than the earlier draft claimed**: the click branch, the scroll call, and the
per-click heading iteration all already exist in `find_matching_header`. What's missing is slug
normalization inside that one function so a hyphenated fragment matches a spaced heading.

## Feasibility summary

- **(i) `<a href>` inline parsing → `Hyperlink::Url`: SMALL.** New `InlineToken` variant(s)
  (or extend delimiter handling, mirroring `<u>`) plus an attribute-extraction step for
  `href`. Reuses 100% of the existing link styling/click/render path once the token exists.
- **(ii) `<a id>`/`<a name>` anchor targets: SMALL — delivered in this PR.** The zero-width
  marker concept described in the original draft below turned out to be unnecessary.
  Characterization of the phase-1 parser (`markdown_parser::parse_inline_token_html_anchor_start`,
  `crates/markdown_parser/src/markdown_parser.rs:1750`) confirmed it only recognizes `<a href>`
  as a delimiter pair — it *requires* an `href` attribute to match at all (:1776-1779: no `href`
  → `Err`), so a bare `<a id="x">`/`<a name="x">` with no `href` never becomes that token. It
  falls through the inline `alt` chain to plain `text`, and the tag's raw markup — including the
  id/name value — survives **verbatim, as visible literal text**, in the buffer. This is already
  a committed, passing assertion: `markdown_parser_tests::test_parse_html_anchor_unterminated_falls_back_to_text`
  asserts `<a id="x"></a>` parses to `FormattedTextFragment::plain_text("<a id=\"x\"></a>")`.
  Because the id survives verbatim in the buffer, resolution doesn't need a new content-model
  field or a parse-time anchor concept at all — it reuses the exact same live-text-walk pattern
  `find_matching_header` already established for headings (item iii), just scanning for anchor
  tags in the whole-buffer text instead of outline-block heading text. See item 5 for the
  mechanism. (The original draft below, describing a `styles.anchor_id` fragment marker, is kept
  for context but was **not** implemented — it solved a problem that characterization showed
  doesn't exist for the empty/self-closing anchor form this feature scopes.)

  **Known gap, shipped as-is, deferred separately:** because the tag survives as literal text,
  it also *renders* as literal text — `<a id="x"></a>` is visible inline, unlike GitHub's
  content-less-anchor-renders-nothing behavior. Investigating a hiding mechanism (a design
  requiring the tag to become first-class block metadata that still re-serializes through
  `to_markdown` on save, or it's silently deleted on the next edit) surfaced a genuine
  content-model migration — every `BufferBlockStyle` variant and/or every `BufferText::BlockMarker`
  call site (roughly 70-130 sites across `core.rs`, `edit.rs`, `buffer.rs`, `markdown.rs`,
  `render/`, and hand-built test fixtures) would need touching, with no clearly-best
  representation among the candidates. That's out of scope for this PR and tracked as
  [#13982](https://github.com/warpdotdev/warp/issues/13982) — a design-discussion ticket, not a
  quick follow-up patch, deliberately left unbuilt until maintainers weigh in on the
  representation.
- **(iii) Heading slug matching + fragment click resolution: SMALL.** The click branch, the
  scroll call, and the per-click heading walk all already exist in `find_matching_header`
  (`app/src/notebooks/editor/model.rs:1351`). The only work is:
  - Slug normalization (GitHub-style: lowercase, spaces→hyphens, strip punctuation). This is
    a pure function with no existing code to build on, straightforward to unit test in
    isolation. It gets applied inside `find_matching_header`: normalize the incoming fragment
    and normalize each heading's text with the **same** function, then compare — replacing the
    current `heading.trim().to_lowercase() == target` check (:1374). Both sides run through
    one normalizer, so a hyphenated fragment matches a spaced heading.
  - **No new anchor index.** `find_matching_header` already iterates `content.outline_blocks()`
    and reads each heading's text live from the buffer on every click, so there is no
    id→offset map to build, no place for it to live, and no cache-invalidation problem — the
    function recomputes against the current buffer each time it runs. (Dedupe of collision
    slugs, if wanted, is likewise "first match wins" as a natural consequence of the loop
    returning on the first hit — no separate `-1`/`-2` bookkeeping is required for phase 1;
    the product spec's non-goal already accepts first-wins.)
  - Miss behavior (product invariant 7) is already correct: `find_matching_header` returns
    `None` → `scroll_to_matching_header` returns `false` → `maybe_open_url` falls through.
    The one nuance to preserve: on a miss today, `maybe_open_url` still hands `#fragment` to
    the URL opener (view.rs:1984+), which for an in-document fragment is a no-op in practice
    but should be confirmed not to surface a broken-link tooltip; if it does, the miss path
    should early-return instead of falling through. Flag this for the implementer to verify
    against invariant 7's "no error dialog" clause.

- **(iv) Cross-document fragment navigation (`other-file.md#section`): MEDIUM — delivered in
  this PR.** The file-open, tab-focus, and dedup were *supposed* to be free (a fragment-less
  relative link opens the target in the Markdown viewer today, `resolve_and_open` →
  `OpenFileNotebook` → `open_file_notebook`, `app/src/workspace/view.rs:8470`) — but that
  baseline turned out to be broken in three ways that had to be repaired first (item 6b). The
  feature work itself is bounded: split the `#section` off before file resolution, thread it to
  the destination pane (mirroring the code editor's existing `line_and_column` plumbing), and
  drain it as a **deferred scroll** once the new document parses — the one genuinely new
  element, since there is no on-load hook to reuse. Full mechanism in **item 6a**; resolution
  repairs in **item 6b**.

This spec recommends implementing **(i) + (iii)** as phase 1 (matches the product spec's
phasing — this is the slice that fixes the issue's headline test case and also repairs
markdown-native `[text](#heading)` links, which get zero benefit from (ii) alone),
and **(iv)** delivered together with phase 1 in the same PR (see the amendment note below).
**(ii)** — arbitrary `<a id>`/`<a name>` markers — is deferred to a follow-up.

> **Amendment (cross-document delivered with phase 1).** Item (iv) was originally sequenced
> as "phase 3, later." Implementation moved it into this PR because verifying the
> fragment-less baseline it was supposed to build on ("a bare relative link already opens
> the target today") turned out to be false in three ways — a bare `README.md` link could
> silently open the browser, and even a `./file.md#frag` link no-op'd. Those three
> resolution defects (documented in the new **Resolution repairs** section below) had to be
> fixed for the cross-document feature to work at all, so the feature and its repairs ship
> together. `<a id>` markers (item ii) remain the follow-up.

## Proposed changes

### 1. `<a href="…">text</a>` inline token (phase 1)

Add HTML-anchor delimiter tokens to `InlineToken`
(`crates/markdown_parser/src/markdown_parser.rs:1684`), following the `<u>` precedent
exactly in shape but carrying data:

- A start token that captures the `href` attribute value — unlike `<u>`'s zero-data
  `Delimiter { kind: UnderlineStart, count: 1 }`, this needs the URL string threaded through
  to close-time, so it likely needs its own variant rather than reusing `Delimiter` verbatim
  (e.g. `InlineToken::HtmlAnchorStart(String)` for the href, paired with a `HtmlAnchorEnd`
  token on `</a>`), or the URL could live in a small ad hoc attribute parser called at
  `<a` and stashed on the delimiter-stack entry (mirroring how `parse_link` stashes
  `link_start.node_index` today at :1153).
- Closing `</a>` applies `styles.hyperlink = Some(Hyperlink::Url(href))` to the fragments
  between start and end — the exact same `backtrack_styles` call `parse_link` makes at
  :1157-1158, just triggered by `</a>` instead of `]`+`(url)`.
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

### 2. Heading slug normalization (phase 1)

No parse-time change and **no field on `FormattedTextHeader`**. Heading matching happens in
the editor at click time via `find_matching_header`
(`app/src/notebooks/editor/model.rs:1351`), which already reads each heading's text out of
the live buffer (`content.text_in_range(...)`, :1371-1373) on every click. The fix is a single
slug normalizer applied on both sides of the comparison inside that function.

**Slug algorithm — genuinely GitHub-compatible, not ASCII-only.** GitHub's heading slugger
(`gfm-auto-identifiers`, the same rule set GitHub Pages/`github/cmark-gfm` implement) does
**not** strip non-ASCII text: it lowercases (Unicode-aware), removes characters that are
punctuation/symbols per the Unicode categories it excludes, converts spaces to `-`, and
otherwise **preserves any Unicode alphanumeric/word character**, including accented Latin,
CJK, Cyrillic, etc. A heading `"Café Société"` slugs to `café-société`, not `caf-socit`;
`"日本語"` slugs to `日本語`. An ASCII-only `[a-z0-9 -]` filter silently breaks every non-English
README anchor, which is a real regression against the product goal, not a cosmetic gap.

Concretely, the normalizer should:

1. Lowercase the input (Unicode-aware lowercasing, e.g. Rust's `str::to_lowercase`, not an
   ASCII-only lowercase — this already matters for e.g. Turkish/German/accented casing).
2. Strip characters that are ASCII punctuation/symbols outside `-` and whitespace (mirroring
   GFM's exclusion set: roughly `!"#$%&'()*+,./:;<=>?@[\]^`{|}~`), while leaving all other
   Unicode letters/digits/marks (CJK, accented Latin, etc.) untouched. This is the one place
   the two algorithms diverge from a naive "strip non-ASCII" reading — the exclusion set is
   defined by *punctuation class*, not by *ASCII range*.
3. Replace runs of whitespace with a single `-`, trim leading/trailing `-`.
4. Do **not** apply `user-content-` or any other prefix (see the Security section's note on
   why Warp doesn't need GitHub's DOM-collision prefix).

Add it as a small pure function (natural home: alongside `find_matching_header` in
`model.rs`, or a shared helper if a second caller appears). It should be unit-testable in
isolation on `&str → String`, with cases spanning plain ASCII, accented Latin, and CJK
headings (see the Testing section's updated case list) — the Unicode-preserving behavior is
exactly the kind of thing that regresses silently if only ASCII cases are tested.

`find_matching_header` already normalizes the incoming fragment as `target = fragment
.strip_prefix('#')` → `urlencoding::decode` → `trim().to_lowercase()` (:1352-1357). Replace
that trailing `to_lowercase()` with the slug normalizer, and change the per-heading comparison
at :1374 from `heading.trim().to_lowercase() == target` to `slug(&heading) == target`. Both
sides then run through the same normalizer, so `#target-section` matches a heading "Target
Section". No document-wide pass and no prior-heading state are needed — first-match-wins for
collisions falls out of the loop returning on the first hit, which satisfies the product
non-goal's "first wins" dedupe stance.

### 3. No anchor index (phase 1)

There is deliberately no anchor index in phase 1. The earlier draft of this spec proposed a
document-scoped `HashMap<String, CharOffset>` rebuilt per edit, and treated its storage and
invalidation lifecycle as the biggest unknown — but that entire structure is unnecessary
because `find_matching_header` already performs the id→offset lookup live: it iterates
`content.outline_blocks()`, filters to headers, and returns the matching header's
`start..end` range directly from the current buffer. Iterating a handful of outline blocks on
a click is cheap, and re-reading the buffer each time means there is nothing to keep in sync
with edits. **No new per-document cached state, no invalidation hook, no lifecycle question.**

Phase 2 (`<a id>`/`<a name>` markers) needs the parsed anchor positions to be reachable at
click time. It resolves this the same way phase 1 does — a live walk at click time, no cached
index — detailed concretely in item 5 below, including the precedence rule for an explicit
anchor and a heading slug that collide.

### 4. Fragment-aware click resolution — already exists (phase 1)

The click branch does **not** need to be built: `maybe_open_url`
(`app/src/notebooks/editor/view.rs:1955`) already does, on a leading `#` (:1971-1983), call
`scroll_to_matching_header`, which on a hit calls
`request_autoscroll_to(AutoScrollMode::PositionOffsetInViewportCenter(range.start))`
(`app/src/notebooks/editor/model.rs:1346`). The `#`-branch, the resolution call, and the
scroll are all live on master. Do **not** target
`FormattedTextElement::register_default_click_handlers` — that helper is used by unrelated
surfaces and is not on the Markdown viewer's click path.

The only phase-1 behavior change here is downstream of the slug fix in item 2:

- **Hit:** already correct — `find_matching_header` returns a range,
  `scroll_to_matching_header` requests the autoscroll, `maybe_open_url` early-returns
  (view.rs:1978-1982). Once item 2's slug normalization lands, headings that previously missed
  now resolve.
- **Miss:** `find_matching_header` returns `None` → `scroll_to_matching_header` returns
  `false` → `maybe_open_url` falls through to the ordinary URL path. Per product invariant 7
  this must be observably inert (no scroll, no error, no crash). Confirm the fall-through does
  not raise a broken-link tooltip for an in-document `#fragment`; if it does, add an
  early-return on the `#`-prefixed miss instead of falling through. This is the one spot the
  implementer must check by hand against invariant 7.

### Render surfaces (GUI vs. TUI)

Master now has a second Markdown render surface — the TUI renderer at
`crates/warp_tui/src/tui_markdown.rs` — and it shares the same `markdown_parser` crate and the
same `FormattedText`/`Hyperlink` model as the GUI. This splits the feature cleanly:

- **`<a href>` *parsing* (item 1) benefits both surfaces for free.** Because the new
  `Hyperlink::Url` fragments are produced in the shared parser, the TUI renderer picks them up
  with no TUI-specific code: `inline_spans` already reads
  `fragment.styles.hyperlink → Some(Hyperlink::Url(url))` and renders the link text styled but
  inert (`crates/warp_tui/src/tui_markdown.rs:194-196`). So an `<a href>` link will *display*
  correctly in the TUI the moment the parser change lands.
- **Fragment *scroll* resolution (items 2–4) is GUI-only.** The whole resolution path lives in
  `app/src/notebooks/editor/` (`maybe_open_url`, `scroll_to_matching_header`,
  `find_matching_header`) — the TUI has no `maybe_open_url` equivalent and no scroll model to
  drive, so there is nothing to hook. `#fragment` clicks resolving to a scroll are explicitly
  **out of scope for the TUI surface** in this slice; the TUI simply renders the link as inert
  styled text. Call this out so a reviewer doesn't read "shared parser" as "shared behavior."

### 5. `<a id>`/`<a name>` anchor markers (phase 2)

> **Superseded by what actually shipped.** This section is the pre-implementation design
> draft and is kept for historical context; it was **not** built. What shipped instead is a
> live text scan over the buffer (see item (ii) in the Feasibility summary above and item 3's
> "no anchor index" precedent) — no new fragment field, no zero-width construct. The one
> design point from this draft that *is* still an open, deferred problem is rendering: this
> draft's zero-width marker would have rendered nothing, matching GitHub; the shipped
> live-text-scan approach leaves the tag visible, because hiding it would require the very
> content-model investment ("no anchor index" i.e. no persisted representation) this section
> otherwise avoided. That trade-off — and the honest sizing of what a hiding fix would cost —
> is tracked as [#13982](https://github.com/warpdotdev/warp/issues/13982).

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

**Lookup strategy at click time: extend the same live walk, no cache.** `find_matching_header`
already re-walks `content.outline_blocks()` on every click rather than consulting a cached
index (item 3) — phase 2 extends that same function rather than introducing a second,
differently-shaped lookup:

- `outline_blocks()` returns block-level markers (`BlockType::Text(BufferBlockStyle::Header {
  .. })`, `CodeBlock`, `PlainText`, list/table markers, etc.) — it does not itself walk inline
  fragments within a block, so it cannot see an `anchor_id` marker sitting mid-paragraph.
  `find_matching_header` therefore needs a second pass alongside the header scan: for each
  block returned by `outline_blocks()`, additionally read that block's fragments (via the same
  `content` accessor the header path already uses to pull heading text,
  `content.text_in_range`/the fragment-level equivalent) and check each fragment's
  `styles.anchor_id` for a match, before or interleaved with the existing header-text
  comparison. Concretely: rename/extend `find_matching_header` (or add a sibling
  `find_matching_anchor` called first) so the combined resolver checks explicit anchors and
  heading slugs in one pass over the document, still with no persisted state — the walk itself
  is the "index," recomputed per click exactly as phase 1 established.
- This keeps phase 2 architecturally consistent with phase 1's core decision (item 3): no
  `HashMap<String, CharOffset>`, no invalidation hook, no lifecycle question. The only new
  per-click cost is reading fragment styles in addition to block outlines, which is the same
  order of cost as the existing header-text read.

**Precedence when an explicit `<a id>` and a heading slug collide on the same fragment.**
Define one deterministic rule: **explicit anchors win.** Concretely, the combined resolver
checks explicit `anchor_id` fragments for an exact match against the (undecoded, as-authored)
id first; only if no explicit anchor matches does it fall back to the slug-normalized heading
comparison from item 2. Rationale: an `<a id="x">` is an author's literal, intentional target
string — it should never be shadowed by an incidental heading-slug collision the author didn't
control (e.g. two different headings elsewhere in the doc normalizing to the same slug, see
item 2's "first-wins" dedupe among headings). This rule composes cleanly with item 2's
first-occurrence-wins rule, which continues to apply only *among* headings when no explicit
anchor is present — i.e. the full ordering is: (1) exact match against an explicit `<a
id>`/`<a name>` value, first occurrence wins if multiple share an id; (2) else, slug-normalized
match against a heading, first occurrence wins among headings. Both levels reuse the same
single-pass walk, so there is no separate index or second traversal for the fallback tier.

### 6. Feature gating

Recommend a **new** feature flag (e.g. `FeatureFlag::MarkdownAnchorLinks`) rather than
riding an existing one — unlike the tables spec, there's no existing "structural HTML"
flag this naturally extends, and gating separately lets phase 1 (headings + `<a href>`) ship
independently of phase 2 (`<a id>`) if their cost estimates diverge during implementation.

### 6b. Resolution repairs (delivered with cross-document navigation)

Cross-document navigation (item 6a) assumed a fragment-less relative link already opens its
target today. Implementation found that assumption false in three independent ways, each
fixed in `app/src/notebooks/link.rs`. All three are in the same code path — `NotebookLinks::resolve`
— and all three block even the plain `[text](other-file.md)` case, so they are prerequisites,
not polish.

**Repair 1 — ccTLD misclassification of bare `file.md`.** `resolve` (link.rs:150-169) applies
a bare-domain heuristic *before* file resolution: it takes the substring up to the first `/`
and, if `addr::parse_domain_name` reports a known public suffix with a root, treats the whole
target as `http://…`. Because `.md` is Moldova's ccTLD (and `.dev`, `.com`, … are TLDs), a bare
`README.md`/`notes.md`/`other-file.md` — no `./`, no `/` — is classified as a domain and opened
in the browser instead of the viewer. **Verified empirically** with `addr` 0.15.6:
`README.md`, `file.md`, `other-file.md`, `notes.md`, `warp.dev`, `google.com` all return
`true` from the heuristic; `./README.md`, `app/src/main.rs`, `index.html` return `false` (the
`./` prefix and multi-segment paths dodge it, and `.html` is not a known suffix — which is why
the existing `test_open_markdown_file_uses_viewer_when_preferred` had to spell its link
`./README.md`). *Fix:* before applying the heuristic, synchronously check whether the
scheme-less target resolves to an existing file relative to the base directory (reusing
`absolute_path_if_valid`, which already does a sync `fs::metadata` existence check). If it
does, resolve as a file; only fall through to the domain heuristic when there is no matching
local file — so `warp.dev` (no local file) still opens the browser, and a bare `nonexistent.md`
with nothing on disk also still does. Deterministic and file-existence-gated.

**Repair 2 — literal `#fragment` breaks the file stat.** `CleanPathResult` strips `:line:col`
and `#L100` suffixes (`crates/warp_util/src/path.rs:47-57`) but **not** a bare `#section`
fragment, so `other-file.md#section` reaches `resolve_file` as a literal on-disk path, misses,
and `resolve_and_open`'s closure silently drops the error (link.rs:328-332) — today's no-op.
*Fix:* a `split_anchor_fragment` helper peels the trailing `#…` off before file resolution,
returning `(path, Option<anchor>)`. It splits only the final `#` (so `weird#name.md#frag`
keeps `weird#name.md`), decodes the fragment with `urlencoding`, treats an empty fragment as
no anchor, and — critically — leaves a `#L<digits>[:<digits>]` suffix attached to the path so
the existing line-number routing in `CleanPathResult` is not regressed. The split runs *after*
the explicit-URL branch (so a real `https://…#frag` keeps its fragment) and feeds repair 1's
file check.

**Repair 3 — standalone viewer tab lacks a base directory.** A cross-document link clicked
from *within* an open notebook already gets the right base dir: `FileNotebookView::open_local`
→ `set_context` sets `SessionSource::Target { base_directory: <document parent> }`
(`app/src/notebooks/file/mod.rs`), so this repair is not needed for the primary in-app click
path. The gap is a standalone viewer tab opened with no session (`open -a Warp file.md`): with
no session, `SessionSource::Active` fell back to the window's active-session cwd, which is
absent, so even `./file.md` resolved to `MissingContext`. *Fix:* `SessionSource::Active` now
carries an optional `document_dir`; `base_directory()` prefers the active session's local cwd
and falls back to the document's own parent directory. `open_local`'s no-session branch seeds
that fallback from the file's parent. The document knows where it lives even when the window
doesn't. (A truly session-less tab still cannot resolve files that require a `Session` object
for other reasons; surfacing resolution failures non-silently — `resolve_and_open` swallows
errors at link.rs:328-332 — is noted as a **follow-up**, since no cheap existing tooltip
affordance is on this path and the spec forbids building new UI here.)

### 6a. Cross-document fragment navigation (delivered in this PR)

**Size: MEDIUM.** The file-open half is free — a fragment-less relative link
(`other-file.md`) already opens the target in the Markdown viewer today, including
tab-focus and dedup. What's missing is carrying the `#section` through that flow and
scrolling after the *new* document parses. This is bounded, well-scoped plumbing, not new
targeting machinery — but it is more than a single parameter through one existing call, and
it needs a new piece of deferred state on the destination editor because the scroll target
can't be applied until the document is loaded and there is no on-load hook today.

**What already works (verified on master).** Clicking a relative link with no fragment
routes through `maybe_open_url` (`app/src/notebooks/editor/view.rs:1955`); because it does
not start with `#`, it goes to `NotebookLinks::resolve_and_open`
(`app/src/notebooks/link.rs:323`). `resolve` (:128) resolves the relative path against the
session's base directory (:211-222), `resolve_file` (:235) confirms it exists on disk, and
`open` (:258) emits `LinkEvent::OpenFileNotebook` for a Markdown target (:282/:290). That
event is consumed at `app/src/pane_group/pane/notebook_pane.rs:175`, re-emitted as
`Event::OpenFileInWarp`, and handled by `Workspace::open_file_notebook`
(`app/src/workspace/view.rs:8470`) — which **already de-dupes an open pane and focuses it**
(:8489-8494) or opens a new tab/split (:8503-8520). So open, focus, and dedup for the target
document are live; only the fragment is lost.

> **Dedup needs canonicalization for self-reference (and symlink aliases).** The dedup
> compares `file_view.path()` against the resolved link path, but an open notebook stores its
> *canonical* path (recorded on load via `CanonicalizedPath`/`dunce::canonicalize`), while the
> link resolves to `base_directory.join(relative)` — which keeps `.`/`..` components and, on
> macOS, the `/tmp` vs `/private/tmp` symlink alias. Without normalization a self-referential
> link (`./this-doc.md`, the same file that's already open) fails to match its own pane and
> opens a duplicate. Fix: canonicalize the resolved local target with the *same*
> `dunce::canonicalize` before the dedup comparison (extracted as
> `canonicalize_local_path_for_dedup`, unit-tested against the `.`/`..`/symlink-alias shapes).
> A self-link with a fragment then hits the already-open branch and scrolls immediately; a
> self-link without one just refocuses. This is invariant 12.

**Why the fragment is lost today.** The path is cleaned by
`CleanPathResult::with_line_and_column_number` (`crates/warp_util/src/path.rs:158`), whose
`LINE_AND_COLUMN_REGEX` (:47) strips `:line[:col]`, `[l, c]`, and `#L100`-style suffixes
(:48-56) but **not** a bare `#section` fragment. So `other-file.md#section` survives cleaning
intact, is resolved as a literal on-disk path, misses (`ResolveError::FileNotFound`), and
`resolve_and_open`'s closure silently drops the error (:328-332) — the current no-op.

**The precedent for "open a file AND position the viewport" already exists — for the code
editor.** `LinkTarget::LocalFile` carries `line_and_column` (link.rs:35) end-to-end, and
`add_tab_for_code_file` (`app/src/workspace/view.rs:12828`) threads it through to position the
code editor. The Markdown-notebook path is the gap: `add_tab_for_file_notebook` (:12767) and
`open_file_notebook` (:8470) carry only a `path`, no viewport target, and `OpenFileNotebook`
(link.rs:441) has no fragment field.

**Concrete mechanism (three localized changes):**

1. **Split the fragment before file resolution.** In `NotebookLinks::resolve`
   (link.rs:128), for a link that is neither a parseable URL nor a bare `#fragment`, peel a
   trailing `#…` off the string before it reaches `CleanPathResult`, keeping it as an
   `Option<String> anchor` alongside the cleaned path. (A bare `#fragment` is still handled
   earlier by `maybe_open_url`'s `starts_with('#')` branch — this split only affects strings
   that have a path *and* a fragment.) Once the fragment is removed, `resolve_file` finds the
   real file and the existing open/focus flow runs unchanged.

2. **Thread the fragment to the destination pane**, mirroring how `line_and_column` already
   rides the code path. Add `anchor: Option<String>` to `LinkTarget::LocalFile` (link.rs:33),
   to `LinkEvent::OpenFileNotebook` (link.rs:441), to `pane_group::Event::OpenFileInWarp`
   (`app/src/pane_group/mod.rs:549`), and through `open_file_notebook` (view.rs:8470) into the
   `FilePane`/notebook it constructs. This is additive field-plumbing along an existing event
   chain — the analog of the code editor's `line_and_column`, which already proves the shape
   works end-to-end.

3. **Apply the scroll after the destination document loads — the one genuinely new piece.**
   The same-document jump can call `scroll_to_matching_header` immediately because the buffer
   is already parsed. A freshly opened notebook is not: the offset the slug resolves to does
   not exist until parse completes, and there is **no on-load callback in the notebook-open
   flow to hang the scroll on** (`open_file_notebook` constructs the pane and returns; nothing
   fires when its content finishes parsing). The destination editor model therefore needs a
   small piece of **deferred-scroll state** — a `pending_anchor: Option<String>` set at
   construction — that is consumed once, on the first successful parse/layout, by calling the
   existing `scroll_to_matching_header` (`app/src/notebooks/editor/model.rs:1335`) with the
   pending fragment and then clearing it. This reuses phase 1's resolver verbatim; the only
   new logic is *when* to call it. The implementer must locate the model's
   content-ready/relayout point (the notebook already rebuilds layout on content load — e.g.
   the `rebuild_layout` path around `model.rs:1315`) and drain `pending_anchor` there. If no
   anchor matches after load, draining is a no-op — identical to the same-document miss
   (product invariant 7).

**Resolution reuse.** The slug comparison is entirely unchanged: the destination scroll runs
through the same `find_matching_header` (`app/src/notebooks/editor/model.rs:1351`) the
same-document jump uses, so phase 1's slug normalizer (item 2) is the cross-document resolver
too — no second matcher, no divergence. This is the concrete sense in which phase 1 "leaves
room" for cross-document navigation: the target document's resolver is the *same function*,
invoked after open instead of in place.

**Non-Markdown / external-editor targets.** If the Markdown Viewer preference is off, `open`
routes the file to `open_file` → the code editor or system handler (link.rs:284), which has
no slug concept. Per the product non-goal, the fragment is simply dropped in that case: the
file opens, unscrolled. Only the `OpenFileNotebook` branch carries the anchor.

### 7. Security

`<a href>` reuses `Hyperlink::Url` verbatim — no new trust boundary, no script/event-handler
attributes are read (only `href`; all others parsed-but-discarded per product invariant 8).
Fragment resolution never leaves the document (no network, no file access) — a `#fragment`
click either scrolls within the current buffer or is a no-op. `<a id>`/`<a name>` values
(phase 2) are used only as in-document lookup keys, never interpolated into a URL, path, or
shell context.

**On GitHub's `user-content-` prefix (intentionally not replicated).** GitHub's renderer
prefixes the *rendered DOM ids* it emits for headings and `<a id>` anchors with
`user-content-` (to avoid collisions with GitHub's own page-chrome ids), while leaving the
`href="#…"` fragments authors write unprefixed; a small piece of client JS bridges the two at
click time. Warp has no such collision surface and no DOM: resolution happens **in-process**,
comparing the fragment against slugs computed live from heading text (item 2), not against any
emitted id attribute. So Warp deliberately does **not** add a `user-content-` prefix on either
side — there is nothing for it to disambiguate against, and adding it would only break parity
with the plain `#slug` fragments authors actually write. Worth stating explicitly so a
reviewer familiar with GitHub's scheme doesn't flag its absence as a bug.

## Testing and validation

**Validation is committed, tracked test cases — not the probe file.** All cases below live in
tracked test modules (`markdown_parser_tests.rs`, editor-model tests) that CI runs today and
will run against the implementation PR; nothing in this spec's validation depends on a file
that isn't checked in.

**Historical context on the probe file.** The master checkout currently carries an *untracked*
scratch file, `crates/markdown_parser/src/html_tag_support_tests.rs` (wired via `#[path] mod
html_tag_support_tests;` at `markdown_parser.rs:1998-1999`, itself marked "DO NOT COMMIT" in
its header comment), whose two anchor probes —
`test_raw_html_anchor_href_not_parsed_as_hyperlink` and
`test_raw_html_anchor_id_not_registered_as_target` — assert today's **no-op** status quo (raw
`<a href>` produces no hyperlink style; raw `<a id>` registers no target). These were exploratory
probes used to confirm current behavior while scoping this spec, not a test suite the
implementation is meant to build on. The implementation PR must **not** leave these assertions
in place or rely on the untracked file continuing to exist — it must add the committed positive
assertions listed below to `markdown_parser_tests.rs` directly (`<a href>` *does* produce a
`Hyperlink::Url`; `<a id>` *does* register an anchor), which subsumes and inverts what the probe
file was checking. If the probe file is still present at implementation time, delete it or fold
any remaining useful cases into the committed suite — it should not ship as project state.

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<a href="https://warp.dev">Visit Warp</a>` → `Hyperlink::Url` fragment identical in
  shape to the equivalent markdown link (invariant 1).
- `<a href="#target">Jump</a>` → `Hyperlink::Url("#target")` fragment (invariant 2) —
  parsing only; resolution is tested separately below.
- Attributes beyond `href` (`title`, `target="_blank"`, `class="x"`) parsed-but-ignored, no
  effect on output (invariant 8).
- Unterminated `<a href="…">` / missing closing `</a>` → literal text fallback, rest of
  paragraph intact (invariant 10).
- Slug normalizer (item 2), tested as a pure `&str → String` function, table-driven over:
  - Plain ASCII text, text with punctuation/mixed case, multi-space runs → normalized slug
    (invariant 4).
  - **Unicode headings, to lock in genuine GitHub-compatibility (not ASCII-only):**
    accented Latin (`"Café Société"` → `café-société`), CJK (`"日本語"` → `日本語`, unchanged —
    no word characters to strip and no ASCII case to fold), and a mixed-script heading
    (e.g. `"Section 日本語 Café"` → `section-日本語-café`). These guard against silently
    reintroducing an ASCII-only filter.
  - First-wins collision behavior is covered by the resolution tests below, not the
    normalizer itself.
- (Delivered, revised from the original phase-2 draft) `<a id="x"></a>` / `<a name="x"></a>`
  → parses as a single literal-text fragment (`FormattedTextFragment::plain_text`), **not** a
  zero-width marker — characterization confirmed the phase-1 `<a href>` grammar requires
  `href` to match, so an id-only tag falls through to plain text and stays visible (invariant
  5's rendering caveat, deferred to #13982). *Landed* as
  `test_parse_html_anchor_unterminated_falls_back_to_text`'s trailing case in
  `markdown_parser_tests.rs`.
- (Phase 2) `<a id="x">text</a>` (both id and content on one tag) — confirm documented
  behavior (out of scope; assert it doesn't panic, even if unspecified which role wins).

### Resolution tests (`app/src/notebooks/editor/` — `find_matching_header` / `model.rs`)

These target the matcher directly, since that is where the phase-1 change lives.

- Heading `## Target Section` + fragment `#target-section` → `find_matching_header` returns
  the heading's range; today (exact-text match) it returns `None`. This is the regression the
  slug normalizer fixes (invariants 2, 3, 4).
- Heading with a non-English title (e.g. `## Café Société`) + fragment `#café-société` →
  resolves, exercising the Unicode-preserving normalizer end-to-end through the matcher, not
  just the pure function in isolation.
- `<a href="#slug">` and markdown `[text](#slug)` targeting the same heading resolve to the
  same range (invariant 3 — the two syntaxes are equivalent because both reach
  `find_matching_header` via the same `#`-branch in `maybe_open_url`).
- Fragment with no matching heading → `find_matching_header` returns `None`,
  `scroll_to_matching_header` returns `false`, no panic (invariant 7).
- First-wins collision: two headings normalizing to the same slug → the fragment resolves to
  the first, exercised by asserting the returned range is the earlier heading's.
- (Phase 2) Explicit `<a id="x">` sharing a fragment/document with a heading whose implicit
  slug is also `x` → the combined resolver's explicit-anchor-first pass (item 5) returns the
  anchor's position, not the heading's, per the defined precedence rule (invariant 6).
- (Phase 2) Explicit anchor lookup when no heading collides: `<a id="x"></a>` alone resolves
  via the fragment-styles pass added to the combined resolver (item 5), independent of the
  header-outline pass.
- (Phase 2) Two `<a id="x">` markers with the same id → first occurrence wins, mirroring the
  heading first-wins rule.
- (Cross-doc, delivered) Fragment-split in `resolve`: `other-file.md#section` yields cleaned
  path `other-file.md` + anchor `section`; `other-file.md` (no fragment) yields no anchor;
  `other-file.md#L10` still routes to the existing line-number path, not the anchor path
  (guard against regressing the `#L`-suffix handling in `CleanPathResult`). *Landed* as
  `test_split_fragment_before_file_resolution`, `test_fragment_split_preserves_line_number_routing`,
  and the pure-function `test_split_anchor_fragment_pure` (covers multiple `#`, empty
  fragment, URL-decode, `#L` vs `#License`/`#L10x`).
- (Repair 1, delivered) ccTLD classification matrix: a bare `README.md`/`notes.md` that
  exists on disk resolves as a file; `warp.dev` and a bare `nonexistent.md` with no local
  file still resolve as URLs. *Landed* as `test_bare_markdown_file_prefers_local_file_over_cctld`;
  the existing viewer test was also switched from `./README.md` to bare `README.md` to assert
  the repair end-to-end.
- (Cross-doc, delivered) A resolved `LinkTarget::LocalFile` for a `#section` link carries the
  anchor through to `OpenFileNotebook` (assert on the emitted event). *Landed* as
  `test_cross_document_fragment_threads_anchor_to_open_event`.
- (Cross-doc, delivered) Deferred-scroll drain: a `pending_anchor` set before layout resolves
  through the same slug matcher on drain (hit → scroll, cleared one-shot; miss → silent
  no-op). *Landed* as `test_pending_anchor_drains_and_scrolls_on_match` and
  `test_pending_anchor_miss_is_silent_no_op` in the editor-model tests.

### Integration / manual

Per CONTRIBUTING, before/after screenshots plus a short recording reproducing the issue's
motivating test document verbatim: the raw-HTML `<a href="#target-section">` jump, the
markdown-native `[Jump to Target Section](#target-section)` jump (contrast case — today
resolves as a plain URL), the external `<a href="https://warp.dev">` link, and the `<a
id="target-section"></a>` marker preceding the heading. Cross-document (delivered)
additionally: clicking `[text](other-file.md#section)` opens/focuses `other-file.md` and
lands on its `section` heading after load; a bare `[text](other-file.md)` link opens the
target in the viewer (not the browser — the ccTLD repair); re-clicking when the tab is
already open focuses and re-scrolls rather than opening a duplicate; a `#section` with no
matching heading in the opened file shows the file unscrolled with no error. Confirm scroll lands the heading in
view (the existing call uses `AutoScrollMode::PositionOffsetInViewportCenter`, i.e. the target
is centered rather than top-aligned — sanity-check that this reads well for the anchor-jump
case, since it is the behavior already shipped for other `#`-fragment jumps and changing it
would affect them too).

## Risks and follow-ups

- **No anchor-index lifecycle risk in phase 1.** An earlier draft of this spec called a
  per-document id→offset map's storage and invalidation the single biggest unknown. That risk
  is now moot: phase 1 builds no index (item 3). `find_matching_header` resolves live against
  the current buffer per click, so there is nothing to store, sync, or invalidate. The only
  residual judgment call is cosmetic — whether the existing center-in-viewport autoscroll
  reads well for anchor jumps (noted in the manual-testing section).
- **The one behavioral check the implementer must not skip** is the miss path (item 4): today
  a `#`-fragment that resolves to nothing falls through to the URL opener. Confirm that
  fall-through is observably inert (no broken-link tooltip) per invariant 7, and add an
  early-return if it is not. This is the sole place phase-1 correctness is not already
  guaranteed by existing master behavior.
- **`<a href>`'s attribute parser (item 1) needs a maintainer call between a purpose-built
  scanner and reusing `html5ever`.** The purpose-built path matches the `<u>` precedent's
  spirit (minimal, inline-grammar-native) but is more exposed to malformed real-world HTML
  than a real parser; `html5ever` is already a dependency of the crate (via
  `html_parser.rs`) so there's no new-dependency cost either way.
- **Interaction with the HTML-table spec:** an `<a href>`/`<a id>` inside a table cell should
  work automatically once cell inline content is parsed via the same `parse_phrasing_content`
  path the tables spec already plans to reuse — verify once both land, no explicit design
  change anticipated here.
- **Cross-document fragment links are delivered in this PR (item 6a), with three resolution
  repairs (item 6b).** The original spec assumed a fragment-less relative link already
  opens/focuses the target Markdown-viewer tab today. That was only partly true: a bare
  `file.md` (no `./`) misrouted to the browser (ccTLD collision), a `#fragment` broke the file
  stat, and a standalone viewer tab lacked a base directory. Repairing those was a prerequisite
  for the feature, so the two ship together. The cross-document jump resolves against the
  *destination* document's buffer using the **same** `find_matching_header` slug resolver,
  invoked after open via a `pending_anchor` drained on first `LayoutUpdated`; the dedup case
  (tab already open) scrolls immediately. See items 6a/6b and product invariant 11.
- **Non-silent resolution failures (follow-up).** `resolve_and_open` swallows resolution
  errors (link.rs closure). A genuinely session-less standalone tab still can't resolve files
  that need a `Session` object, and today that failure is silent. Surfacing it (e.g. a
  broken-link tooltip) is deferred — no cheap existing affordance is on this path, and the
  spec forbids building new UI in this slice.
- **`<a id>`/`<a name>` explicit anchor markers (follow-up, was "phase 2").** Not in this PR.
  Heading auto-anchors cover the common case; item 5's design stands for the follow-up.
