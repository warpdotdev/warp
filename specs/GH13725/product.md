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
  hand-authored form) as a named anchor target at that point in the document. **As shipped,
  the tag itself still renders as visible literal text** (e.g. `<a id="x"></a>` appears
  inline in the document) rather than rendering nothing the way GitHub/browsers do — see the
  note under invariant 5 and #13982 for why, and for the deferred follow-up that would hide
  it.
- Give every heading (`#`…`######`) an **implicit** anchor slug derived from its rendered
  text, so `[Jump to Target Section](#target-section)` works against ordinary headings with
  zero authoring effort — the common case the issue's test document exercises.
- Resolve a `#fragment` hyperlink click (from either an `<a href>` tag or a markdown
  `[text](#fragment)` link) against the set of anchors in the current document — explicit
  `<a id>`/`<a name>` targets and implicit heading slugs — and scroll the viewer so the
  target is visible, instead of the current behavior (treated as an opaque URL).
- **Cross-document fragment links** — a link whose target combines a relative file path and
  a fragment, e.g. `[text](other-file.md#section)`. Clicking it opens (or, if already open,
  focuses) that file's Markdown-viewer tab and scrolls it to the matching anchor once the
  document has loaded. A relative file link *without* a fragment already opens the target
  today; this in-scope item is specifically the fragment half — carrying the `#section`
  through the file-open flow and scrolling after load. **Delivered in this PR** alongside
  phase 1 (see phasing), because it builds on the same-document slug resolver rather than
  replacing it, and because implementation surfaced three latent resolution defects
  (documented in the tech spec) that had to be repaired for even a fragment-less bare
  `README.md` link to open reliably.
- A `#fragment` link with no matching anchor in the document degrades gracefully: it
  remains a normally-styled, clickable-looking link, but clicking it is a no-op (no
  navigation, no error, no crash) rather than attempting to open `#fragment` as a URL.
- `<a href>` attributes beyond `href` — `title`, `target`, `rel`, `class`, etc. — are
  parsed-but-ignored: they don't break parsing, and they don't do anything (no `target`
  window semantics, no HTML tooltip from `title`).

Out of scope (explicit non-goals):

- **Following a cross-document fragment link into a non-Markdown target, or into an
  external editor.** The cross-document scroll-to-anchor (in-scope above, phase 3) applies
  only when the target file opens in Warp's Markdown viewer. If the user's Markdown Viewer
  preference is off — so the file opens in the code editor or an external app — the
  fragment is dropped and only the file opens, matching how a plain relative file link
  behaves today. Bridging a `#slug` anchor to a code-editor line is a separate concern.
- **Cross-document anchor scroll in the terminal (TUI) Markdown renderer.** Same rationale
  as the same-document TUI non-goal below — the TUI has no scroll model or click-to-open
  path. `other-file.md#section` in the TUI renders as inert styled text.
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
- **Anchor scroll-to in the terminal (TUI) Markdown renderer.** `<a href>` links will
  *render* in the TUI viewer for free (it shares the parser), but clicking a `#fragment`
  there does not scroll — the TUI has no scroll model or click-to-navigate path. Fragment
  resolution is a GUI-viewer feature in this slice.

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
   registers `target-section` as a jump target.

   **Rendering, as shipped: the tag is visible, not hidden.** GitHub/browsers render a
   content-less anchor tag as nothing at all; this PR does not replicate that. The phase-1
   inline parser only recognizes `<a href>` as a token (it requires an `href` attribute to
   match at all), so a bare `<a id>`/`<a name>` falls through to literal text and appears
   inline exactly as authored (`<a id="target-section"></a>`). Resolution (this invariant's
   jump-target behavior) works regardless, because it's a live text scan over that same
   literal content — see tech spec item 5. Making the tag disappear from the rendered view
   requires representing it as first-class block metadata that still round-trips through
   `to_markdown` on save (otherwise editing the document silently deletes the anchor) — a
   genuine content-model change sized at 70-130+ call sites across the buffer/editor layer,
   not a rendering tweak. That work is tracked as its own ticket,
   [#13982](https://github.com/warpdotdev/warp/issues/13982), deliberately deferred so
   maintainers can weigh the representation trade-offs before it's built, rather than shipping
   a design nobody reviewed.

6. Explicit `<a id>`/`<a name>` anchors and implicit heading slugs share **one namespace** —
   there is no separate anchor-vs-heading priority tier. If both an explicit `<a id="x">`
   and a heading whose implicit slug is also `x` exist, `#x` resolves to **whichever occurs
   first in document order**, matching GitHub's single shared id space. This is the same
   "first wins" rule invariant 4's heading-collision case already uses, just extended to
   cover anchors and headings together rather than headings only. See tech spec item 5 for
   the resolution mechanism.

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

11. Clicking `[text](other-file.md#section)` opens `other-file.md` in the
    Markdown viewer (or focuses its tab if already open — the same open/focus behavior a
    plain `other-file.md` link has today) and, once that document has loaded, scrolls it to
    the `section` anchor using the same slug resolution as a same-document jump. If
    `other-file.md` opens but has no matching anchor, the outcome degrades to the
    same-document miss (invariant 7): the file is shown, unscrolled, no error. If the file
    itself cannot be resolved (does not exist relative to the document), clicking is a
    no-op, matching a broken plain relative link today.

12. **Self-referential relative links** — a relative link whose target resolves to the
    *currently-open* document (`this-doc.md`, `./this-doc.md`, with or without a `#fragment`) —
    keep focus on the same tab rather than opening a duplicate, and a fragment scrolls within
    it. This reuses the same open/focus dedup a cross-document link uses; the only subtlety is
    path equality: an open notebook stores its *canonical* path, while the link resolves to
    `base_directory.join(relative)` (with `.`/`..` components and, on macOS, the `/tmp` vs
    `/private/tmp` symlink alias), so the resolved target is canonicalized before the dedup
    comparison. A self-link with a fragment scrolls immediately (the tab is already laid out);
    a self-link without one just refocuses, no scroll.

13. Every link-form's behavior is now fully specified — there are no undefined interim
    states. Concretely, for a scheme-less relative-looking target:
    - **Bare `file.md` that exists on disk** (no `./`, no `/`) resolves as a local file, not
      a web URL — even though `.md`/`.dev`/`.com` are known public suffixes. This repairs a
      latent collision: the pre-existing bare-domain heuristic classified `README.md`,
      `notes.md`, etc. as domains (`.md` is Moldova's ccTLD) and opened them in the browser.
    - **`./file.md`** always resolves as a file (the `./` prefix never parses as a domain);
      unchanged.
    - **`file.md#section`** splits the `#section` fragment off before file resolution, so the
      file opens and the fragment drives the cross-document scroll. A trailing `#L100`
      line-number suffix is *not* an anchor and continues to route through line-number
      handling.
    - **Bare `nonexistent.md` with no matching local file** falls through to the browser,
      preserving the genuine bare-domain behavior (`warp.dev` still opens the browser).
    - **A fragment or file miss** is inert per invariant 7 — no scroll, no error.

## Suggested phasing

The two capabilities compound in value but are separately shippable:

- **Phase 1:** `<a href>` inline links (parsed via the existing `Hyperlink` link-styling
  machinery) **and** heading auto-anchors + fragment click resolution + scroll-to. This
  alone delivers the issue's headline case — an inline HTML link or a markdown
  `[text](#heading)` link jumping to a heading — and is the highest-value slice because it
  fixes markdown-native fragment links too, not just the new HTML tag.
- **Phase 2 (delivered with phase 1 in this PR):** Arbitrary `<a id>`/`<a name>` anchor
  targets (anchors not attached to a heading). Completes the issue's
  hand-built-table-of-contents use case for authors who anchor mid-paragraph. Pulled forward
  from a deferred follow-up: since the phase-1 parser already leaves a bare `<a id>`/`<a
  name>` tag's raw markup (including the id/name value) intact in the buffer as literal text
  — it never matches the `<a href>` delimiter grammar, which requires an `href` attribute —
  resolution can reuse the same zero-cache, live-text-walk pattern phase 1 established for
  headings, with no new content-model field or cached index. See tech spec item 5.
  **Delivered with a caveat:** the anchor tag itself renders as visible literal text (see
  invariant 5) rather than being hidden the way GitHub renders it — hiding it requires a
  first-class, save-round-trippable content-model representation, sized and deferred to
  [#13982](https://github.com/warpdotdev/warp/issues/13982).
- **Phase 3 (delivered with phase 1 in this PR):** Cross-document fragment links
  (`other-file.md#section`). Built on phase 1's slug resolver: the file-open, tab-focus, and
  dedup machinery already exists (a fragment-less relative link opens today), so this phase
  adds fragment carry-through plus a deferred scroll after the target document loads. It was
  pulled forward into this PR because implementation revealed that "a fragment-less relative
  link opens today" was only *partly* true — three latent resolution defects (ccTLD
  misclassification of bare `file.md`, literal `#fragment` breaking file stats, and a
  standalone viewer tab lacking a base directory) meant a bare `README.md` link could
  silently open the browser or no-op. Delivering the fragment feature required repairing
  those, so the whole cross-document path ships together. See tech spec item 6a and the
  resolution-repairs section.
