# CommonMark Image Title and Alt Text Fallback in Block-List Image Rendering — Product Spec
GitHub issue: https://github.com/warpdotdev/warp-external/issues/849
Figma: none provided

## Summary
Expand Warp's markdown image parser and renderer so that the AI block list supports same-line CommonMark image title forms such as `![alt](source \"title\")`, using the title as a hover tooltip on successful renders and the alt text as the load-failure fallback. This makes the dogfood block-list image feature consistent with how GitHub, GitLab, VS Code Markdown Preview, Obsidian, Typora, and Pandoc render markdown images today.

## Problem
The AI block-list image renderer described in `specs/zachlloyd/inline-markdown-images-in-blocklist/PRODUCT.md` parses only `![alt](source)`. Two concrete gaps fall out of that today:

- Any authored image that uses the CommonMark title form — `![alt](source "title")`, `![alt](source 'title')`, or `![alt](source (title))` — fails to parse as an image and falls back to raw markdown text. A valid, standards-conformant image never renders.
- Alt text is captured on the parsed image model but never surfaced anywhere user-visible. When an image fails to render (missing file, unsupported format, asset load failure), the user sees raw markdown instead of the author-provided alt fallback, and successfully rendered images expose no tooltip and no hover affordance for the title.

The behavior in this spec is additive on top of the existing block-list image feature and does not change the default "no title, no alt" code path.

## Goals
- Parse the optional CommonMark title suffix on markdown images — `"..."`, `'...'`, and `(...)` — in both block-level and inline image positions already supported by the block-list renderer.
- Carry the parsed title through every downstream consumer of the shared `FormattedImage` image model so it is available to the AI block-list renderer, the editor buffer round-trip, and HTML export.
- Render the title as a hover tooltip on successfully rendered block-list images, using Warp's standard tooltip treatment.
- Surface the alt text as a visible fallback string when the image fails to render (missing file, unsupported format, asset load failure).
- Preserve the parsed alt, source, and title content through block-level copy and export. Markdown re-serialization may canonicalize titled images to the double-quoted form `![alt](src \"title\")`; HTML export must carry `alt=\"...\"` and, when present, `title=\"...\"`.
- Leave behavior unchanged for images that do not have a title (pure `![alt](source)`).

## Non-goals
- Visible captions under rendered images. Title is rendered as a hover tooltip only, matching GitHub/GitLab/VS Code/Obsidian/Typora/Pandoc default behavior; a visible caption would be a separate feature.
- Lightbox chrome changes. Title and alt are not required to appear inside the shared fullscreen lightbox overlay in this change.
- Surfacing title or alt in non-block-list markdown surfaces beyond the editor round-trip and HTML export changes needed to carry the title field.
- Any new parser behavior outside of the standard CommonMark image form — for example, reference-style images, link references, or arbitrary inline image prose remain out of scope (see `specs/zachlloyd/inline-markdown-images-in-blocklist/PRODUCT.md` follow-ups).
- Changing the existing block-list-image feature flag gating. The dogfood-only rollout continues to follow the same flag.

## Behavior

1. When block-list markdown contains `![alt](source)` — with no title — rendering, copy, export, and fallback behave exactly as they do today. No regression to the default form.

2. When block-list markdown contains `![alt](source "title")`, the parser accepts the entire construct as a single markdown image whose alt text is `alt`, whose source is `source`, and whose title is `title`.
   - Whitespace between `source` and the opening title delimiter may be any amount of spaces or tabs.
   - A line ending between `source` and the opening title delimiter is intentionally not accepted in this change; that form falls back to plain text rather than being treated as a titled image.
   - The trailing `)` closes the image the same way it does when no title is present.
   - An image that would otherwise be recognized only when it stands alone on its own line continues to have that same placement rule when a title is present.

3. In addition to the double-quote form, the parser accepts both of the other CommonMark title delimiters:
   - single-quoted: `![alt](source 'title')`
   - paren-wrapped: `![alt](source (title))`

4. Title content follows CommonMark rules:
   - Title text may be empty (`\"\"`, `''`, or `()`); an empty title is equivalent to no title and must not produce a tooltip.
   - Leading and trailing whitespace inside a non-empty title is preserved literally. A whitespace-only title such as `\"   \"` is therefore treated as non-empty rather than normalized away.
   - A title that contains the matching closing delimiter must be escaped per CommonMark (`\"`, `\'`, `\)`); the renderer receives the unescaped literal string.
   - A title that never closes before end-of-line or end-of-input causes the whole image construct to fall back to plain text, the same way a malformed `![alt](source` falls back today.

5. When the image source is unparseable or the closing `)` is missing, the entire construct continues to fall back to plain text. Adding title support must not change the existing fallback surface area for malformed images.

6. When a block-list image renders successfully and the parsed title is non-empty, hovering the rendered image displays the title string in Warp's standard tooltip.
   - The tooltip uses the same tooltip primitive already used elsewhere in the product.
   - The tooltip content is the literal title string, with no markdown re-rendering, no link decoration, and no truncation beyond the tooltip's existing layout rules.
   - A successful render with an empty or absent title shows no tooltip on hover.

7. When a block-list image fails to render (path cannot be resolved, file does not exist, format is unsupported, asset load fails), the fallback string is:
   - the alt text, when the parsed alt text is non-empty
   - the raw markdown source (preserving the current behavior) when the parsed alt text is empty
   This applies in both inline image rows and block-level image groups defined by the block-list image product spec. Unsupported/failed items still fall back individually while their neighbors continue to render, as today.

8. Right-click copy on a successfully rendered image continues to place the underlying markdown source on the clipboard. When the image carries a title, the copied source preserves the parsed alt text, source, and title content, but markdown serialization canonically uses the double-quoted form — for example, a titled image copies as `![alt](src \"title\")` rather than dropping the title.

9. Block-level output copy and conversation export serialize image sections from the preserved image fields. When a title is present, markdown export includes it using the canonical double-quoted form `![alt](src \"title\")`. When the export path emits HTML (for rich-text or HTML clipboard), the resulting `<img>` tag includes `alt=\"...\"` and, when a non-empty title was present, `title=\"...\"` — and omits `title` when the parsed title is empty or absent.

10. The title is not rendered as a visible caption, badge, or sibling text under the image in any layout (inline image row, block-level image group, or any future layout). The hover tooltip in invariant 6 is the only user-visible surface for title in this change.

11. Streamed and restored AI blocks behave identically. A response that contains a titled image renders the same tooltip + alt-fallback behavior whether the block is live-streaming or is re-rendered from conversation history. Restored images whose source can no longer be resolved fall back to the alt text (when non-empty) using the rules in invariant 7.

12. The block-list image feature remains gated behind the existing `BlocklistMarkdownImages` flag. When that flag is disabled, image sections continue to fall back to the raw markdown source, now including the title suffix. Title parsing itself is not gated separately — the shared markdown parser always accepts CommonMark titles, so non-block-list callers that later consume images get the title field for free.

13. Adjacent markdown constructs are not affected by title support:
    - A paragraph that contains text resembling `something ("title")` but is not a markdown image renders as plain text.
    - A link with a title (`[text](url "title")`) is out of scope for this change; the link parser's current behavior is preserved.
    - A code span or code block that contains `![alt](source "title")` continues to render as literal code and never becomes a rendered image.
