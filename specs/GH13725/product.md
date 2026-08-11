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
- Parse `<a id="…"></a>` and `<a name="…"></a>` (the empty or self-closing form authors actually write) as a named jump target at that point in the document.
- Give every heading (`#`…`######`) an implicit anchor slug derived from its rendered text, so `[Jump to Target Section](#target-section)` works against an ordinary heading with zero authoring effort.
- Resolve a `#fragment` click — from either syntax — against the document's anchors (explicit targets and implicit heading slugs) and scroll so the target is visible.
- Resolve a combined path-and-fragment link (`other-file.md#section`) by opening or focusing that file's viewer tab and scrolling it to the anchor once loaded.
- Degrade a `#fragment` with no matching anchor to an inert click: no navigation, no error, no crash.
- Accept and ignore `<a>` attributes beyond `href`/`id`/`name` (`title`, `target`, `rel`, `class`, `style`, …) — they neither break the parse nor do anything.

Out of scope (explicit non-goals):

- **Hiding the rendered anchor tag.** A bare `<a id="x"></a>` remains *visible as literal text* rather than rendering as nothing the way browsers and GitHub do. Invariant 5 states this fully. Hiding it requires a first-class content-model representation that survives `to_markdown` on save, tracked as #13982.
- **Following a cross-document fragment into a non-Markdown target or an external editor.** The cross-document scroll applies only when the target opens in Warp's Markdown viewer. With the Markdown Viewer preference off, the file opens in the code editor or an external app and the fragment is dropped — matching how a plain relative file link behaves today. Bridging a `#slug` to a code-editor line is a separate concern.
- **Fragment scroll in the terminal (TUI) Markdown renderer.** `<a href>` links *render* in the TUI for free because it shares the parser, but clicking a `#fragment` there does not scroll: the TUI has no scroll model and no click-to-navigate path. Invariant 14 states the TUI's behavior in full.
- **GitHub's duplicate-slug disambiguation.** GitHub appends `-1`, `-2`, … to colliding slugs. Warp resolves collisions by first occurrence in document order (invariant 6); replicating the numeric suffix scheme is not required.
- **`<a>` tags carrying both `href` and `id`/`name` on one element.** The real-world pattern uses them separately. Invariant 11 pins the behavior deterministically so it is never undefined, but supporting both roles meaningfully on one tag is not a goal.
- **Any other raw-HTML tag** (`<img>`, `<table>`, `<details>`, …) — each has its own spec in this split.
- **Editing and authoring affordances** — no "copy anchor link" UI, no auto-slug preview while typing. This is a rendering and navigation feature only.
- **A new URL-scheme trust boundary.** `<a href>` reuses the same link representation and click path markdown links already use, inheriting that boundary unchanged rather than introducing a second one. Invariant 12 states which schemes act and which are inert.

## Behavior

1. `<a href="https://warp.dev">Visit Warp</a>` renders as a clickable link reading "Visit Warp", visually and behaviorally identical to the markdown link `[Visit Warp](https://warp.dev)`.

2. `<a href="#target-section">Jump to Target Section</a>` renders as a clickable link; clicking it scrolls the viewer so the anchor matching `target-section` is visible. This is the HTML-tag half of the issue's test case.

3. The markdown-native equivalent `[Jump to Target Section](#target-section)` gets the **same** click behavior as invariant 2. It already parses as a link today; only resolution is new. This is the contrasting case the issue names explicitly as "resolves as a plain URL hyperlink" today.

4. A heading `## Target Section` is addressable as `#target-section` with no authoring effort and no `<a id>` required. The slug is derived from the heading's rendered text by the algorithm the tech spec defines; that algorithm is Unicode-preserving, so `## Café Société` is addressable as `#café-société` and `## 日本語` as `#日本語`. Non-English headings are a supported case, not a degraded one.

5. `<a id="target-section"></a>` or `<a name="target-section"></a>`, placed anywhere in the document, registers `target-section` as a jump target.

   **The tag renders as visible literal text.** Browsers and GitHub render a content-less anchor as nothing; Warp does not replicate that in this slice. The tag appears inline exactly as authored. Jump-target resolution works regardless, because resolution reads the document's live text rather than a parsed anchor node. Making the tag disappear requires representing it as content-model metadata that still re-serializes on save — otherwise editing the document silently deletes the author's anchor — which is a genuine model change, deferred to #13982 so maintainers can weigh the representation before it is built.

6. Explicit anchors and implicit heading slugs share **one namespace**, resolved by **first occurrence in document order**. This single rule covers every collision case: two headings with the same slug, two `<a id>` values that are identical, or an `<a id>` colliding with a heading slug. There is no anchor-versus-heading priority tier — whichever appears first in the document wins, matching GitHub's single shared id space. Explicit anchor ids match exactly as authored (no slug normalization is applied to them); heading slugs match after normalization per invariant 4.

7. A `#fragment` that matches nothing remains a normal-looking, clickable link, and clicking it does nothing observable: no scroll, no error dialog, no attempt to open `#fragment` as an external URL, no broken-link tooltip. It must not panic or freeze the viewer. This is the single statement of miss behavior; invariants 13 and 14 refer to it rather than restating it.

8. Attributes on `<a>` other than `href`/`id`/`name` are accepted without breaking the parse and have no behavioral effect — no `target` window semantics, no tooltip from `title`, no styling from `class` or `style`.

9. `<a href>` renders its inline content with at least plain text. Bold, italic, and code *inside* the anchor text are a nice-to-have; plain-text-only anchor content is an acceptable outcome for this slice provided the tech spec states which was built.

10. Malformed anchor markup degrades to literal text for the offending tag without swallowing the rest of the paragraph or document, and without panicking. This covers, deterministically: an unterminated `<a` with no closing `>`; a well-formed opening tag with no closing `</a>`; `href`, `id`, or `name` present with no value or an empty value; unbalanced quoting; and nested `<a>` tags, where the outer tag's `href` applies and the inner opening tag is literal text. An anchor tag opened inside a code span or fenced code block is not parsed as an anchor at all — code content stays literal, matching every other inline construct.

11. An `<a>` carrying both `href` and `id`/`name` resolves deterministically rather than being undefined: it renders as a link per invariant 1 (the `href` role wins for rendering) **and** registers its `id`/`name` as a jump target per invariant 5. Neither role is silently dropped. This is a non-goal in the sense that it is not a case the feature is designed around, but its behavior is specified so it can never be a surprise.

12. Only fragments and the schemes markdown links already open act on click. A `#fragment` resolves in-document per invariants 2–7. A link whose target is an ordinary external URL opens through the viewer's existing link path exactly as the markdown equivalent does. Any other scheme — `javascript:`, `data:`, `file:` and the like — is treated exactly as the markdown link with the same target is treated today; `<a href>` introduces no scheme handling of its own and no new capability. Event-handler attributes (`onclick`, any `on*`) are attributes per invariant 8: read past, never executed.

13. Clicking `[text](other-file.md#section)` opens `other-file.md` in the Markdown viewer — or focuses its tab if already open — and scrolls to `section` once loaded, using the same resolution as a same-document jump. If the file opens but has no matching anchor, the outcome is invariant 7's miss: the file is shown, unscrolled, no error. If the file cannot be resolved at all, clicking is a no-op, matching a broken relative link today. A relative link that resolves to the *currently open* document keeps focus on that tab rather than opening a duplicate, and a fragment on it scrolls within that tab.

14. On the TUI Markdown render surface, an `<a href>` link renders as styled but inert text — the link text is displayed, the target is not followed, and a `#fragment` does not scroll. A bare `<a id>` tag renders as visible literal text there exactly as it does in the GUI (invariant 5). No TUI-specific behavior is added by this feature; it inherits only what the shared parser produces.

15. Anchors work inside other block constructs, because anchor parsing is ordinary inline content: an `<a href>` inside a table cell, a list item, or a blockquote renders and resolves normally, and a heading inside those constructs is addressable by its slug per invariant 4. The sole exception is code — inline code spans and fenced code blocks — where anchor markup stays literal per invariant 10.

16. Resolution is computed against the document's current content at click time, so a fragment link keeps working after the document is edited — adding, renaming, or deleting a heading changes what resolves on the *next* click with no stale-cache window. A renamed heading's old slug simply becomes a miss per invariant 7.
