# PRODUCT.md — Markdown viewer: honor raw-HTML `<img>` sizing (width/height/align)

Issue: https://github.com/warpdotdev/warp/issues/13652

## Summary

Warp's Markdown Viewer renders Markdown image syntax (`![alt](src)`) but silently
drops raw inline HTML, including the `<img>` tag. This is the single highest-priority
tag in the issue's prioritized list, and the one that directly motivated the request:
authors who embed locally-generated SVG dashboards in Markdown notes have no way to
control the rendered size, because the Markdown-image renderer draws every image at a
fixed default size that ignores the image's intrinsic dimensions, and the HTML `width`
attribute — the only author-controllable sizing signal — is dropped entirely.

This spec covers a narrow, self-contained slice of the broader issue: teach the
Markdown viewer to recognize a block-level `<img>` HTML tag and honor its `width`,
`height`, and `align` attributes. It intentionally does **not** attempt the full
raw-HTML subset (tables, `<details>`, anchors, `<kbd>`, `<picture>`, etc.); those
remain follow-ups tracked by the same issue.

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
   absolute paths, and paths relative to the document's directory).

2. The tag's `alt` attribute, when present, is used as the image's alt text (the same
   role `alt` plays for a Markdown image). When `alt` is absent, alt text is empty.

3. The tag's `title` attribute, when present and non-empty, is preserved for the same
   uses as a Markdown image title (round-trip serialization / export / tooltip where
   applicable). An absent or empty `title` is treated as no title.

4. When the tag specifies `width` as an integer number of pixels (`width="640"` or
   `width="640px"`), the image is laid out at that width in pixels, clamped so it never
   exceeds the available content width of the pane. `height` is treated the same way.

5. When the tag specifies `width` as a percentage (`width="90%"`), the image is laid
   out at that percentage of the pane's available content width. `height` as a
   percentage is likewise relative to the available height budget.

6. When only one of `width` / `height` is specified, the other dimension is derived to
   preserve the image's aspect ratio (the renderer already scales by the smaller of the
   width/height ratios inside its layout box, so specifying one dimension and leaving
   the other unconstrained yields proportional scaling).

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
    all other attributes (including `onerror`, `onload`, `style`, `srcset`, etc.) are
    ignored. `src` values are resolved through the same asset-source resolver used for
    Markdown images and are never executed.

14. Copy / export round-trips of a document containing a sized `<img>` preserve enough
    information to reproduce the rendered image. (The exact serialized form — whether the
    raw `<img>` tag is preserved verbatim or canonicalized — is an implementation
    decision for the tech spec, but the `src`, `alt`, `title`, and specified dimensions
    must survive.)
