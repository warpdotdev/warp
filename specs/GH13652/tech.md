# TECH.md — Markdown viewer: honor raw-HTML `<img>` sizing (width/height/align)

Product spec: `specs/GH13652/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13652

## Context

Warp's Markdown viewer parses Markdown in `crates/markdown_parser` into a
`FormattedText` (`crates/markdown_parser/src/lib.rs`), a list of `FormattedTextLine`
variants. Markdown images parse into `FormattedTextLine::Image(FormattedImage)`, where
`FormattedImage` currently models only `alt_text`, `source`, and `title`
(`crates/markdown_parser/src/lib.rs:335-342`):

```rust
pub struct FormattedImage {
    pub alt_text: String,
    pub source: String,
    pub title: Option<String>,
}
```

The Markdown viewer is the notebook/editor viewer in `crates/editor` (Cargo package
`warp_editor`). `FormattedTextLine::Image` is converted into a `BufferBlockItem::Image`
in `crates/editor/src/content/core.rs:877-889`, whose definition lives in
`crates/editor/src/content/text.rs:398-410`:

```rust
pub enum BufferBlockItem {
    HorizontalRule,
    Embedded { item: … },
    Image { alt_text: String, source: String, title: Option<String> },
}
```

That buffer item is laid out into a render block in
`crates/editor/src/content/edit.rs:721-746`, which today **hardcodes** the size,
ignoring any author intent:

```rust
BufferBlockItem::Image { alt_text, source, title: _ } => {
    let spacing = …PlainText…;
    // Default size for images - will scale based on actual image dimensions
    let max_width = layout.max_width() - spacing.x_axis_offset();
    let default_height =
        layout.rich_text_styles().base_line_height()
        * DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER.into_pixels();   // 10.0
    Self::Image {
        alt_text: alt_text.clone(),
        source: source.clone(),
        config: ImageBlockConfig { width: max_width, height: default_height, spacing },
        document_path: document_path.map(|p| p.to_path_buf()),
    }
}
```

`ImageBlockConfig` (`crates/editor/src/render/model/mod.rs:1470-1475`) carries only
`width: Pixels`, `height: Pixels`, `spacing: BlockSpacing`. The drawn element
`RenderableImage` (`crates/editor/src/render/element/image.rs`) uses the
`warpui_core::elements::Image` primitive with `.contain()`, which already preserves
aspect ratio inside the config box (`crates/warpui_core/src/elements/gui/image.rs:120-121`,
`388-394`) and already supports SVG via `usvg`/`resvg`
(`crates/warpui_core/src/image_cache.rs:274-283, 463, 472-476`).

Two facts make this change tractable and low-risk:

1. **The entire image render path is open source in this checkout** — parser →
   content model → layout/sizing → block model → drawn element → primitive. Nothing is
   stubbed or behind a private crate.
2. **There is a direct precedent** for threading a new field through every
   `FormattedImage` consumer: `specs/GH849/` added the `title` field and lists the same
   consumer sites. This spec follows that pattern for `width`/`height`/`align`.

The one piece that does **not** exist today is a parser for a raw-HTML `<img>` tag in
the Markdown block grammar. The current Markdown block parser
(`crates/markdown_parser/src/markdown_parser.rs:132-218`) has a `parse_image` branch
for `![alt](src)` only. Raw HTML tags other than the special-cased inline `<u>`/`</u>`
fall through to the plain-text parser and render as literal text. Warp does already
have a full HTML-document parser (`crates/markdown_parser/src/html_parser.rs`,
using `html5ever`), but that is used for pasting rich text from GDocs/Notion/Confluence
into the editor — it is a whole-document parser, not a per-line block parser, and is
not wired into the Markdown block grammar. This spec adds a small, targeted `<img>`
block parser rather than routing block Markdown through the full HTML parser.

Relevant code:

- `crates/markdown_parser/src/lib.rs:335-342` — `FormattedImage` model.
- `crates/markdown_parser/src/lib.rs:155-300` — `FormattedTextLine::Image` raw-text /
  line-count / weight handling.
- `crates/markdown_parser/src/markdown_parser.rs:138-182` — block parser `alt` chain,
  where a new `parse_html_image` branch is added next to `parse_image`.
- `crates/markdown_parser/src/markdown_parser.rs:295-356` — existing `parse_image`,
  `parse_image_prefix_internal`, `parse_image_target` (the model to mirror).
- `crates/markdown_parser/src/markdown_parser_tests.rs:2320-2576` — existing image
  parser tests (extend here).
- `crates/editor/src/content/core.rs:877-889` — `FormattedTextLine::Image` →
  `BufferBlockItem::Image`.
- `crates/editor/src/content/text.rs:398-410, 420-500` — `BufferBlockItem::Image`
  definition, `PartialEq`, `as_markdown`, `to_formatted_text_line`.
- `crates/editor/src/content/edit.rs:721-746` — image layout/sizing (the core change).
- `crates/editor/src/content/edit.rs:129-131` — `DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER`.
- `crates/editor/src/render/model/mod.rs:1470-1475` — `ImageBlockConfig`.
- `crates/editor/src/render/element/image.rs` — `RenderableImage` (drawing; likely
  needs a horizontal-offset for `align`).
- `crates/editor/src/content/markdown.rs:~1129` — HTML serialization branch for images.
- Test files: `crates/editor/src/content/text_tests.rs`,
  `crates/editor/src/content/core_tests.rs`,
  `crates/editor/src/render/model/mod_tests.rs`.

## Proposed changes

### 1. Model: add optional sizing to `FormattedImage`

Extend `FormattedImage` (`crates/markdown_parser/src/lib.rs`) with three optional
fields:

```rust
pub struct FormattedImage {
    pub alt_text: String,
    pub source: String,
    pub title: Option<String>,
    /// Author-specified width from a raw-HTML `<img width>` attribute.
    /// `None` for Markdown `![alt](src)` images, which have no sizing syntax.
    pub width: Option<ImageDimension>,
    pub height: Option<ImageDimension>,
    pub align: ImageAlign,   // defaults to Left
}
```

Add two small public types in the same module:

```rust
pub enum ImageDimension {
    /// Absolute pixels, e.g. `width="640"` or `width="640px"`.
    Pixels(f32),
    /// Percentage of the available content dimension, e.g. `width="90%"`.
    Percent(f32),
}

#[derive(Default)]
pub enum ImageAlign { #[default] Left, Center, Right }
```

`FormattedImage` stays `Clone + Debug + PartialEq + Eq`-compatible with the rest of the
enum. Because `f32` is not `Eq`, either store the parsed dimension as an integer
(pixels as `u32`, percent as `u16`) or newtype it so the `Eq`/`Hash` derive on the
surrounding types continues to hold. **Recommended:** store integers
(`Pixels(u32)` / `Percent(u16)`) — HTML width/height attributes are integers, so this
loses no precision and keeps `Eq`.

`FormattedTextLine::Image` behavior is unchanged: `raw_text` stays `alt_text\n`,
`num_lines` stays `1`, and `compute_formatted_text_delta` needs no change (still a
derived structural compare).

Markdown `![alt](src)` images continue to construct `FormattedImage` with
`width: None, height: None, align: Left`, so their behavior is byte-for-byte unchanged.

### 2. Parser: recognize a block-level `<img>` tag

Add `parse_html_image` to `crates/markdown_parser/src/markdown_parser.rs`, inserted in
the block `alt(( … ))` chain (`:140-181`) immediately after the existing
`map(parse_image, FormattedTextLine::Image)` branch. Ordering matters: `parse_image`
(Markdown) runs first so nothing about Markdown images changes; the new branch only
matches lines the Markdown image parser rejects.

`parse_html_image` parses, on a single line (optionally with block-leading spaces and
optional trailing whitespace before the line ending, matching `parse_image`'s
`parse_block_leading_spaces` + `parse_line_ending`/`eof` framing at `:305-310`):

- an opening `<img`, ASCII-case-insensitively (reuse `tag_no_case`, already imported);
- a sequence of `name="value"` / `name='value'` / boolean attributes, tolerant of
  arbitrary whitespace, until `>` or `/>`;
- extraction of the recognized attributes (`src`, `alt`, `title`, `width`, `height`,
  `align`), case-insensitively by attribute name. Unrecognized attributes are consumed
  and discarded (invariant 13).

Then it constructs a `FormattedImage`:

- `src` is required; if absent or empty, the parser **fails** so the block falls back
  to `parse_paragraph` and the tag renders as literal text (invariant 10).
- `alt` → `alt_text` (empty when absent, invariant 2).
- `title` → `Some(non_empty)` else `None` (invariant 3), normalizing empty to `None`
  exactly like `parse_image` does at `:347`.
- `width`/`height` parse via a shared `parse_image_dimension(&str) -> Option<ImageDimension>`:
  a trailing `%` → `Percent`, an optional trailing `px` or bare integer → `Pixels`,
  anything else / empty / negative → `None` (attribute ignored, invariant 12).
- `align` parses case-insensitively to `Left`/`Center`/`Right`, defaulting to `Left`
  for absent/unrecognized values (invariant 8, 9).

To keep the grammar small and avoid re-implementing a full HTML tokenizer, the
attribute scanner is a hand-written `nom` loop (mirroring the manual char loops already
used in `parse_image_destination`/`parse_image_title`, `:389-526`) that:

- treats the tag as ending at the first unquoted `>`;
- fails (falls back to text) if it hits a line ending before `>` (invariant 10, matches
  the existing "destinations never span lines" rule at `:401-404`);
- handles both single- and double-quoted attribute values and unquoted values.

This keeps all Markdown-viewer image parsing inside `markdown_parser` and does not
touch the paste-oriented `html_parser.rs`.

Add `pub fn parse_html_image_prefix(input: &str) -> Option<(&str, FormattedImage)>`
paralleling `parse_image_prefix` (`:336-338`) so the existing image-run logic in
`parse_image_run_line` (`:316-334`) can optionally accept `<img>` tags in a whitespace-
separated run (invariant 11, "MAY render as a run"). This is an additive change to the
run loop: try Markdown image prefix, then HTML image prefix.

### 3. Thread the new fields through every `FormattedImage` consumer

`grep`-driven, mechanical — every construction/destructure of `FormattedImage` and
`BufferBlockItem::Image` must carry the new fields. Known sites (from `specs/GH849/`
plus current grep):

- `crates/editor/src/content/text.rs:398-410` — add `width`, `height`, `align` to
  `BufferBlockItem::Image`; update the manual `PartialEq` at `:414-435`, `as_markdown`
  (`:451+`), and `to_formatted_text_line` (`:496+`) to carry them.
- `crates/editor/src/content/core.rs:877-889` — forward `image.width/height/align` into
  `BufferBlockItem::Image`.
- `crates/editor/src/content/text.rs:496-500`, `markdown.rs:~1129` — serialization
  (see §5).
- `crates/ipynb_parser/src/lib.rs:217` — notebook image construction; add the
  `None/None/Left` defaults (notebook images have no HTML sizing).
- Any remaining destructure sites in `edit.rs`, `render/model/mod.rs`,
  `render/model/location.rs`, `selection.rs` — extend patterns with the new fields (or
  `..`). The style guide prefers exhaustive matches over `_` wildcards, so add explicit
  bindings.

### 4. Honor sizing and alignment in layout (the core behavior change)

In `crates/editor/src/content/edit.rs:721-746`, replace the hardcoded size with a
resolution against the new fields:

- Compute `available_width = layout.max_width() - spacing.x_axis_offset()` (as today).
- Resolve `width`:
  - `Some(Pixels(px))` → `min(px, available_width)` (invariant 4).
  - `Some(Percent(p))` → `available_width * p / 100` (invariant 5).
  - `None` → today's default (`available_width`, invariant 7).
- Resolve `height` analogously; the height budget for a `Percent` is the same
  `default_height` basis used today (`base_line_height * DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER`).
- Aspect ratio when only one dimension is set (invariant 6): because
  `Image::contain()` already scales by the smaller ratio inside the box
  (`image.rs:388-394`), passing a large/unconstrained value for the unspecified
  dimension yields proportional scaling. Concretely: if only `width` is specified, set
  `height` to a generous cap (e.g. `available_height` budget) so `contain` scales to
  the width; if only `height` is specified, cap `width` at `available_width`. This
  reuses the primitive's existing aspect-preservation rather than computing intrinsic
  ratios in the editor (which would require the decoded image, not available at layout
  time).

Add an `align: ImageAlign` (or a resolved horizontal offset) to `ImageBlockConfig`
(`render/model/mod.rs:1470-1475`). In `RenderableImage::paint`
(`crates/editor/src/render/element/image.rs`), offset the paint origin horizontally:
`Left` = no offset (today's behavior), `Center` = `(available_width - image_width)/2`,
`Right` = `available_width - image_width` (invariant 8). Left alignment must be
pixel-identical to today so untagged/`align="left"` images do not shift.

### 5. Serialization / round-trip

`BufferBlockItem::Image::as_markdown` and the HTML serializer
(`crates/editor/src/content/markdown.rs`) must preserve enough to reproduce the image
(invariant 14). Recommended canonical form:

- If `width`/`height`/`align` are all default (a Markdown image), serialize as today:
  `![alt](src "title")`.
- If any sizing attribute is present (an HTML image), serialize as a canonical
  `<img>` tag: `<img src="…" alt="…" title="…" width="…" height="…" align="…">`,
  emitting only the attributes that are set. Values go through the existing HTML
  attribute-escaping path so `"`, `<`, `>` are escaped, not interpolated raw
  (invariant 13). This mirrors how §6 of `specs/GH849/` handled title-aware
  serialization.

Add buffer-round-trip coverage that `<img src=… width="90%">` survives
markdown → `BufferBlockItem::Image` → markdown, and that a plain `![alt](src)` still
round-trips to the Markdown form (regression guard).

### 6. Security / sanitization

The parser is an **attribute allowlist**: only `src`, `alt`, `title`, `width`,
`height`, `align` are read; every other attribute (`onerror`, `onload`, `style`,
`srcset`, `usemap`, …) is parsed-and-discarded (invariant 13). No attribute value is
ever executed or used to navigate. `src` is resolved exclusively through the existing
`resolve_asset_source_relative_to_directory`
(`crates/editor/src/content/edit.rs:77-127`), which already handles `data:` / `http(s)`
/ absolute / relative sources — this change introduces no new source-resolution path,
so it inherits the viewer's existing asset-loading trust boundary. There is no
`<script>`, no event-handler surface, and no HTML injected into any web context; the
`<img>` tag is only ever interpreted structurally by the `nom` parser into a
`FormattedImage`.

### 7. Feature gating

No new feature flag is required. The `markdown_parser` change is unconditional (a
Markdown image without HTML attributes is unaffected). The layout change only alters
behavior for images that carry the new optional fields, which today can only originate
from a raw-HTML `<img>` tag. Existing Markdown-image behavior is unchanged by
construction.

## Testing and validation

### Unit tests — parser (`crates/markdown_parser/src/markdown_parser_tests.rs`)

Covers invariants 1–3, 8–13:

- `<img src="a.svg">` on its own line → `FormattedTextLine::Image` with that source,
  empty alt, `width/height = None`, `align = Left`.
- `<img src="a.svg" alt="Chart" title="T" width="90%">` → percent width, alt, title.
- `<img src="a.png" width="640" height="480">` → pixel width/height.
- `<img src="a.png" width="640px">` → `px` suffix parsed as pixels.
- `WIDTH`/`Width`/`ALIGN="Center"` → case-insensitive names and `align` value.
- `align="left|center|right"` → each alignment; unknown value → `Left`.
- `width="abc"`, `width=""`, `width="-40"` → dimension ignored (`None`), image still
  parses (invariant 12).
- `<img alt="x">` (no `src`) and `<img>` → parser fails, line renders as text
  (assert it becomes `FormattedTextLine::Line`, invariant 10).
- `text <img src="a.png"> more text` → renders as text, not image (invariant 11).
- Unterminated `<img src="a.png"` (no `>`, or `>` on next line) → text fallback.
- Ignored dangerous attributes: `<img src="a.png" onerror="x()">` parses to an image
  whose only carried attributes are the allowlisted ones (invariant 13).
- Regression: `![alt](src)` and `![alt](src "title")` parse exactly as before, with the
  new fields at their defaults.

### Unit tests — buffer round-trip (`crates/editor/src/content/text_tests.rs`, `core_tests.rs`)

Covers invariants 4–7, 14:

- `<img src="assets/chart.svg" width="90%">` → `BufferBlockItem::Image` with the parsed
  dimensions → re-serialized to a canonical `<img>` tag preserving `src`/`width`.
- Plain `![alt](src)` still round-trips to Markdown form (regression).
- A `data:` URI `<img>` above the size limit still hits the existing
  `IMAGE_TOO_LARGE_PLACEHOLDER` path (`core.rs:32-38`), unchanged.

### Unit tests — layout (`crates/editor/src/render/model/mod_tests.rs`)

Covers invariants 4–8:

- Pixel width smaller than the pane → `ImageBlockConfig.width == px`.
- Pixel width larger than the pane → clamped to `available_width`.
- Percent width → `available_width * p / 100`.
- No dimensions → identical `ImageBlockConfig` to today (regression against the
  hardcoded default).
- `align = Center/Right` → expected horizontal offset in the laid-out block.

### Integration / manual

Per CONTRIBUTING, include before/after screenshots in the PR: open a `.md` file in the
Markdown viewer containing the issue's motivating example
(`<img src="assets/chart.svg" alt="Pipeline Funnel" width="90%">`) plus a pixel-sized
image and a centered image; show the before (dropped / fixed-size) vs. after
(correctly sized and aligned) rendering. Add `crates/integration/` coverage for opening
a Markdown file whose content includes a sized `<img>` if the viewer flow is
exercisable there.

## Risks and follow-ups

- **Scope discipline:** this PR is deliberately only `<img>` sizing. The remaining tags
  in the issue (`<a>`, tables, `<details>`/`<summary>`, `<br>`, `<kbd>`, `<sub>`/`<sup>`,
  `<p/div align>`, `<picture>`) are follow-ups under the same issue. Landing this slice
  first delivers the motivating use case (sizing embedded SVG dashboards) and
  establishes the `FormattedImage`-field-threading + `<img>`-block-parser plumbing that
  later tags can reuse.
- **Aspect ratio at layout time:** the editor lays out before the image is decoded, so
  it cannot know intrinsic dimensions; single-dimension sizing relies on the primitive's
  `contain()` scaling. If a future change wants exact intrinsic-ratio sizing at layout
  time, it would need the decoded image size threaded back — out of scope here.
- **Honoring intrinsic SVG size with no attributes** (the other half of the issue's
  repro) is intentionally deferred: it changes default behavior for existing documents
  and deserves its own spec/PR.
