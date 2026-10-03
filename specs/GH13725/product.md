# PRODUCT.md — Markdown viewer: `<a href>`/`<a id>` anchor links

Issue: https://github.com/warpdotdev/warp/issues/13725

Split from #13652 (bulk raw-HTML-subset request, closed in favor of per-feature issues). Sibling specs in the same split: raw HTML tables (#13726), `<img>` sizing (#13721), `<details>`/`<summary>` (#10259), `<br>` (#13732), `<kbd>` (#13733), `<sub>`/`<sup>` (#13734), `align` (#13735), `<picture>`/`<source>` (#13736).

## Summary

Hand-written READMEs and docs routinely build a table of contents out of two halves: `<a id="section">` (or `name="section"`) targets, and `[Jump to Section](#section)` or `<a href="#section">` links pointing at them. Warp's Markdown viewer supports neither half today.

Raw `<a>` tags render as literal text — the inline parser has no HTML-tag concept beyond `<u>`. And the markdown-native form `[text](#fragment)`, which *does* already parse as a link, has nothing to resolve against: the viewer compares a fragment against a heading's exact lowercased text, so the hyphenated `#target-section` never matches a heading reading `Target Section`. The headline symptom is that the most common anchor link in real-world Markdown silently does nothing.

This spec covers two related capabilities:

1. **Raw HTML anchor tags** — recognize inline `<a href="…">…</a>` as a hyperlink, and `<a id="…">`/`<a name="…">` as a named jump target.
2. **Fragment resolution and scroll-to** — give every heading an implicit GitHub-style slug, let explicit `<a id>`/`<a name>` register targets in the same namespace, and make any `#fragment` link scroll the viewer to its target rather than falling through to plain-URL handling.

Capability 2 is the one that fixes the issue's headline case, and it benefits markdown-native links that gain nothing from HTML tag parsing at all. The issue frames working in-document anchors as a prerequisite for a future table-of-contents feature (#13083, #4720).

Figma: none provided.

## Goals / Non-goals

In scope:

- Parse inline `<a href="…">link text</a>` as a hyperlink — styled, clickable, and behaviorally identical to the markdown link `[link text](…)` — for both external URLs (`https://…`) and in-page fragments (`#target`).
- Parse `<a id="…"></a>`, `<a name="…"></a>`, and the self-closing `<a id="…" />` — the forms authors actually write — as a named jump target at that point in the document.
- Give every heading (`#`…`######`) an implicit anchor slug derived from its rendered text, so `[Jump to Target Section](#target-section)` works against an ordinary heading with zero authoring effort.
- Resolve a `#fragment` click — from either syntax — against the document's anchors (explicit targets and implicit heading slugs) and scroll so the target is visible.
- Resolve a combined path-and-fragment link (`other-file.md#section`) by opening or focusing that file's viewer tab and scrolling it to the anchor once loaded.
- Degrade a `#fragment` with no matching anchor to an inert click: no navigation, no error, no crash.
- Accept and ignore `<a>` attributes beyond `href`/`id`/`name` (`title`, `target`, `rel`, `class`, `style`, …) — they neither break the parse nor do anything.

Out of scope (explicit non-goals):

- **Hiding the rendered anchor tag.** A bare `<a id="x"></a>` or self-closing `<a id="x" />` remains *visible as literal text* rather than rendering as nothing the way browsers and GitHub do. Invariant 5 states this fully; dual-role tags render as links and are not affected. Hiding a bare tag requires a first-class content-model representation that survives `to_markdown` on save, tracked as #13982.
- **Following a cross-document fragment into a non-Markdown target or an external editor.** The cross-document scroll applies only when the target opens in Warp's Markdown viewer. With the Markdown Viewer preference off, the file opens in the code editor or an external app and the fragment is dropped — matching how a plain relative file link behaves today. Bridging a `#slug` to a code-editor line is a separate concern.
- **Fragment scroll in the terminal (TUI) Markdown renderer.** `<a href>` links *render* in the TUI for free because it shares the parser, but clicking a `#fragment` there does not scroll: the TUI has no scroll model and no click-to-navigate path. Invariant 14 states the TUI's behavior in full.
- **GitHub's duplicate-slug disambiguation.** GitHub appends `-1`, `-2`, … to colliding slugs. Warp resolves collisions by first occurrence in document order (invariant 6); replicating the numeric suffix scheme is not required.
- **Any other raw-HTML tag** (`<img>`, `<table>`, `<details>`, …) — each has its own spec in this split. Anchors inside Markdown's own pipe-table cells are in scope per invariant 15; only raw-HTML `<table>` markup waits on #13726.
- **Blockquote-specific behavior.** The viewer has no blockquote construct — a `>`-prefixed line is an ordinary paragraph — so anchors in one work for that reason rather than through any blockquote handling. Nothing here adds a blockquote model.
- **Editing and authoring affordances** — no "copy anchor link" UI, no auto-slug preview while typing. This is a rendering and navigation feature only.
- **A new URL-scheme trust boundary.** `<a href>` reuses the same link representation and click path markdown links already use, inheriting that boundary unchanged rather than introducing a second one. Invariant 12 states which schemes act and which are inert.

## Behavior

1. `<a href="https://warp.dev">Visit Warp</a>` renders as a clickable link reading "Visit Warp", visually and behaviorally identical to the markdown link `[Visit Warp](https://warp.dev)`.

2. `<a href="#target-section">Jump to Target Section</a>` renders as a clickable link; clicking it scrolls the viewer so the anchor matching `target-section` is visible. This is the HTML-tag half of the issue's test case.

3. The markdown-native equivalent `[Jump to Target Section](#target-section)` gets the **same** click behavior as invariant 2. It already parses as a link today; only resolution is new. This is the contrasting case the issue names explicitly as "resolves as a plain URL hyperlink" today.

4. A heading `## Target Section` is addressable as `#target-section` with no authoring effort and no `<a id>` required. The slug is derived from the heading's rendered text by the algorithm the tech spec defines. That algorithm treats every script alike: letters, digits, and marks are preserved, so `## Café Société` is addressable as `#café-société` and `## 日本語` as `#日本語`, while punctuation and symbols are removed whether they are ASCII or not, so `## 日本語です。` is addressable as `#日本語です` exactly as `## Target Section?` is addressable as `#target-section`. Two characters are deliberate exceptions to the punctuation rule, both matching GitHub: the ASCII hyphen and the ASCII underscore survive, so `## snake_case_name` is addressable as `#snake_case_name`. Non-English headings are a supported case, not a degraded one.

5. `<a id="target-section"></a>` or `<a name="target-section"></a>`, placed anywhere in the document, registers `target-section` as a jump target. The self-closing spelling `<a id="target-section" />` registers identically; both forms appear in real-world documents and neither is preferred.

   **A bare or self-closing tag renders as visible literal text.** Browsers and GitHub render a content-less anchor as nothing; Warp does not replicate that in this slice. The tag appears inline exactly as authored. Resolution works precisely *because* the tag stays literal: it reads the document's text, where an untokenized tag survives intact. This limitation is confined to anchors with no `href` — a dual-role tag renders as its link text and is unaffected (invariant 11). Making a bare tag disappear requires representing it as content-model metadata that still re-serializes on save — otherwise editing the document silently deletes the author's anchor — which is a genuine model change, deferred to #13982 so maintainers can weigh the representation before it is built.

6. Explicit anchors and implicit heading slugs share **one namespace**, resolved by **first occurrence in document order**. This single rule covers every collision case: two headings with the same slug, two `<a id>` values that are identical, an `<a id>` colliding with a heading slug, or a bare anchor colliding with a dual-role tag's id. There is no priority tier of any kind — not anchor-versus-heading, and not bare-versus-dual-role — whichever appears first in the document wins, matching GitHub's single shared id space. This holds even though the two kinds of explicit anchor reach the resolver by different mechanisms; the tech spec states how document order is guaranteed across them. Explicit anchor ids match exactly as authored (no slug normalization is applied to them); heading slugs match after normalization per invariant 4.

7. A `#fragment` that matches nothing remains a normal-looking, clickable link, and clicking it does nothing observable: no scroll, no error dialog, no attempt to open `#fragment` as an external URL, no broken-link tooltip. It must not panic or freeze the viewer. This is the single statement of miss behavior; invariants 13 and 14 refer to it rather than restating it.

8. Attributes on `<a>` other than `href`/`id`/`name` are accepted without breaking the parse and have no behavioral effect — no `target` window semantics, no tooltip from `title`, no styling from `class` or `style`.

9. `<a href>` renders its inline content with at least plain text. Bold, italic, and code *inside* the anchor text are a nice-to-have; plain-text-only anchor content is an acceptable outcome for this slice provided the tech spec states which was built.

10. Malformed anchor markup degrades to literal text for the offending tag without swallowing the rest of the paragraph or document, and without panicking. This covers, deterministically: an unterminated `<a` with no closing `>`; a well-formed opening tag with no closing `</a>`; `href`, `id`, or `name` present with no value or an empty value; unbalanced quoting; and nested `<a>` tags, where the outer tag's `href` applies and the inner opening tag is literal text. An anchor tag written inside a code span or fenced code block is inert in **both** roles: it does not render as a link, and it does not register a jump target — so a `#fragment` pointing at an id that appears only inside code is a miss per invariant 7. Code content stays literal, matching every other inline construct.

11. An `<a>` carrying both `href` and `id`/`name` performs **both** roles, matching GitHub: it renders as a clickable link per invariant 1, **and** it registers its `id`/`name` as a jump target per invariant 5. Neither role is dropped, and neither is degraded by the presence of the other. Both attribute orders behave identically — `<a href="…" id="…">` and `<a id="…" href="…">` are the same tag. Because the tag renders as a link, its markup is not visible literal text; invariant 5's visible-tag limitation applies only to bare and self-closing anchors. A jump to this tag's id lands on the link, and clicking the link follows its `href`.

12. Only fragments and the schemes markdown links already open act on click. A `#fragment` resolves in-document per invariants 2–7. A link whose target is an ordinary external URL opens through the viewer's existing link path exactly as the markdown equivalent does. Any other scheme — `javascript:`, `data:`, `file:` and the like — is treated exactly as the markdown link with the same target is treated today; `<a href>` introduces no scheme handling of its own and no new capability. Event-handler attributes (`onclick`, any `on*`) are attributes per invariant 8: read past, never executed.

13. Clicking `[text](other-file.md#section)` opens `other-file.md` in the Markdown viewer — or focuses its tab if already open — and scrolls to `section` once loaded, using the same resolution as a same-document jump. If the file opens but has no matching anchor, the outcome is invariant 7's miss: the file is shown, unscrolled, no error. If the file cannot be resolved at all, clicking is a no-op, matching a broken relative link today. A relative link that resolves to the *currently open* document keeps focus on that tab rather than opening a duplicate, and a fragment on it scrolls within that tab.

14. On the TUI Markdown render surface, an `<a href>` link renders as styled but inert text — the link text is displayed, the target is not followed, and clicking a `#fragment` produces invariant 7's miss outcome for a different reason: the TUI has no scroll model at all, so nothing resolves there rather than merely failing to match. This applies to dual-role tags too: the TUI renders them as links exactly as the GUI does, since both share the parser, but their registered jump targets are unreachable there for the same no-scroll-model reason. A bare or self-closing `<a id>` tag renders as visible literal text there exactly as it does in the GUI (invariant 5). No TUI-specific behavior is added by this feature; it inherits only what the shared parser produces.

15. Anchors work inside other block constructs, because anchor parsing is ordinary inline content: an `<a href>` inside a list item or a table cell renders and resolves normally, and a heading inside those constructs is addressable by its slug per invariant 4. Table cells here means Markdown's own pipe tables, which the viewer already renders; anchors inside raw-HTML `<table>` markup arrive with that feature (#13726). The sole exception is code — inline code spans and fenced code blocks — where anchor markup stays literal per invariant 10.

16. Resolution is computed against the document's current content at click time, so a fragment link keeps working after the document is edited — adding, renaming, or deleting a heading changes what resolves on the *next* click with no stale-cache window. A renamed heading's old slug simply becomes a miss per invariant 7.

## Delivery scope

All sixteen invariants above are one deliverable. They are listed here as three groups only to show how the work decomposes if it needs to be split for review — not as a staged rollout where users would see a partial feature.

- **Anchor links and heading resolution** — invariants 1 through 4, 7 through 10, 12, 14, and 16, plus the heading-slug half of invariant 6 and the `<a href>` half of invariant 15. This is the slice that fixes the issue's headline case, and it is the highest-value one because it repairs markdown-native `[text](#heading)` links too, which gain nothing from HTML tag parsing on its own.
- **Explicit `<a id>`/`<a name>` targets** — invariants 5 and 11, plus the explicit-anchor half of invariant 6 (an anchor id colliding with a heading slug, with another anchor id, or across the two anchor kinds). This completes the hand-built table-of-contents case for authors who anchor mid-paragraph rather than at a heading. The bare-anchor half costs very little on top of the previous group, reusing the same click-time resolution walk rather than introducing a parsed anchor node. Invariant 11's dual-role case is the larger part: it requires the anchor-id style field and its save round trip that the tech spec specifies, since a tag consumed into a link cannot be recovered from document text. The `<a id>` half of invariant 15 sits here too, since registering a target inside a list item or table cell rides the same explicit-anchor mechanism.
- **Cross-document fragments** — invariant 13, together with the three resolution repairs the tech spec details. These repairs are prerequisites rather than polish: without them even a plain `[text](other-file.md)` link misroutes to the browser or silently no-ops, so the fragment feature cannot work without them.

**One known limitation ships with this feature and is not deferred out of it:** a bare `<a id="x"></a>` remains visible as literal text in the rendered document (invariant 5). This is current, intended behavior — not a bug to be found later. Hiding the tag requires a content-model representation that survives `to_markdown` on save, or the author's anchor is silently deleted by the next edit; that work is tracked separately as #13982 so the representation can be designed deliberately rather than chosen under implementation pressure.
