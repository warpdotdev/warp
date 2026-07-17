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

- **(iv) Cross-document fragment navigation (`other-file.md#section`): MEDIUM.** The
  file-open, tab-focus, and dedup are free — a fragment-less relative link opens the target
  in the Markdown viewer today (`resolve_and_open` → `OpenFileNotebook` →
  `open_file_notebook`, `app/src/workspace/view.rs:8470`). The remaining work is bounded:
  split the `#section` off before file resolution, thread it to the destination pane
  (mirroring the code editor's existing `line_and_column` plumbing), and drain it as a
  **deferred scroll** once the new document parses — the one genuinely new element, since
  there is no on-load hook to reuse. Sized above SMALL because of that new deferred-scroll
  state and the multi-hop field plumbing; below LARGE because the targeting, dedup, and
  scroll primitives all already exist. Full mechanism in **item 6a**.

This spec recommends implementing **(i) + (iii)** as phase 1 (matches the product spec's
phasing — this is the slice that fixes the issue's headline test case and also repairs
markdown-native `[text](#heading)` links, which get zero benefit from (ii) alone),
**(ii)** as phase 2, and **(iv)** as phase 3.

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

Slug algorithm (GitHub-compatible, since that's the ecosystem convention the product spec
points to): lowercase, strip characters outside `[a-z0-9 -]`, collapse/trim spaces, replace
spaces with `-`. Add it as a small pure function (natural home: alongside `find_matching_header`
in `model.rs`, or a shared helper if a second caller appears). It should be unit-testable in
isolation on `&str → String`.

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
click time, and that is where any indexing question actually lives — deferred to that phase's
design, not phase 1's.

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

### 6a. Cross-document fragment navigation (phase 3)

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

**Invert the existing negative-behavior probes first.** The master checkout carries an
untracked probe file, `crates/markdown_parser/src/html_tag_support_tests.rs` (wired via
`#[path] mod html_tag_support_tests;` at `markdown_parser.rs:1998-1999`), whose two anchor
tests — `test_raw_html_anchor_href_not_parsed_as_hyperlink` and
`test_raw_html_anchor_id_not_registered_as_target` — currently assert the **no-op** status
quo (raw `<a href>` produces *no* hyperlink style; raw `<a id>` registers *no* target). Once
item 1 lands, both assertions become false and CI goes red. The implementation must **invert
them into positive assertions** (`<a href>` *does* produce a `Hyperlink::Url`; `<a id>` *does*
register an anchor) as part of the same change, not leave them asserting the old behavior.

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<a href="https://warp.dev">Visit Warp</a>` → `Hyperlink::Url` fragment identical in
  shape to the equivalent markdown link (invariant 1).
- `<a href="#target">Jump</a>` → `Hyperlink::Url("#target")` fragment (invariant 2) —
  parsing only; resolution is tested separately below.
- Attributes beyond `href` (`title`, `target="_blank"`, `class="x"`) parsed-but-ignored, no
  effect on output (invariant 8).
- Unterminated `<a href="…">` / missing closing `</a>` → literal text fallback, rest of
  paragraph intact (invariant 10).
- Slug normalizer (item 2): plain text, text with punctuation/mixed case, multi-space runs
  → normalized slug (invariant 4). Test the pure `&str → String` function directly; first-wins
  collision behavior is covered by the resolution tests below, not the normalizer.
- (Phase 2) `<a id="x"></a>` / `<a name="x"></a>` → zero-width anchor marker, no visible
  text emitted (invariant 5).
- (Phase 2) `<a id="x">text</a>` (both id and content on one tag) — confirm documented
  behavior (out of scope; assert it doesn't panic, even if unspecified which role wins).

### Resolution tests (`app/src/notebooks/editor/` — `find_matching_header` / `model.rs`)

These target the matcher directly, since that is where the phase-1 change lives.

- Heading `## Target Section` + fragment `#target-section` → `find_matching_header` returns
  the heading's range; today (exact-text match) it returns `None`. This is the regression the
  slug normalizer fixes (invariants 2, 3, 4).
- `<a href="#slug">` and markdown `[text](#slug)` targeting the same heading resolve to the
  same range (invariant 3 — the two syntaxes are equivalent because both reach
  `find_matching_header` via the same `#`-branch in `maybe_open_url`).
- Fragment with no matching heading → `find_matching_header` returns `None`,
  `scroll_to_matching_header` returns `false`, no panic (invariant 7).
- First-wins collision: two headings normalizing to the same slug → the fragment resolves to
  the first, exercised by asserting the returned range is the earlier heading's.
- (Phase 2) Explicit `<a id="x">` colocated with a heading whose implicit slug is also `x`
  → explicit anchor wins per invariant 6, or documented fallback if not achievable.
- (Phase 3) Fragment-split in `resolve`: `other-file.md#section` yields cleaned path
  `other-file.md` + anchor `section`; `other-file.md` (no fragment) yields no anchor;
  `other-file.md#L10` still routes to the existing line-number path, not the anchor path
  (guard against regressing the `#L`-suffix handling in `CleanPathResult`).
- (Phase 3) A resolved `LinkTarget::LocalFile` for a `#section` link carries the anchor
  through to `OpenFileNotebook` (assert on the emitted event, mirroring the existing
  `link_tests.rs:432` `OpenFileNotebook` assertion).

### Integration / manual

Per CONTRIBUTING, before/after screenshots plus a short recording reproducing the issue's
motivating test document verbatim: the raw-HTML `<a href="#target-section">` jump, the
markdown-native `[Jump to Target Section](#target-section)` jump (contrast case — today
resolves as a plain URL), the external `<a href="https://warp.dev">` link, and the `<a
id="target-section"></a>` marker preceding the heading. (Phase 3) additionally: clicking
`[text](other-file.md#section)` opens/focuses `other-file.md` and lands on its `section`
heading after load; re-clicking when the tab is already open focuses and re-scrolls rather
than opening a duplicate; a `#section` with no matching heading in the opened file shows the
file unscrolled with no error. Confirm scroll lands the heading in
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
- **Cross-document fragment links are now specced as phase 3 (item 6a), not just gestured
  at.** Investigation confirmed a fragment-less relative link already opens/focuses the target
  Markdown-viewer tab today, so phase 3 is MEDIUM (carry the fragment through + deferred
  scroll after load), not LARGE. Phase 1's live-resolution approach doesn't paint this into a
  corner: the cross-document jump resolves against the *destination* document's buffer using
  the **same** `find_matching_header` slug resolver, invoked after open instead of in place —
  so nothing in phase 1 forecloses it, and phase 1 landing first is what phase 3 builds on.
  The one new element phase 3 introduces is deferred-scroll state on the destination editor
  (there is no on-load hook to reuse); see item 6a and product invariant 11.
