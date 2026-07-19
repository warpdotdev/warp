# PRODUCT.md — Markdown viewer: honor raw-HTML `<img>` sizing (width/height/align)

Issue: https://github.com/warpdotdev/warp/issues/13721

## Summary

Warp's Markdown Viewer renders Markdown image syntax (`![alt](src)`) but silently
drops raw inline HTML, so a raw-HTML `<img>` tag does not load or render **at all** —
it disappears into literal, unrendered text. This is the single highest-priority tag in
the issue's prioritized list, and the one that directly motivated the request. Making
the tag render is the core fix; honoring its presentational attributes is the rest.
Authors reach for HTML `<img>` over `![alt](src)` precisely *because* of those
attributes — chiefly sizing: the Markdown-image renderer draws every image at a fixed
default size that ignores the image's intrinsic dimensions, and the HTML `width`
attribute — the only author-controllable sizing signal — is dropped along with the rest
of the tag. So the motivating example (embedding a locally-generated SVG dashboard at a
controlled width) needs both halves: the tag must render, and its `width`/`height`/`align`
must be honored.

This spec covers a narrow, self-contained slice split out of the original bulk
raw-HTML-subset request (#13652): teach the Markdown viewer to recognize a
block-level `<img>` HTML tag and honor its `width`, `height`, and `align`
attributes. It intentionally does **not** attempt the other tags from that
original request (tables, `<details>`, anchors, `<kbd>`, `<picture>`, etc.); those
are tracked as their own focused follow-up issues, several of which (e.g. #13736,
`<picture>`/`<source>`) are blocked on this one landing first.

Figma: none provided.

## Goals / Non-goals

In scope:

- Recognize a block-level raw-HTML `<img …>` tag in a Markdown document as an image,
  equivalent to the existing `![alt](src)` Markdown image, and render it through the
  same image pipeline.
- Honor the `src`, `alt`, `title`, `width`, and `height` attributes of that tag.
- Honor the `align` attribute (`left` / `center` / `right`) for horizontal placement.
- Support both absolute pixel sizes (`width="640"`, `width="640px"`) and
  percentage-of-available-width sizes (`width="90%"`), matching the issue's motivating
  example `<img src="assets/chart.svg" alt="…" width="90%">`.
- Preserve aspect ratio when only one of `width` / `height` is specified.
- Degrade gracefully: an `<img>` tag that cannot be parsed as a well-formed image
  (malformed, missing `src`, or embedded mid-sentence with other text on the line)
  renders as literal text exactly as it does today — never a panic, never a blank gap.

Out of scope (not changed by this spec):

- Inline `<img>` tags mixed with other text on the same line. Like Markdown images
  today, only images that occupy their own line are rendered; mixed-content lines fall
  back to text.
- All other HTML tags from the issue's list: `<a href/id>`, `<table>`/`<tr>`/`<td>`,
  `<details>`/`<summary>`, `<br>`, `<kbd>`, `<sub>`/`<sup>`, `<p align>`/`<div align>`,
  `<picture>`/`<source>`.
- Responsive-image source selection via `srcset` (density/width candidate lists such as
  `1x`/`2x`/`480w`). This is a refinement deferred to the `<picture>`/`<source>` feature
  (#13736), which owns responsive selection; on a bare `<img>`, `srcset` is ignored and
  only `src` is read — matching the `src`-fallback behavior of a `srcset`-unaware
  browser. This is a **feature deferral, not a security exclusion** (contrast the
  `onerror`/`onload`/`style` drops in invariant 13, which are security-motivated).
- Honoring the intrinsic `width`/`height` of an SVG when **no** HTML sizing attribute
  is present. This spec makes author-specified sizing work; changing the no-attribute
  default sizing behavior is a separate follow-up.
- Arbitrary CSS in a `style="…"` attribute on `<img>`. Only the discrete `width`,
  `height`, and `align` attributes are honored.
- Any script execution, event handlers, or embedded-browser behavior. This is static,
  sanitized HTML only.

## Behavior

1. A Markdown document line consisting solely of a raw-HTML `<img>` tag (optionally
   surrounded by leading/trailing whitespace) renders as an image in the Markdown
   viewer, using the tag's `src` attribute as the image source, resolved with the
   same rules as a Markdown `![alt](src)` image (data URIs, `http(s)://` URLs,
   absolute paths, and paths relative to the document's directory). If that source
   *fails to load* (missing file, unreadable, or a decode failure), the tag behaves
   exactly as a Markdown `![alt](missing.png)` does today — it stays an image block at
   its configured size but renders as an empty box (no placeholder, broken-image icon,
   or alt text), never a panic and never a fallback to literal text. This load-failure
   visual is inherited from the existing image pipeline and unchanged by this spec; see
   the tech spec's failure-state note for the mechanism.

2. The tag's `alt` attribute, when present, is used as the image's alt text (the same
   role `alt` plays for a Markdown image). When `alt` is absent, alt text is empty.

3. The tag's `title` attribute, when present and non-empty, is preserved for the same
   uses as a Markdown image title (round-trip serialization / export / tooltip where
   applicable). An absent or empty `title` is treated as no title.

4. When the tag specifies `width` as an integer number of pixels (`width="640"` or
   `width="640px"`), the image is laid out at that width in pixels, clamped so it never
   exceeds the available content width of the pane (horizontal overflow would force an
   unpleasant horizontal scroll). A pixel `height` is **not** treated the same way: it
   is honored verbatim (`height="480"` renders at 480px), because vertical space is free
   — the document scrolls. A pixel height is subject only to a sanity cap for
   unreasonable/hostile values (see the tech spec), not to the pane's dimensions. This
   asymmetry follows the guiding principle *model any reasonable markdown file, not any
   possible HTML file*: a reasonable author-specified height should be respected. (The
   one case where a specified height yields is when the image has no specified width and
   its aspect-derived width would overflow the pane — see invariant 6; horizontal space
   remains the hard constraint, so the box scales down uniformly rather than distorting
   or overflowing.)

5. When the tag specifies `width` as a percentage (`width="90%"`), the image is laid
   out at that percentage of the pane's available content width. A percentage `height`
   is relative to the image block's **default height** (the fallback size an image with
   no specified height would render at — `base_line_height × DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER`
   in the tech spec), **not** the pane's visible viewport height. A percentage over
   100% clamps to that reference (`height="200%"` renders at the full default-height
   bound), mirroring how `width="200%"` clamps to the pane width.

6. When only one of `width` / `height` is specified, the other dimension is derived
   from the image's intrinsic aspect ratio once it has decoded, so the specified
   dimension is honored exactly and the settled image is not distorted — with one
   pane-fit exception below. (See the tech spec for the layout-time mechanism — it
   re-derives the missing dimension from the decoded image's intrinsic size, the same
   approach already used for Mermaid diagrams, rather than relying on generic
   contain-fit scaling.)

   **Pane-fit exception (specified `height`, no `width`).** If the aspect-derived width
   would exceed the pane's available content width, the box scales down *uniformly* so
   the width equals the pane and the effective height is proportionally reduced below
   the specified value. The precedence is **aspect ratio > pane-width bound > specified
   dimension**: the image is never distorted (the ratio always wins) and never overflows
   the pane horizontally (the pane bound outranks the specified height), so in this one
   case the specified height is not honored exactly. This mirrors the width side — a
   too-wide specified `width` already clamps to the pane and derives its height down —
   reached from the opposite axis. It deliberately diverges from browsers, which let the
   image overflow horizontally; the pane model treats horizontal space as a hard
   constraint (invariant 4). A specified `width` with a derived height has no analogous
   case, because the derived height is unbounded (vertical space is free), so a
   specified width is always honored exactly.

   **Transient pre-decode exception.** For a single **transient** layout pass *before
   the asset has decoded*, the intrinsic ratio is not yet known, so the derived
   (unspecified) axis falls back to a default and the frame is drawn stretched to fill
   that box. This means the derived axis — never the author-specified one — may be
   briefly distorted for that one pre-decode frame; it self-corrects to the
   aspect-ratio-correct size on the very next layout pass once the asset decodes (the
   same self-correction Mermaid diagrams rely on today). The trade-off is deliberate:
   stretching the *derived* axis for one frame is preferable to letting contain-fit
   shrink the *specified* axis below its requested value. See the tech spec's
   "Why the pre-decode fallback needs `stretch()`" section and its layout-math tests
   (the `Loading` → `Loaded` transition case) for the precise mechanism.

7. When neither `width` nor `height` is specified, the `<img>` renders at the same
   default size a Markdown `![alt](src)` image renders at today. The presence of the
   HTML tag alone does not change default sizing.

8. The `align` attribute positions the image horizontally within the pane:
   `align="left"` (default) left-aligns, `align="center"` centers, `align="right"`
   right-aligns. An absent or unrecognized `align` value left-aligns.

9. Attribute parsing is case-insensitive for attribute names and for the `align`
   keyword values, matching HTML semantics (`WIDTH`, `Width`, `width` are equivalent;
   `align="Center"` centers).

10. An `<img>` tag that is not well-formed enough to yield a usable image — for example
    a tag with no `src` attribute, an empty `src`, or an unterminated tag — is rendered
    as literal text (the raw tag characters), identical to how unrecognized raw HTML is
    rendered today. It must not panic, and must not render a blank/zero-size image box.

11. A line that contains an `<img>` tag alongside other non-whitespace text (before or
    after the tag) is rendered as text, not as an image — mirroring the existing
    Markdown-image "images must be on their own line" behavior. (Multiple `<img>` tags
    separated only by whitespace on one line MAY render as a run of images, matching the
    existing Markdown image-run behavior; this is allowed but not required by this spec.)

12. An unparseable or non-numeric sizing attribute (`width="abc"`, `width=""`,
    `height="-40"`) is ignored for that dimension, and the image falls back to the
    default sizing for that dimension (invariant 7) rather than failing to render.

13. Malicious or unexpected attribute values do not cause code execution or navigation.
    Only `src`, `alt`, `title`, `width`, `height`, and `align` are read from the tag;
    all other attributes (including `onerror`, `onload`, `style`, etc.) are ignored.
    `src` values are resolved through the same asset-source resolver used for Markdown
    images and are never executed. (`srcset` is also not read, but for a different
    reason — it is a deferred *feature*, not a security drop; see the responsive-image
    non-goal above.)

14. Copy / export round-trips of a document containing a sized `<img>` preserve enough
    information to reproduce the rendered image. (The exact serialized form — whether the
    raw `<img>` tag is preserved verbatim or canonicalized — is an implementation
    decision for the tech spec, but the `src`, `alt`, `title`, and specified dimensions
    must survive.)
