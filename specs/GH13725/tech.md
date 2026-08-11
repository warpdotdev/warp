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
- **(ii) `<a id>`/`<a name>` targets: SMALL.** No new content-model field. The `<a href>` grammar requires an `href` to match at all, so a bare `<a id="x">` falls through the inline `alt` chain to plain text and its markup — including the id value — survives verbatim in the buffer. Resolution reuses the same live-text-walk pattern the heading path already establishes. The cost of this choice is that the tag stays visible (product invariant 5); hiding it needs a save-round-trippable representation and is deferred to #13982.
- **(iii) Heading slug matching + click resolution: SMALL.** The click branch, the scroll call, and the per-click heading walk all exist. The work is a slug normalizer and one comparison change.
- **(iv) Cross-document fragment navigation: MEDIUM.** The file-open, tab-focus, and dedup machinery exists, but three latent resolution defects (item 6) block even a fragment-less bare `file.md` link, and the deferred scroll after load is genuinely new state.

## Proposed changes

### 1. `<a href="…">text</a>` inline token

Add HTML-anchor tokens to `InlineToken` (`markdown_parser.rs:1731`), following the `<u>` precedent in shape but carrying data. Unlike `<u>`'s zero-data delimiter, the start token must thread the `href` value through to close time — either as its own variant (`HtmlAnchorStart(String)` paired with `HtmlAnchorEnd`) or by stashing the value on the delimiter-stack entry, mirroring how `parse_link` stashes `link_start.node_index` at `:1169`.

Closing `</a>` applies `styles.hyperlink = Some(Hyperlink::Url(href))` to the fragments between start and end — the same `backtrack_styles` assignment `parse_link` makes at `:1202`, triggered by `</a>` instead of `](url)`.

The opening tag needs a minimal attribute scanner: extract `href` (single- or double-quoted), and tolerate-and-discard every other attribute per product invariant 8. **This is a decision for maintainer review.** A purpose-built `key="value"` scanner matches the `<u>` precedent's spirit and keeps the inline grammar self-contained, but is more exposed to malformed real-world markup. `html5ever` is already a dependency of this crate (`crates/markdown_parser/Cargo.toml:16`, used by the separate paste-path parser in `html_parser.rs`), so reusing it carries no new-dependency cost — only the cost of pulling a full HTML tokenizer into a single-tag inline case. The spec does not pick for you; both are viable and the tradeoff is robustness against grammar simplicity.

Malformed input falls back to literal text for the tag, matching how `parse_link` falls back to a literal `]` on failure. The deterministic cases product invariant 10 enumerates map onto the parser as follows: an unterminated `<a` fails the tag parse and emits literal text; a missing `</a>` leaves the start delimiter unmatched on the stack and it is emitted as literal text at paragraph end, exactly as an unmatched markdown delimiter is; an empty or valueless `href` fails the attribute scan and the whole tag degrades to literal text; nested `<a>` resolves by the outer tag pairing with the first `</a>`, leaving the inner opening tag as literal text. Because the anchor parser is registered in the inline `alt` chain alongside the other inline tokens, code spans and fenced blocks never reach it — they are consumed by the existing `CodeSpan` handling first — which is what makes invariant 10's code-block clause true without special-casing.

### 2. Heading slug normalization

No parse-time change and no field on `FormattedTextHeader`. The fix is one pure function applied to both sides of the comparison inside `find_matching_header` (`app/src/notebooks/editor/model.rs:1351`).

**The slug algorithm is Unicode-preserving, not ASCII-only.** GitHub's heading slugger (`gfm-auto-identifiers`, the rule set `github/cmark-gfm` implements) does not strip non-ASCII text. An ASCII-only `[a-z0-9 -]` filter would silently break every non-English README anchor — a real regression against product invariant 4, not a cosmetic gap. This is the single authoritative statement of the algorithm; everything else in this spec refers back to it:

1. Lowercase the input with Unicode-aware lowercasing (Rust's `str::to_lowercase`, not `to_ascii_lowercase`).
2. Remove characters that are ASCII punctuation or symbols other than `-`, mirroring GFM's exclusion set (roughly ``!"#$%&'()*+,./:;<=>?@[\]^`{|}~``), while leaving all other Unicode letters, digits, and marks untouched. The exclusion is defined by punctuation class, not by ASCII range — this is the one place a naive reading goes wrong.
3. Replace runs of whitespace with a single `-`.
4. Trim leading and trailing `-`.

No `user-content-` prefix is applied; see the Security section for why.

Natural home is alongside `find_matching_header` in `model.rs`, or a shared helper if a second caller appears. It should be unit-testable in isolation as `&str -> String`.

Then, inside `find_matching_header`: replace the trailing `to_lowercase()` on the incoming fragment (`model.rs:1357`) with the normalizer, and change the per-heading comparison at `:1374` from `heading.trim().to_lowercase() == target` to compare normalized slugs. Both sides run through the same function, so a hyphenated fragment matches a spaced heading.

First-occurrence-wins for collisions (product invariant 6) falls out of the loop returning on its first hit — no `-1`/`-2` bookkeeping.

### 3. `<a id>`/`<a name>` target resolution

Explicit anchors resolve by the same live walk, with no cached index and no new content-model field.

Because the `<a href>` grammar from item 1 requires an `href` attribute to match, a bare `<a id="x"></a>` never becomes an anchor token. It falls through to plain text and its raw markup survives verbatim in the buffer. Resolution therefore scans the document's live text for anchor-tag markup and extracts the `id`/`name` value, in the same click-time pass that walks headings.

Extend `find_matching_header` into a combined resolver (renaming it accordingly, since it will no longer match only headings) that walks the document once and returns the first match of either kind in document order. Per product invariant 6 there is no priority tier: explicit anchor ids are compared **exactly as authored** — no slug normalization, since an author's literal id is not a heading title — while heading text is compared after normalization per item 2. The first of either kind wins.

This keeps the architecture consistent with item 2: no `HashMap<String, CharOffset>`, no invalidation hook, no lifecycle question. The only added per-click cost is a text scan for anchor markup alongside the existing outline walk.

The consequence, stated once here and once in product invariant 5: because the tag survives as literal text, it also *renders* as literal text. Hiding it requires the tag to become content-model metadata that re-serializes through `to_markdown` on save — otherwise the next edit silently deletes the author's anchor. Sizing that surfaced a genuine migration across `BufferBlockStyle` variants and `BufferText::BlockMarker` call sites (roughly 70–130 sites across `core.rs`, `edit.rs`, `buffer.rs`, `markdown.rs`, `render/`, and hand-built test fixtures) with no clearly-best representation among the candidates. That is #13982 — a design-discussion ticket, deliberately left unbuilt pending maintainer input on the representation.

### 4. Click resolution

The click branch does not need to be built. `maybe_open_url` (`app/src/notebooks/editor/view.rs:1959`) already routes a leading-`#` URL to `scroll_to_matching_header` (`:1976`, `:1981`), which already requests the autoscroll (`model.rs:1346`). Once item 2 lands, fragments that previously missed begin resolving.

One behavior change is required here. On a miss, `find_matching_header` returns `None`, `scroll_to_matching_header` returns `false`, and `maybe_open_url` currently **falls through to the ordinary URL path** with the `#fragment` string still in hand. Product invariant 7 requires a miss to be observably inert — no broken-link tooltip, no attempt to open `#fragment` externally. The fall-through must therefore be replaced by an early return for `#`-prefixed URLs: a fragment that fails to resolve is a no-op, and never reaches the URL opener. This is the one place phase-1 correctness is not already guaranteed by existing master behavior, and it should not be left to the implementer to discover by inspection.

### 5. Cross-document fragment navigation

**What already works.** A relative link with no fragment routes through `maybe_open_url` (`view.rs:1959`); not starting with `#`, it reaches `NotebookLinks::resolve_and_open` (`app/src/notebooks/link.rs:322`). `resolve` (`:128`) resolves the path against the session's base directory, `resolve_file` (`:234`) confirms it exists, and `open` (`:257`) emits `LinkEvent::OpenFileNotebook` for a Markdown target. That event is consumed in the notebook pane, re-emitted as `pane_group::Event::OpenFileInWarp` (`app/src/pane_group/mod.rs:550`), and handled by `Workspace::open_file_notebook` (`app/src/workspace/view.rs:8646`), which de-dupes an already-open pane and focuses it, or opens a new tab. Open, focus, and dedup are live; only the fragment is lost.

**The precedent for "open a file and position the viewport" exists** — for the code editor. `LinkTarget::LocalFile` carries `line_and_column` (`link.rs:35`) end to end, and `add_tab_for_code_file` (`view.rs:13032`) threads it through. The Markdown path is the gap: `add_tab_for_file_notebook` (`view.rs:12971`) and `open_file_notebook` (`view.rs:8646`) carry only a path.

Three localized changes:

1. **Split the fragment before file resolution.** In `resolve` (`link.rs:128`), for a target that is neither a parseable URL nor a bare `#fragment`, peel a trailing `#…` off before it reaches `CleanPathResult`, keeping it as an `Option<String>`. Split only on the final `#`, decode with `urlencoding`, and treat an empty fragment as no anchor. Critically, leave a `#L<digits>[:<digits>]` suffix attached to the path: `CleanPathResult::with_line_and_column_number` (`crates/warp_util/src/path.rs:160`) and its `LINE_AND_COLUMN_REGEX` (`:49`) already route `#L100`-style suffixes to line-number handling, and that must not regress. A bare `#fragment` is still handled earlier by `maybe_open_url`'s `#` branch; this split only affects targets with both a path and a fragment.

2. **Thread the fragment to the destination**, mirroring `line_and_column`: add an anchor field to `LinkTarget::LocalFile` (`link.rs:35`), to `LinkEvent::OpenFileNotebook`, to `pane_group::Event::OpenFileInWarp` (`mod.rs:550`), and through `open_file_notebook` (`view.rs:8646`). Additive plumbing along an existing chain whose shape the code editor already proves.

3. **Apply the scroll after the destination loads.** A same-document jump can scroll immediately; a freshly opened notebook cannot, because the target offset does not exist until parse completes and there is no on-load callback to hang the scroll on. The destination model needs a small piece of deferred state — a pending-anchor field set at construction, drained once on the first successful parse/layout by calling the existing `scroll_to_matching_header` (`model.rs:1335`) and clearing it. The resolver is reused verbatim; only *when* it is called is new. A drain that matches nothing is a no-op, identical to invariant 7.

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

Recommend a new flag (e.g. `FeatureFlag::MarkdownAnchorLinks`) rather than riding an existing one. There is no existing "structural HTML" flag this naturally extends, and a dedicated flag lets the `<a href>` and heading-slug work ship independently of the cross-document path if their costs diverge during implementation.

### 8. Security

`<a href>` reuses `Hyperlink::Url` verbatim — no new trust boundary. Only `href`, `id`, and `name` are read; every other attribute is parsed and discarded per product invariant 8, so `onclick` and other event handlers are inert text, never executed. Scheme handling is inherited unchanged from the markdown link path (product invariant 12): this feature adds no scheme-specific capability and no new opener, so a `javascript:` or `data:` target in an `<a href>` behaves exactly as the same target in `[text](javascript:…)` behaves today. If that inherited behavior is judged insufficient, the fix belongs in the shared link path where it protects both syntaxes — not in the anchor parser, where it would protect one and leave the other open.

Fragment resolution never leaves the document: a `#fragment` click either scrolls the current buffer or is a no-op — no network, no filesystem access. Explicit `<a id>`/`<a name>` values are used only as in-document lookup keys, never interpolated into a URL, path, or shell context.

**On GitHub's `user-content-` prefix, intentionally not replicated.** GitHub prefixes the DOM ids it emits for headings and anchors with `user-content-` to avoid collisions with its own page chrome, while leaving author-written `href="#…"` fragments unprefixed, bridging the two in client JS. Warp has no DOM and no such collision surface: resolution happens in process against slugs computed live from heading text. Adding the prefix would only break parity with the plain `#slug` fragments authors write. Stated explicitly so a reviewer familiar with GitHub's scheme does not read its absence as a bug.

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<a href="https://warp.dev">Visit Warp</a>` produces a `Hyperlink::Url` fragment identical in shape to the equivalent markdown link (invariant 1).
- `<a href="#target">Jump</a>` produces `Hyperlink::Url("#target")` (invariant 2) — parsing only; resolution is covered below.
- Attributes beyond `href` (`title`, `target="_blank"`, `class="x"`, `onclick="…"`) are parsed and ignored with no effect on output (invariants 8, 12).
- Malformed markup degrades to literal text without consuming the rest of the paragraph, one case per clause of invariant 10: unterminated `<a`, missing `</a>`, empty/valueless `href`, unbalanced quoting, and nested `<a>` (outer wins, inner is literal).
- Anchor markup inside an inline code span and inside a fenced code block stays literal (invariant 10's code clause).
- A bare `<a id="x"></a>` parses to a single plain-text fragment — the characterization that item 3's resolution strategy depends on, and the assertion that would catch a future grammar change silently swallowing the tag.
- An `<a>` carrying both `href` and `id` renders as a link and does not panic (invariant 11).
- Slug normalizer (item 2), table-driven as a pure `&str -> String`: plain ASCII; mixed case and punctuation; multi-space runs; leading/trailing whitespace; **accented Latin** (`"Café Société"` → `café-société`); **CJK** (`"日本語"` → `日本語`, unchanged); and a **mixed-script** heading (`"Section 日本語 Café"` → `section-日本語-café`). The Unicode cases exist specifically to fail if an ASCII-only filter is reintroduced.

### Resolution tests (`app/src/notebooks/editor/`)

- Heading `## Target Section` plus fragment `#target-section` resolves to that heading's range. On master this returns `None`; this is the core regression the normalizer fixes (invariants 2, 3, 4).
- A non-English heading (`## Café Société`) plus `#café-société` resolves, exercising the Unicode normalizer through the matcher rather than only in isolation.
- `<a href="#slug">` and `[text](#slug)` targeting the same heading resolve to the same range (invariant 3).
- A fragment matching nothing returns `None`, `scroll_to_matching_header` returns `false`, and `maybe_open_url` **early-returns without invoking the URL opener** — the item 4 behavior change, and the direct test of invariant 7's "no broken-link tooltip" clause.
- First-occurrence-wins, one test per collision shape in invariant 6: two headings sharing a slug; two `<a id>` values sharing an id; an `<a id>` and a heading slug colliding, asserting the earlier-in-document one resolves regardless of which kind it is.
- An explicit `<a id="x"></a>` with no colliding heading resolves via the anchor scan, independent of the heading walk.
- Anchor ids match exactly as authored: an id with uppercase or punctuation resolves only to the exact fragment, confirming heading normalization is not applied to explicit ids (item 3).
- Editing the document changes what resolves on the next click — rename a heading and assert the old slug becomes a miss and the new one resolves (invariant 16, and the assertion that no cache was introduced).

### Cross-document and resolution-repair tests (`app/src/notebooks/link.rs`, workspace tests)

- Fragment split: `other-file.md#section` yields the cleaned path plus anchor `section`; `other-file.md` yields no anchor; `other-file.md#L10` still routes to line-number handling, guarding repair 2 against regressing `#L` suffixes. Pure-function coverage for multiple `#`, empty fragment, URL-decoding, and `#L` versus `#License`.
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
