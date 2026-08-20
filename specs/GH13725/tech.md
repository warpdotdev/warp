# TECH.md — Markdown viewer: `<a href>`/`<a id>` anchor links

Product spec: `specs/GH13725/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13725

All file and line references in this document were verified against master at commit `495f97572`. They describe the code as it exists *before* this feature, so a reviewer can read the current state and the proposed change without ambiguity about which is which.

## Context

The two halves of this issue have very different shapes, and the scoping only makes sense once both are pinned to real code.

### `<a href>` parsing: the inline grammar has no HTML concept

`InlineToken` (`crates/markdown_parser/src/markdown_parser.rs:1731`) covers `Delimiter`, `Text`, `BackslashEscape`, `HtmlEntity`, `CodeSpan`, `AutoLink`, `LinkEnd`, and `UnderlineEnd`. The only literal HTML tag special-cased anywhere in the inline grammar is `<u>`/`</u>`, handled by `parse_inline_token_underline_start` / `parse_inline_token_underline_end` (`markdown_parser.rs:1682` / `:1695`, registered in the `alt` chain at `:1597-1598`) exactly like a markdown delimiter pair — a token push, no attribute parsing.

`<a href="…">…</a>` is a direct structural analog: a start delimiter carrying one piece of data plus a fixed end tag. The output type already exists, too. `parse_link` (`markdown_parser.rs:1169`) and `parse_link_target` (`:1236`) build precisely the styling this needs, assigning `styles.hyperlink = Some(Hyperlink::Url(url.clone()))` at `:1202`. An HTML `<a>` reader needs a new front door into that styling, not a new link model.

### Fragment resolution: the click path and scroll already exist; the *matcher* is wrong

This is the part that reshapes the scoping, and it is worth being precise because it is easy to over-estimate.

A markdown link `[text](#fragment)` **already parses today**. `parse_link_target` has no opinion on a destination beyond balanced delimiters, so `#fragment` is accepted as a URL string like any other — confirmed by a committed test at `markdown_parser_tests.rs:1644-1648` ("Example 501"), which asserts `[link](#fragment)` produces `FormattedTextFragment::hyperlink("link", "#fragment")`.

The click path **already branches on a leading `#`**. `NotebookEditorView::maybe_open_url` (`app/src/notebooks/editor/view.rs:1959`) tests `url.starts_with('#')` at `:1976` and calls `model.scroll_to_matching_header(&url, ctx)` at `:1981`, early-returning on a hit and falling through to ordinary URL handling on a miss.

The scroll primitive is **already wired end to end**. `scroll_to_matching_header` (`app/src/notebooks/editor/model.rs:1335`) calls `request_autoscroll_to(AutoScrollMode::PositionOffsetInViewportCenter(range.start))` at `:1346`, backed by `EditorRenderState::request_autoscroll_to` (`crates/editor/src/render/model/mod.rs:3119`).

What is actually broken is one comparison. `find_matching_header` (`app/src/notebooks/editor/model.rs:1351`) strips the `#`, rejects an empty fragment, `urlencoding::decode`s it, and lowercases it (`:1352-1357`). It then walks `content.outline_blocks()`, filters to `BlockType::Text(BufferBlockStyle::Header { .. })`, reads each heading's text live out of the buffer via `content.text_in_range(...)`, and compares at `:1374`:

```rust
if heading.trim().to_lowercase() == target {
```

That is an **exact lowercased text** comparison, not a slug comparison. `#target-section` (hyphenated) cannot match a heading reading `Target Section` (spaced) — precisely the issue's failing case. URL-escaped fragments are already handled; only the text-versus-slug rule is wrong.

Two consequences worth stating, because they remove work an earlier reading would assume:

- **No content-model field is needed for headings.** `FormattedTextHeader` (`crates/markdown_parser/src/lib.rs:303`) is `{ heading_size: usize, text: FormattedTextInline }` with no id or slug field, and it does not need one: matching happens in the editor at click time against live buffer text, never against a parse-time slug.
- **No anchor index is needed.** `find_matching_header` already performs the id-to-offset lookup live on every click. There is no map to build, no place to store it, and no invalidation problem. This is also what makes product invariant 16 (resolution reflects the current document) true by construction rather than by maintenance.

`grep -rn "slug" crates/markdown_parser/ crates/editor/` finds no slug-generation helper, so the normalizer itself is genuinely new code — but it plugs into a loop that already exists.

### Where the fix does *not* go

`FormattedTextElement::register_default_click_handlers` (`crates/warpui_core/src/elements/gui/formatted_text_element.rs:336`) is a general helper used by settings pages, banners, modals, the changelog, and AI views. It is **not** on the notebook Markdown viewer's click path and none of its callers has a `#`-fragment branch. It is not the fix site.

### Render surfaces

Master has two Markdown render surfaces sharing the `markdown_parser` crate and the `FormattedText`/`Hyperlink` model, and this feature splits cleanly across them.

The **GUI Markdown viewer** (`app/src/notebooks/editor/`) owns the entire resolution path: `maybe_open_url`, `scroll_to_matching_header`, `find_matching_header`. All navigation behavior in this spec is GUI behavior.

The **TUI renderer** (`crates/warp_tui/src/tui_markdown.rs`) consumes the same parser output. `inline_spans` (`tui_markdown.rs:228`) reads `fragment.styles.hyperlink`, matching `Some(Hyperlink::Url(url))` at `:237-239` and applying link styling at `:298`. It has no `maybe_open_url` equivalent and no scroll model. So `<a href>` *parsing* benefits the TUI for free — a link will display styled the moment the parser change lands — while fragment *scrolling* has nothing to hook into and is out of scope there. This is product invariant 14. A reviewer should not read "shared parser" as "shared behavior."

The **notebook/plan editor** surfaces (`app/src/code/editor/`, `app/src/ai/ai_document_view.rs`) construct `NotebooksEditorModel` and share the buffer and rendering stack with the viewer. They inherit `<a href>` rendering through the same shared parser, and they inherit fragment resolution wherever they route clicks through `maybe_open_url`; no surface-specific code is added for them. Where a surface does not route through that entry point, a `#fragment` is inert there, which is invariant 7's miss behavior and requires no separate handling.

## Feasibility summary

- **(i) `<a href>` inline parsing → `Hyperlink::Url`: SMALL.** A new token plus an attribute-extraction step for `href`, reusing 100% of the existing link styling, click, and render path.
- **(ii) `<a id>`/`<a name>` targets: SMALL.** No new content-model field. Resolution reads **buffer text**, the same access pattern the heading path already uses, and the grammar is shaped so that every id-bearing tag stays untokenized and therefore visible in that text: a bare `<a id="x">` never matches the `href`-requiring grammar, and a dual-role `<a href="…" id="…">` is excluded from it deliberately. See item 3 for the mechanism, including why consuming a dual-role tag would lose its id, plus code exclusion and attribute-order independence. The cost of this choice is that an id-bearing tag stays visible (product invariant 5); hiding it needs a save-round-trippable representation and is deferred to #13982.
- **(iii) Heading slug matching + click resolution: SMALL.** The click branch, the scroll call, and the per-click heading walk all exist. The work is a slug normalizer and one comparison change.
- **(iv) Cross-document fragment navigation: MEDIUM.** The file-open, tab-focus, and dedup machinery exists, but three latent resolution defects (item 6) block even a fragment-less bare `file.md` link, and the deferred scroll after load is genuinely new state.

### Sequencing

Items (i) and (iii) are the natural first slice: together they fix the issue's headline case and repair markdown-native fragment links, and neither depends on the other. Item (ii) follows cheaply because it reuses (iii)'s click-time walk rather than adding a parsed anchor node — the two share a single resolver, so building them separately would mean writing that resolver twice. Item (iv) is last and largest, and it carries item 6's repairs as hard prerequisites: without them a plain relative link is already broken, so the cross-document feature has no working baseline to build on.

This ordering describes how the work decomposes for review, not a staged rollout — the product spec's delivery-scope section is the authoritative statement of what ships together. Item (ii)'s visible-literal-tag consequence (product invariant 5, item 3 below) ships as current known behavior, tracked for improvement as #13982 rather than held back.

## Proposed changes

### 1. `<a href="…">text</a>` inline token

Add HTML-anchor tokens to `InlineToken` (`markdown_parser.rs:1731`), following the `<u>` precedent in shape but carrying data. Unlike `<u>`'s zero-data delimiter, the start token must thread the `href` value through to close time — either as its own variant (`HtmlAnchorStart(String)` paired with `HtmlAnchorEnd`) or by stashing the value on the delimiter-stack entry, mirroring how `parse_link` stashes `link_start.node_index` at `:1169`.

Closing `</a>` applies `styles.hyperlink = Some(Hyperlink::Url(href))` to the fragments between start and end — the same `backtrack_styles` assignment `parse_link` makes at `:1202`, triggered by `</a>` instead of `](url)`.

**Rich inline content inside the anchor is supported, satisfying product invariant 9's nice-to-have rather than its plain-text floor.** This costs nothing to build: the content between `<a>` and `</a>` is parsed by the ordinary inline loop before the closing tag applies the hyperlink style, so `**bold**`, `*italic*`, and code spans inside anchor text produce their usual fragments and the hyperlink style is layered onto them. This is the same reason `[**bold** link](url)` works today — the style assignment is additive over existing fragments, not a replacement of them.

The opening tag needs a minimal attribute scanner: extract `href` (single- or double-quoted), and tolerate-and-discard every other attribute per product invariant 8 — with one exception. **A tag whose attribute list carries `id` or `name` does not match this grammar at all**, and falls through to literal text so that item 3's source-text scan can still see its id. Item 3 states why in full; the scanner needs only to detect the presence of either attribute and reject the tag. **The scanner's construction is a decision for maintainer review.** A purpose-built `key="value"` scanner matches the `<u>` precedent's spirit and keeps the inline grammar self-contained, but is more exposed to malformed real-world markup. `html5ever` is already a dependency of this crate (`crates/markdown_parser/Cargo.toml:16`, used by the separate paste-path parser in `html_parser.rs`), so reusing it carries no new-dependency cost — only the cost of pulling a full HTML tokenizer into a single-tag inline case. The spec does not pick for you; both are viable and the tradeoff is robustness against grammar simplicity.

Malformed input falls back to literal text for the tag, matching how `parse_link` falls back to a literal `]` on failure. The deterministic cases product invariant 10 enumerates map onto the parser as follows: an unterminated `<a` fails the tag parse and emits literal text; a missing `</a>` leaves the start delimiter unmatched on the stack and it is emitted as literal text at paragraph end, exactly as an unmatched markdown delimiter is; an empty or valueless `href` fails the attribute scan and the whole tag degrades to literal text; nested `<a>` resolves by the outer tag pairing with the first `</a>`, leaving the inner opening tag as literal text. Because the anchor parser is registered in the inline `alt` chain alongside the other inline tokens, code spans and fenced blocks never reach it — they are consumed by the existing `CodeSpan` handling first — which is what keeps anchor markup in code from *rendering as a link* without special-casing. This protection is parse-time only and covers only the link half of invariant 10's code clause; the target half is a separate mechanism running at click time over source text, and it needs its own explicit code exclusion (item 3).

### 2. Heading slug normalization

No parse-time change and no field on `FormattedTextHeader`. The fix is one pure function applied to both sides of the comparison inside `find_matching_header` (`app/src/notebooks/editor/model.rs:1351`).

**The slug algorithm is an allow-list: it preserves Unicode letters, digits, marks, and connector punctuation, and strips everything else.** GitHub's heading slugger is not part of `cmark-gfm` — that library emits no heading ids — it lives downstream in GitHub's rendering pipeline, whose behavior the widely-used `github-slugger` reproduces. An ASCII-only `[a-z0-9 -]` filter would silently break every non-English README anchor — a real regression against product invariant 4, not a cosmetic gap. The compatibility target is GitHub parity in both directions: letters survive in every script, and punctuation is removed in every script. This is the single authoritative statement of the algorithm; everything else in this spec refers back to it:

1. Lowercase the input with Unicode-aware lowercasing (Rust's `str::to_lowercase`, not `to_ascii_lowercase`).
2. Keep only characters in these classes, removing every other character outright: Unicode letters (`L*`), marks (`M*`), decimal digits (`Nd`), connector punctuation (`Pc` — the class containing the ASCII underscore `_`), the literal ASCII hyphen `-`, and whitespace (folded to `-` by step 3). The scope is Unicode-category-based, not an ASCII range, in both directions: non-ASCII letters survive on the same footing as ASCII letters, and non-ASCII punctuation is removed on the same footing as ASCII punctuation, so `。` (U+3002), `？` (U+FF1F), `¿` (U+00BF), math symbols such as `+` (`Sm`), and every dash in `Pd` other than the ASCII hyphen (em dash, en dash, fullwidth `－`) are all stripped.
3. Replace runs of whitespace with a single `-`.
4. Trim leading and trailing `-`.

**The allow-list framing is load-bearing, because the two obvious category-exclusion shorthands are both wrong.** "Remove `P*` and `S*` except the ASCII hyphen" over-strips: `_` is `Pc`, and GitHub preserves it. "Remove `P*` and `S*` except hyphens" under-strips: the em dash is `Pd` and GitHub removes it rather than folding it to `-`. Only the allow-list above gets both right, which is why the algorithm is stated as characters kept rather than categories removed.

This is verified against both reproductions and GitHub itself. `github-slugger`'s removal class is generated from the Unicode category tables, and `_` (U+005F) and `-` (U+002D) are deliberate gaps carved out of its ASCII sub-ranges; GitHub's own `html-pipeline` `TableOfContentsFilter` historically used `/[^\p{Word}\- ]/u`, and Ruby's `\p{Word}` is defined as `Alpha` plus `M*` plus `Nd` plus `Pc` — the same allow-list. GitHub's live renderer agrees: `POST /markdown` on `## snake_case_name` returns `id="user-content-snake_case_name"` (underscore preserved), on `## Ma—Dash` returns `user-content-madash` (em dash removed, not folded), on `## a+b` returns `user-content-ab`, and on `## 日本語です。` returns `user-content-日本語です`.

No `user-content-` prefix is applied; see the Security section for why.

Natural home is alongside `find_matching_header` in `model.rs`, or a shared helper if a second caller appears. It should be unit-testable in isolation as `&str -> String`.

Then, inside `find_matching_header`: replace the trailing `to_lowercase()` on the incoming fragment (`model.rs:1357`) with the normalizer, and change the per-heading comparison at `:1374` from `heading.trim().to_lowercase() == target` to compare normalized slugs. Both sides run through the same function, so a hyphenated fragment matches a spaced heading.

First-occurrence-wins for collisions (product invariant 6) falls out of the loop returning on its first hit — no `-1`/`-2` bookkeeping.

### 3. `<a id>`/`<a name>` target resolution

Explicit anchors resolve by the same live walk, with no cached index and no new content-model field.

**The scan reads buffer text, and only text the parser left untokenized survives in it.** This is the load-bearing fact for everything below, so it is stated once here and referred to elsewhere. The resolver reads text out of the buffer — the same access pattern `find_matching_header` already uses to pull heading text via `content.text_in_range(...)` (`app/src/notebooks/editor/model.rs:1371-1373`). That text is **not** the authored source. The buffer stores post-parse content: `text_in_range` resolves through `StyledTextBlock::text()` (`crates/editor/src/content/buffer.rs:5514-5516`), which concatenates the `run: String` field of each surviving `StyledBufferRun` (`buffer.rs:5467-5471`), and no field on a run, block, or style retains a raw source span. Markdown syntax the parser consumes is gone from that text and lives on as structural metadata — which is why `to_markdown` reconstructs `#` prefixes from `BufferBlockStyle::Header` and link brackets from style deltas (`crates/editor/src/content/markdown.rs:200-203`, `:276-289`) rather than replaying retained characters.

The consequence is a hard rule the design must respect: **a source-text scan can only see markup the inline parser declined to tokenize.** (The scan reads that text through `range_to_formatted_text` rather than `text_in_range`, for the code-exclusion reason stated below; both expose the same characters, and only the latter drops the style information the exclusion needs.)

Because the `<a href>` grammar from item 1 requires an `href` attribute to match, a bare `<a id="x"></a>` never becomes an anchor token. Every character of it falls through the inline `alt` chain to `unmatched_char` (`crates/markdown_parser/src/markdown_parser.rs:1536`), which emits the character as literal `InlineToken::Text`, so the full markup lands in the buffer's run text verbatim and the scan sees it. Nothing on the load path HTML-escapes it: `parse_html_entity` (`markdown_parser.rs:1925-1940`) only ever decodes `&…;` and never encodes `<`. Resolution scans that text for anchor-tag markup and extracts the `id`/`name` value, in the same click-time pass that walks headings.

**Dual-role `<a href="…" id="…">` tags register because the parser declines to consume them.** Product invariant 11 requires a tag carrying both roles to render as a link *and* register a jump target. A grammar that consumed the tag could not satisfy both halves, and this is worth stating precisely because it is the one place the obvious design fails:

- If the parser consumed a dual-role opening tag, it would keep only `href`. The value would become `TextStylesWithMetadata.link: Option<String>` (`crates/editor/src/content/text.rs:1091`, assigned at `:1299` via `styles.hyperlink.and_then(Hyperlink::url)`) — a bare URL string on a run, carrying no attribute metadata — and the `id` would survive neither as run text nor as style. The target half would be unrecoverable, and no scan over `text_in_range` could restore it.
- The alternative of teaching the id to ride along as metadata means widening `Hyperlink::Url(String)` (`crates/markdown_parser/src/lib.rs:511-514`) and `TextStylesWithMetadata`, plus both `From` conversions between them (`text.rs:1295-1315`) — a real but avoidable migration, and one that buys nothing the simpler rule below does not.

**So the grammar excludes dual-role tags: item 1's `<a href>` token matches only an opening tag whose attribute list carries neither `id` nor `name`.** A tag carrying both roles fails the anchor grammar and falls through to literal text exactly as a bare `<a id>` does, which keeps the id visible to the scan. This costs no content-model change and no new field, and it makes the source-text premise universally true rather than true-except-one-case.

The cost is that a dual-role tag renders as literal text rather than as a clickable link. That is a deliberate trade against the migration above — it buys the target role, which cannot be recovered any other way without new content-model fields, at the price of the link role, which an author can always spell as a separate ordinary link. Product invariant 11 states the resulting behavior.

Two consequences the implementation must honor. First, the anchor-tag pattern must be **attribute-order independent**: it matches an opening `<a …>` tag and extracts `id` or `name` from anywhere in its attribute list, rather than requiring that attribute to come first. A pattern that binds `id`/`name` directly to `<a` satisfies the simplest bare-anchor case and silently fails whenever any attribute precedes the id, so the test suite pins both attribute orders (see Testing). Second, the grammar's exclusion test and the scan's extraction pattern must agree on what counts as an `id`/`name` attribute; if the grammar consumes a tag the scan would have matched, that tag's target is lost with no diagnostic.

**Canonical fragment representation.** Every caller hands the resolver a fragment in exactly one form: **raw, percent-encoded, with the leading `#` retained** — the form the click path already produces from a link target. The resolver owns both the `#` strip and the single `urlencoding::decode`, as `find_matching_header` does on master (`model.rs:1352-1356`): it requires the `#` (`strip_prefix('#')` returns `None` without it) and decodes exactly once. No caller decodes a fragment it intends to pass on, and no caller strips the `#`. This is the single authoritative statement of the representation; item 5 and the test list refer back to it rather than restating it. A caller that decodes before calling would double-decode — a literal `%2520` would collapse to `%20` and then to a space — and a caller that strips the `#` would make every fragment a silent miss.

Extend `find_matching_header` into a combined resolver (renaming it accordingly, since it will no longer match only headings) that walks the document once and returns the first match of either kind in document order. Per product invariant 6 there is no priority tier: explicit anchor ids are compared **exactly as authored** — no slug normalization, since an author's literal id is not a heading title — while heading text is compared after normalization per item 2. The first of either kind wins.

This keeps the architecture consistent with item 2: no `HashMap<String, CharOffset>`, no invalidation hook, no lifecycle question. The only added per-click cost is a text scan for anchor markup alongside the existing outline walk.

**The scan must exclude code, and parse-time protection does not provide it.** Item 1 notes that the inline parser never sees anchor markup inside code, because `CodeSpan` handling consumes it first. That is a statement about *parsing* and it does not carry over to this scan, which is a separate mechanism running at click time over buffer source text — precisely the text where code content is still present verbatim. Without explicit exclusion, `<a id="x"></a>` written inside a fenced block as documentation of this very feature would become a live jump target, contradicting product invariant 10's rule that code content stays literal.

Exclusion is available from the walk the resolver already performs. `content.outline_blocks()` (`crates/editor/src/content/outline.rs:35`) yields a `BlockOutline` carrying `start`, `end`, and `block_type` (`outline.rs:14-21`), and a fenced block is identifiable as `BlockType::Text(BufferBlockStyle::CodeBlock { .. })` (`crates/editor/src/content/text.rs:367` and `:868`) — the same shape the existing header filter matches on. So the resolver walks blocks once, skipping `CodeBlock` blocks entirely for both the heading comparison and the anchor scan, rather than scanning one flat whole-document string and trying to subtract code ranges afterward.

Inline code spans need separate treatment, because they live *within* a block rather than as their own block: a span is a fragment-level style (`inline_code`, `crates/markdown_parser/src/lib.rs:550`), not a block type. Within a non-code block, an anchor-tag match must therefore be rejected when it falls inside an inline-code span.

**This requires a different buffer API than the heading walk uses, and naming it is load-bearing.** `content.text_in_range(...)` returns style-free flat text — it resolves through `StyledTextBlock::text()`, which concatenates run strings and discards each run's `text_styles` (`crates/editor/src/content/buffer.rs:5514-5516`) — so a resolver holding only that text cannot tell a code span from ordinary prose. The API that does carry the distinction is `Buffer::range_to_formatted_text` (`buffer.rs:2304`), which is public and returns a `FormattedText` whose fragments each carry `styles.inline_code`. The resolver therefore reads anchor-bearing blocks through that call and scans fragment-wise, skipping any fragment with `inline_code` set. The lower-level `styled_blocks_in_range` (`buffer.rs:2708`) is *not* an option: it is `pub(super)` and unreachable from the `app` crate where the resolver lives.

This is the one place where the scan needs structural information rather than raw text, and it is why the scan is specified as a per-block walk over formatted fragments rather than a single document-wide text pass. The heading comparison continues to use `text_in_range` unchanged, since a heading's text needs no style inspection.

The consequence, stated once here and once in product invariant 5: because the tag survives as literal text, it also *renders* as literal text. Hiding it requires the tag to become content-model metadata that re-serializes through `to_markdown` on save — otherwise the next edit silently deletes the author's anchor. Sizing that surfaced a genuine migration across `BufferBlockStyle` variants and `BufferText::BlockMarker` call sites (roughly 70–130 sites across `core.rs`, `edit.rs`, `buffer.rs`, `markdown.rs`, `render/`, and hand-built test fixtures) with no clearly-best representation among the candidates. That is #13982 — a design-discussion ticket, deliberately left unbuilt pending maintainer input on the representation.

### 4. Click resolution

The click branch does not need to be built. `maybe_open_url` (`app/src/notebooks/editor/view.rs:1959`) already routes a leading-`#` URL to `scroll_to_matching_header` (`:1976`, `:1981`), which already requests the autoscroll (`model.rs:1346`). Once item 2 lands, fragments that previously missed begin resolving.

One behavior change is required here. On a miss, `find_matching_header` returns `None`, `scroll_to_matching_header` returns `false`, and `maybe_open_url` currently **falls through to the ordinary URL path** with the `#fragment` string still in hand. Product invariant 7 requires a miss to be observably inert — no broken-link tooltip, no attempt to open `#fragment` externally. The fall-through must therefore be replaced by an early return for `#`-prefixed URLs: a fragment that fails to resolve is a no-op, and never reaches the URL opener. This is the one place correctness is not already guaranteed by existing master behavior, and it should not be left to the implementer to discover by inspection.

### 5. Cross-document fragment navigation

**What already works.** A relative link with no fragment routes through `maybe_open_url` (`view.rs:1959`); not starting with `#`, it reaches `NotebookLinks::resolve_and_open` (`app/src/notebooks/link.rs:322`). `resolve` (`:128`) resolves the path against the session's base directory, `resolve_file` (`:234`) confirms it exists, and `open` (`:257`) emits `LinkEvent::OpenFileNotebook` for a Markdown target. That event is consumed in the notebook pane, re-emitted as `pane_group::Event::OpenFileInWarp` (`app/src/pane_group/mod.rs:550`), and handled by `Workspace::open_file_notebook` (`app/src/workspace/view.rs:8646`), which de-dupes an already-open pane and focuses it, or opens a new tab. Open, focus, and dedup are live; only the fragment is lost.

**The precedent for "open a file and position the viewport" exists** — for the code editor. `LinkTarget::LocalFile` carries `line_and_column` (`link.rs:35`) end to end, and `add_tab_for_code_file` (`view.rs:13032`) threads it through. The Markdown path is the gap: `add_tab_for_file_notebook` (`view.rs:12971`) and `open_file_notebook` (`view.rs:8646`) carry only a path.

Three localized changes:

1. **Split the fragment before file resolution.** In `resolve` (`link.rs:128`), for a target that is neither a parseable URL nor a bare `#fragment`, peel a trailing `#…` off before it reaches `CleanPathResult`, keeping it as an `Option<String>`. Split only on the final `#`, and **retain the `#` and the percent-encoding on the peeled fragment** per item 3's canonical representation — the split removes the fragment from the *path*, it does not normalize the fragment. Treat a fragment that is bare `#` as no anchor. Critically, leave a `#L<digits>[:<digits>]` suffix attached to the path: `CleanPathResult::with_line_and_column_number` (`crates/warp_util/src/path.rs:160`) and its `LINE_AND_COLUMN_REGEX` (`:49`) already route `#L100`-style suffixes to line-number handling, and that must not regress. A bare `#fragment` is still handled earlier by `maybe_open_url`'s `#` branch; this split only affects targets with both a path and a fragment.

2. **Thread the fragment to the destination**, mirroring `line_and_column`: add an anchor field to `LinkTarget::LocalFile` (`link.rs:35`), to `LinkEvent::OpenFileNotebook`, to `pane_group::Event::OpenFileInWarp` (`mod.rs:550`), and through `open_file_notebook` (`view.rs:8646`). Additive plumbing along an existing chain whose shape the code editor already proves.

3. **Apply the scroll after the destination loads.** A same-document jump can scroll immediately; a freshly opened notebook cannot, because the target offset does not exist until parse completes and there is no on-load callback to hang the scroll on. The destination model needs a small piece of deferred state — a pending-anchor field set at construction, drained once on the first successful parse/layout by calling the existing `scroll_to_matching_header` (`model.rs:1335`) and clearing it. The pending anchor is stored and passed in the canonical representation from item 3 (raw, percent-encoded, `#` retained), so the drain call is indistinguishable from a same-document click and the resolver is reused verbatim; only *when* it is called is new. A drain that matches nothing is a no-op, identical to invariant 7.

**Dedup requires canonicalization.** The dedup in `open_file_notebook` compares an open view's path against the resolved link path, but an open notebook stores its *canonical* path while the link resolves to `base_directory.join(relative)` — retaining `.`/`..` components and, on macOS, the `/tmp` versus `/private/tmp` symlink alias. Without normalizing, a self-referential link (`./this-doc.md`) fails to match its own pane and opens a duplicate. Canonicalize the resolved local target with the same mechanism before comparing. A self-link with a fragment then hits the already-open branch and scrolls immediately; without one it simply refocuses. This is product invariant 13's self-reference clause.

**Non-Markdown targets.** With the Markdown Viewer preference off, `open` routes the file to the code editor or system handler, which has no slug concept; the fragment is dropped and the file opens unscrolled, per the product non-goal.

### 6. Resolution repairs (prerequisites)

Cross-document navigation assumes a fragment-less relative link already opens its target. That assumption is false on master in three independent ways, all in `NotebookLinks::resolve`, and all of which break even the plain `[text](other-file.md)` case. These are prerequisites, not polish.

**Repair 1 — ccTLD misclassification of a bare `file.md`.** `resolve` applies a bare-domain heuristic *before* file resolution: it takes the substring up to the first `/` and, if `addr::parse_domain_name` (`link.rs:154`) reports a known public suffix with a root, treats the whole target as `http://…`. Because `.md` is Moldova's ccTLD, a bare `README.md` or `other-file.md` — no `./`, no `/` — is classified as a domain and opened in the browser instead of the viewer. The `./` prefix and multi-segment paths dodge the heuristic, which is why existing viewer tests spell their links `./README.md`.

*Fix:* before applying the heuristic, check whether the scheme-less target resolves to an existing file relative to the base directory, reusing the existing existence check. If it does, resolve as a file; only fall through to the domain heuristic when no local file matches. `warp.dev` still opens the browser, and a bare `nonexistent.md` with nothing on disk still does. The rule is deterministic and file-existence-gated. This is product invariant 13's requirement that a real local file wins over the domain reading.

**Repair 2 — a literal `#fragment` breaks the file stat.** `LINE_AND_COLUMN_REGEX` (`path.rs:49`) strips `:line:col` and `#L100` suffixes but not a bare `#section`, so `other-file.md#section` reaches `resolve_file` as a literal on-disk path, misses, and the error is dropped. Item 5's fragment split fixes this and must run *after* the explicit-URL branch, so a genuine `https://…#frag` keeps its fragment.

**Repair 3 — a standalone viewer tab lacks a base directory.** A link clicked inside an open notebook gets the right base directory from its session source. The gap is a standalone tab opened with no session (`open -a Warp file.md`): with no session, the active-session fallback has no cwd, so even `./file.md` fails to resolve. *Fix:* let the session source carry an optional document directory, preferring the active session's cwd and falling back to the document's own parent. The document knows where it lives even when the window does not.

A residual worth naming: `resolve_and_open` swallows resolution errors, so a genuinely session-less tab that still cannot resolve fails silently. Surfacing that non-silently is a follow-up — no cheap existing affordance sits on this path, and this slice does not build new UI.

### 7. Feature gating

Recommend a new flag (e.g. `FeatureFlag::MarkdownAnchorLinks`) rather than riding an existing one, since there is no existing "structural HTML" flag this naturally extends.

**The flag gates internal rollout only; it does not license partial user-visible shipment.** The product spec's delivery-scope section is authoritative: all sixteen invariants ship together as one deliverable. The flag's purpose is the ordinary one of landing and enabling the work safely — merging behind it, dogfooding internally, and turning it on once complete — not carving the feature into user-visible stages. A build in which the flag is on shows the whole feature, and no shipping configuration exposes some invariants while withholding others.

### 8. Security

`<a href>` reuses `Hyperlink::Url` verbatim — no new trust boundary. Only `href`, `id`, and `name` are read; every other attribute is parsed and discarded per product invariant 8, so `onclick` and other event handlers are inert text, never executed. Scheme handling is inherited unchanged from the markdown link path (product invariant 12): this feature adds no scheme-specific capability and no new opener, so a `javascript:` or `data:` target in an `<a href>` behaves exactly as the same target in `[text](javascript:…)` behaves today. If that inherited behavior is judged insufficient, the fix belongs in the shared link path where it protects both syntaxes — not in the anchor parser, where it would protect one and leave the other open.

Fragment resolution never leaves the document: a `#fragment` click either scrolls the current buffer or is a no-op — no network, no filesystem access. Explicit `<a id>`/`<a name>` values are used only as in-document lookup keys, never interpolated into a URL, path, or shell context.

**On GitHub's `user-content-` prefix, intentionally not replicated.** GitHub prefixes the DOM ids it emits for headings and anchors with `user-content-` to avoid collisions with its own page chrome, while leaving author-written `href="#…"` fragments unprefixed, bridging the two in client JS. Warp has no DOM and no such collision surface: resolution happens in process against slugs computed live from heading text. Adding the prefix would only break parity with the plain `#slug` fragments authors write. Stated explicitly so a reviewer familiar with GitHub's scheme does not read its absence as a bug.

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<a href="https://warp.dev">Visit Warp</a>` produces a `Hyperlink::Url` fragment identical in shape to the equivalent markdown link (invariant 1).
- `<a href="#target">Jump</a>` produces `Hyperlink::Url("#target")` (invariant 2) — parsing only; resolution is covered below.
- Attributes beyond `href` (`title`, `target="_blank"`, `class="x"`, `onclick="…"`) are parsed and ignored with no effect on output (invariants 8, 12).
- Rich inline content inside anchor text keeps both its own styling and the hyperlink (invariant 9): an `<a href>` wrapping `**bold**` and a code span produces bold and inline-code fragments that each also carry `Hyperlink::Url`.
- Malformed markup degrades to literal text without consuming the rest of the paragraph, one case per clause of invariant 10: unterminated `<a`, missing `</a>`, empty/valueless `href`, unbalanced quoting, and nested `<a>` (outer wins, inner is literal).
- Anchor markup inside an inline code span and inside a fenced code block stays literal (invariant 10's code clause).
- A bare `<a id="x"></a>` parses to a single plain-text fragment retaining every authored character including the angle brackets — the characterization that item 3's resolution strategy depends on, and the assertion that would catch a future grammar change silently swallowing the tag. The self-closing form `<a id="x" />` gets the same assertion, since product invariant 5 scopes it explicitly.
- A dual-role `<a>` carrying both `href` and `id` is **excluded from the anchor grammar** and parses to plain text with its id characters intact, asserted in **both attribute orders** (`href` first and `id` first) so the exclusion cannot be order-dependent. This is the parse-side half of item 3's mechanism; the target it registers is asserted in the resolution tests. A `name` attribute triggers the same exclusion as `id`.
- Exclusion is scoped to `id`/`name` only: an `<a href>` carrying unrelated attributes (`title`, `class`, `target`) still parses as a link, confirming the exclusion did not widen into a general attribute rejection.
- Slug normalizer (item 2), table-driven as a pure `&str -> String`: plain ASCII; mixed case and ASCII punctuation; multi-space runs; leading/trailing whitespace; **accented Latin** (`"Café Société"` → `café-société`); **CJK** (`"日本語"` → `日本語`, unchanged); and a **mixed-script** heading (`"Section 日本語 Café"` → `section-日本語-café`). These letter cases fail if an ASCII-only filter is reintroduced.
- Non-ASCII **punctuation** removal, the other half of the rule and the half a letters-only table misses: `"日本語です。"` → `日本語です` (ideographic full stop U+3002 removed); `"Café？"` → `café` (fullwidth question mark U+FF1F removed, matching ASCII `?`); `"Español ¿Qué?"` → `español-qué`; `"a+b"` → `ab` (math symbol removed); and `"Ma—Dash"` → `madash` (em dash removed entirely, **not** converted to `-`, while a literal ASCII `-` in `"Well-Known"` → `well-known` survives). These fail if the exclusion set is written as an ASCII range.
- **Connector punctuation survives**, the case that discriminates the allow-list from a blanket `P*`/`S*` strip: `"snake_case_name"` → `snake_case_name` and `"Hello_World Test"` → `hello_world-test`, both matching GitHub's live output verbatim. An implementation that removes all `P*` categories passes every other case in this table and fails only these two.

### Resolution tests (`app/src/notebooks/editor/`)

- Heading `## Target Section` plus fragment `#target-section` resolves to that heading's range. On master this returns `None`; this is the core regression the normalizer fixes (invariants 2, 3, 4).
- A non-English heading (`## Café Société`) plus `#café-société` resolves, exercising the Unicode normalizer through the matcher rather than only in isolation.
- `<a href="#slug">` and `[text](#slug)` targeting the same heading resolve to the same range (invariant 3).
- A fragment matching nothing returns `None`, `scroll_to_matching_header` returns `false`, and `maybe_open_url` **early-returns without invoking the URL opener** — the item 4 behavior change, and the direct test of invariant 7's "no broken-link tooltip" clause.
- First-occurrence-wins, one test per collision shape in invariant 6: two headings sharing a slug; two `<a id>` values sharing an id; an `<a id>` and a heading slug colliding, asserting the earlier-in-document one resolves regardless of which kind it is.
- An explicit `<a id="x"></a>` with no colliding heading resolves via the anchor scan, independent of the heading walk. The self-closing form `<a id="x" />` and the `<a name="x"></a>` spelling each resolve identically, covering both forms product invariant 5 scopes.
- Anchor ids match exactly as authored: an id with uppercase or punctuation resolves only to the exact fragment, confirming heading normalization is not applied to explicit ids (item 3).
- **Dual-role tags register a target in both attribute orders** (invariant 11): `<a href="#other" id="x">text</a>` and `<a id="x" href="#other">text</a>` each make `#x` resolve to that tag. The href-first case is the one an id-anchored scan pattern silently misses, so both orders are required rather than representative. Both also assert the tag did **not** become a link, pinning item 3's grammar exclusion from the resolution side.
- **Explicit anchors inside code do not register** (invariant 10): `<a id="x"></a>` inside a fenced code block, and inside an inline code span, each leave `#x` unresolved — a miss per invariant 7, not a jump target. The fenced case guards the block-level exclusion and the span case guards the fragment-level one; they exercise different mechanisms and neither substitutes for the other.
- A heading inside a fenced code block (a `#`-prefixed line that is code, not a heading) does not become addressable, confirming the code exclusion applies to the heading walk as well as the anchor scan.
- **Anchors inside other block constructs** (invariant 15): an `<a href="#slug">` inside a list item and inside a blockquote renders and resolves normally, and an `<a id>` placed inside each registers its target — confirming anchor handling rides ordinary inline content and is not restricted to top-level paragraphs. The table-cell case is deferred to the joint verification with #13726 noted under Risks, since cell content is not parsed through the inline path until that spec lands.
- **Scheme handling matches the markdown equivalent** (invariant 12): an `<a href="javascript:…">` and a `[text](javascript:…)` targeting the same URL produce identical click outcomes, asserted as a pair so the test pins parity rather than a specific scheme policy that the shared link path may change independently. The same pairing covers `data:` and `file:`.
- Editing the document changes what resolves on the next click — rename a heading and assert the old slug becomes a miss and the new one resolves (invariant 16, and the assertion that no cache was introduced).

### Cross-document and resolution-repair tests (`app/src/notebooks/link.rs`, workspace tests)

- Fragment split: `other-file.md#section` yields the cleaned path plus anchor `#section` — **`#`-prefixed and still percent-encoded**, per item 3's canonical representation; `other-file.md` yields no anchor; `other-file.md#L10` still routes to line-number handling, guarding repair 2 against regressing `#L` suffixes. Pure-function coverage for multiple `#`, a bare trailing `#`, and `#L` versus `#License`.
- Encoding round-trip across the seam: `other-file.md#caf%C3%A9-soci%C3%A9t%C3%A9` resolves to the `## Café Société` heading in the opened document, asserting the fragment is decoded exactly once — by the resolver, not by the split. A fragment containing a literal encoded percent (`#a%2520b`) resolves to the anchor `a%20b` rather than `a b`, which is the assertion that fails if any caller decodes before handing the fragment on.
- ccTLD matrix (repair 1): a bare `README.md` that exists on disk resolves as a file; `warp.dev` and a bare `nonexistent.md` with no local file still resolve as URLs. An existing viewer test currently spelling its link `./README.md` should be switched to the bare form to assert the repair end to end.
- A resolved local-file target for a `#section` link carries the anchor through to the open event (item 5, step 2).
- Deferred-scroll drain: a pending anchor set before layout resolves through the same matcher on drain — hit scrolls and clears one-shot; miss is a silent no-op (invariant 13).
- Self-referential dedup: `./this-doc.md#section` from within that document focuses the existing tab and scrolls rather than opening a duplicate, covering `.`/`..` and the macOS `/tmp` symlink alias.

### TUI

- An `<a href>` link renders with link styling in the TUI through `inline_spans` (`tui_markdown.rs:228`) with no TUI-specific code, and a `#fragment` there performs no navigation (invariant 14).

### Integration / manual

Per CONTRIBUTING, before/after screenshots plus a short recording reproducing the issue's motivating document verbatim: the raw `<a href="#target-section">` jump; the markdown-native `[Jump to Target Section](#target-section)` jump (the contrast case that resolves as a plain URL today); the external `<a href="https://warp.dev">` link; and the `<a id="target-section"></a>` marker preceding a heading — which will be **visibly present** in the render per invariant 5, and should be shown rather than cropped, since it is the known shipped limitation.

Cross-document: clicking `[text](other-file.md#section)` opens or focuses that file and lands on its heading after load; a bare `[text](other-file.md)` opens in the viewer rather than the browser (the ccTLD repair); re-clicking an already-open target focuses and re-scrolls without duplicating the tab; a `#section` with no match shows the file unscrolled with no error.

Confirm the scroll lands the target readably. The existing call uses `PositionOffsetInViewportCenter` (`model.rs:1346`), so the target is centered rather than top-aligned. This is already the shipped behavior for other `#`-fragment jumps and changing it would affect them too — so it is a sanity check, not a change this spec proposes.

## Risks and follow-ups

- **The miss path is the one correctness item not already guaranteed by master.** Item 4 changes a fall-through into an early return. If it is skipped, invariant 7 fails in a user-visible way (a broken-link tooltip on every unresolved fragment) while every other test in this spec still passes.
- **The attribute-parser choice (item 1) needs a maintainer call** between a purpose-built scanner and reusing `html5ever`. Both are viable; `html5ever` is already a crate dependency, so the tradeoff is robustness against grammar simplicity, not dependency cost.
- **The visible anchor tag (#13982) is a shipped, deliberate limitation**, not an oversight. It is the direct cost of item 3's no-content-model-change decision. If maintainers prefer the tag hidden in this slice, that decision reverses item 3 and materially resizes the work.
- **Interaction with the HTML-table spec (#13726):** an `<a href>` inside a table cell should work automatically once cell content is parsed through the same inline path, which that spec already plans to reuse. Worth verifying once both land; no design change anticipated.
- **Silent resolution failures (follow-up).** `resolve_and_open` swallows resolution errors, so a session-less standalone tab that cannot resolve a file fails silently. Surfacing it is deferred, since no cheap existing affordance sits on this path.
