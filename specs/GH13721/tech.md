# TECH.md — Markdown viewer: honor raw-HTML `<img>` sizing (width/height/align)

Product spec: `specs/GH13721/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13721
Split out of: https://github.com/warpdotdev/warp/issues/13652

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
`warpui_core::elements::Image` primitive with `.contain()`
(`crates/warpui_core/src/elements/gui/image.rs:120-121`, `388-394`), which already
supports SVG via `usvg`/`resvg`
(`crates/warpui_core/src/image_cache.rs:274-283, 463, 472-476`). **This `.contain()`
call is not, by itself, an aspect-ratio mechanism for this feature**: it only fit-scales
the decoded image *within whatever box `ImageBlockConfig.width`/`.height` already are*
(`render/element/image.rs`'s `layout()` passes `SizeConstraint::new(vec2f(0.,0.), size)`
with `size = (config.width, config.height)` — the element's box, not a free constraint).
If those two config values aren't already the correct aspect-ratio-correct pair, no
amount of `.contain()` fixes that; it can only shrink-to-fit inside a wrong box. §4
below is the single source of truth for how `ImageBlockConfig.width`/`.height` are
derived to be aspect-ratio-correct in the first place (the Mermaid-precedent
`AssetCache` mechanism); `.contain()`'s role is unchanged from today — final
fit-clamping in case of any residual rounding, not ratio derivation.

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
- `crates/editor/src/content/mermaid_diagram.rs:54-107` — `mermaid_diagram_config` /
  `mermaid_diagram_size`: the existing precedent for layout-time intrinsic-ratio sizing
  from `AssetCache`, which §4 below reuses verbatim for `<img>` sizing.
- `crates/editor/src/render/model/mod.rs:1470-1475` — `ImageBlockConfig`.
- `crates/editor/src/render/model/positioned.rs:62-64, 202-204` — `Positioned::image()`
  and the generic `content_origin()` that `align` must override (§4).
- `crates/editor/src/render/model/bounds.rs:20-25` — `bounds::content_origin`, today's
  block-type-wide x-origin with no per-instance offset.
- `crates/editor/src/render/element/image.rs` — `RenderableImage` (drawing; the paint
  origin `align` must offset, per §4).
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

**The existing mechanism this reuses.** Plain Markdown images do not have a
precedent for intrinsic-ratio sizing today — `BufferBlockItem::Image`'s layout
(`edit.rs:726-751`) never queries the asset at all, it just always fills
`available_width` at a hardcoded height. But **Mermaid diagrams already solve exactly
this problem**, one block type over: `mermaid_diagram_size`
(`crates/editor/src/content/mermaid_diagram.rs:85-107`) queries
`AssetCache::load_asset::<ImageType>(asset_source)` *during the same layout pass* that
builds `ImageBlockConfig`, and when the asset is `AssetState::Loaded`, reads the
intrinsic size straight off the decoded data (`ImageType::Svg { svg }.size()`,
or generally `ImageType::image_size()` at `warpui_core/src/image_cache.rs:472-484`,
which also handles `StaticBitmap`/`AnimatedBitmap`) and computes
`height = width * intrinsic_height / intrinsic_width` (`mermaid_diagram.rs:104-106`).
When the asset is not yet `Loaded` (`Loading`/`FailedToLoad`/`Evicted`), it falls back
to a height-multiplier default (`mermaid_diagram_config`, `:54-71`) — the same shape of
fallback `BufferBlockItem::Image` already uses today, just parameterized instead of
hardcoded. This is a real, shipped, layout-time re-derivation, not a speculative
"generous cap": every time editor content layout re-runs (the same re-run that lets a
`Loading` Mermaid diagram flip to a rendered `MermaidDiagram` block once its asset
resolves — driven by the normal buffer/viewport invalidation path, not by the paint
layer's `repaint_after_load`), the image block re-queries `AssetCache` and gets a
better answer once decoded data exists. `<img>` sizing adopts this identical pattern
rather than inventing a new one.

(Note: `Image::layout_using_paint_bounds()` in
`crates/warpui_core/src/elements/gui/image.rs:153-161` looks like a shortcut but is
not — it only affects the paint element's own internal `size`, never wired into
`ImageBlockConfig`, and `RenderableImage` in `crates/editor/src/render/element/image.rs`
does not call it. Document-flow height, selection rects, and `align` offsets are all
read from `ImageBlockConfig.width`/`.height` on the content-model `BlockItem::Image`
(`render/model/mod.rs:4314,4375,4399`), so the fix must land in `edit.rs`'s layout
task, exactly where Mermaid's does, not in the paint-layer element.)

In `crates/editor/src/content/edit.rs:721-746`, replace the hardcoded size with a
resolution against the new fields:

- Compute `available_width = layout.max_width() - spacing.x_axis_offset()` (as today).
- Resolve `width`:
  - `Some(Pixels(px))` → `min(px, available_width)` (invariant 4).
  - `Some(Percent(p))` → `available_width * p / 100` (invariant 5).
  - `None` when the other axis is also `None` → today's default (`available_width`,
    invariant 7).
- Resolve `height` analogously; the height budget for a `Percent`, or for the
  neither-specified default, is the same `default_height` basis used today
  (`base_line_height * DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER`).
- **Aspect ratio when exactly one dimension is set (invariant 6):** resolve the
  specified axis per the rules above, then derive the other axis from the intrinsic
  ratio using the Mermaid mechanism verbatim:
  - Call `AssetCache::as_ref(app).load_asset::<ImageType>(asset_source.clone())`
    (the `asset_source` is already computed at this point via
    `resolve_asset_source`, `edit.rs:886`, so this requires no new resolution step).
  - `AssetState::Loaded { data }` with `data.image_size()` returning
    `Some((intrinsic_w, intrinsic_h))` with both `> 0`: derive the missing axis —
    given `width`, `height = width * intrinsic_h / intrinsic_w`; given `height`,
    `width = min(height * intrinsic_w / intrinsic_h, available_width)`.
  - `AssetState::Loading | FailedToLoad(_) | Evicted`, or `Loaded` with a
    zero/unreadable intrinsic size: the missing axis falls back to today's default for
    that axis (`available_width` for width, `default_height` for height) — a plain
    box, not a "cap" — until a later layout pass (triggered the same way a `Loading`
    Mermaid diagram's is) resolves it once the asset decodes. This is a real,
    bounded-in-time transient state, not a permanent behavior: it is one concrete
    layout in the "unresolved intrinsic size" case, not an approximation of one.
- **Both dimensions given:** no ratio math — each axis resolves independently per the
  rules above (invariant 6 only applies when exactly one axis is specified).
- **Percentage width with intrinsic ratio:** if `width` is `Percent` and `height` is
  unspecified, the percent is still resolved against `available_width` first (per
  invariant 5), then the derived `height` uses that resolved pixel width in the ratio
  formula above — percent sizing and aspect-ratio derivation compose rather than being
  mutually exclusive.

**Alignment: what layout must carry, and why `contain()`'s internal centering is not
in the way.** Alignment needs two things at paint time: (a) the block's available
content width, and (b) the actual displayed image bounds. Both already exist by this
point in layout — nothing new needs to be threaded in to know them:

- **(a) Available content width** is `available_width` from this same layout task
  (`layout.max_width() - spacing.x_axis_offset()`, computed above for width
  resolution) — the block's max width, already known at `ImageBlockConfig`
  construction.
- **(b) Displayed image bounds** are exactly `ImageBlockConfig.width`/`.height` as
  resolved by the rules above — by construction these are always the specified
  dimension exactly, and (per invariant 6) the intrinsic-ratio-correct derived
  dimension once the asset is `Loaded`, or today's plain default while it isn't.

**Why `Image::contain()`'s internal centering is not a conflict.**
`RenderableImage::layout()` (`crates/editor/src/render/element/image.rs:39-51`)
constructs the primitive as `Image::new(asset_source, CacheOption::BySize).contain()`
and lays it out with `SizeConstraint::new(vec2f(0., 0.), size)` where
`size = vec2f(config.width.as_f32(), config.height.as_f32())` — i.e. the primitive's
box *is* `ImageBlockConfig.width × .height`, not some larger constraint. Once §4's
sizing makes those two values the aspect-ratio-correct pair (the common case once the
asset is `Loaded`), `contain()` has zero slack to center within: the decoded image
already fills the box exactly, so the primitive's internal
centering/`top_aligned`/`right_aligned` logic in
`crates/warpui_core/src/elements/gui/image.rs` never has room to run. The only case
where the primitive's box and the decoded image's aspect ratio disagree is the
transient "asset not yet `Loaded`" fallback (§4's plain-default case) — during that
one layout pass `contain()` may show letterboxing inside the fallback box, which
self-corrects on the next layout pass exactly like Mermaid's transient case does.
Alignment therefore happens **one level up from the primitive**, at the block's paint
origin, not by fighting `contain()`'s behavior.

**Where the offset is applied.** Add `align: ImageAlign` to `ImageBlockConfig`
(`render/model/mod.rs:1470-1475`) and to `Positioned<ImageBlockConfig>`'s origin
computation. Today, `Positioned::image()` (`render/model/positioned.rs:202-204`) builds
its position via the generic `position_centered`, whose `content_origin()`
(`render/model/positioned.rs:62-64` → `bounds::content_origin`,
`render/model/bounds.rs:20-25`) returns `x = spacing.left_offset()` — a block-type-wide
constant with no per-instance horizontal offset. This is the gap: no block today can
shift itself independently within its available width. Fix: give `ImageBlockConfig`'s
positioning an `align`-aware x-origin — either a dedicated `Positioned<ImageBlockConfig>`
constructor (paralleling `image()`) that adds an alignment offset on top of
`bounds::content_origin`, or an equivalent adjustment applied where
`RenderableImage::paint` reads `positioned_image.content_origin()`
(`render/element/image.rs:66-74`). The offset itself uses the same slack-splitting
arithmetic as the align-blocks spec (GH13735; per-line/block offset applied at paint,
not at the primitive level — the same altitude this fix operates at):

- `Left` → offset `0` (today's behavior, pixel-identical — untagged/`align="left"`
  images do not shift).
- `Center` → offset `(available_width - config.width) / 2`.
- `Right` → offset `available_width - config.width`.

(invariant 8). This offset shifts only the block's own paint origin — selection rects
and the cursor position in `RenderableImage::paint` (`render/element/image.rs:75-95`),
which are derived from the same `content_origin()`, automatically follow the aligned
position with no separate change needed.

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
- `align = Left` → `Positioned<ImageBlockConfig>`'s x-origin is pixel-identical to the
  no-`align`-field baseline (regression guard against shifting untagged images).
- `align = Center` → x-origin offset equals `(available_width - config.width) / 2`.
- `align = Right` → x-origin offset equals `available_width - config.width`.
- `align = Center/Right` with a narrower-than-pane pixel width → offset uses the
  resolved (post-clamp) `config.width`, not the raw requested width.
- **Width-only + `AssetState::Loaded` intrinsic size** → `height` equals
  `width * intrinsic_h / intrinsic_w` (mirror the existing
  `mermaid_diagram_size` test coverage in `mermaid_diagram_tests.rs`, same formula,
  different block type).
- **Height-only + `AssetState::Loaded` intrinsic size** → `width` equals
  `height * intrinsic_w / intrinsic_h`, clamped to `available_width`.
- **Width-only + `AssetState::Loading`** (asset not yet decoded) → `height` falls back
  to the plain default (`default_height`), not a placeholder cap; re-running layout
  after the asset transitions to `Loaded` produces the ratio-derived height (regression
  guard against silently freezing on the fallback).
- **Width-only + `AssetState::Loaded` with zero/invalid intrinsic size** → falls back to
  `default_height` exactly like the `Loading` case (invariant 6 degenerate case).
- **Both `width` and `height` given** → no ratio math is applied; each axis resolves
  independently even if it does not match the intrinsic aspect ratio (regression guard
  against accidentally overriding an explicit two-dimension author intent).
- **`width="90%"` + intrinsic ratio** → `height` is derived from the *resolved pixel*
  width (`available_width * 90 / 100`), not from the unresolved percentage.

### Integration / manual

Per CONTRIBUTING, include before/after screenshots in the PR: open a `.md` file in the
Markdown viewer containing the issue's motivating example
(`<img src="assets/chart.svg" alt="Pipeline Funnel" width="90%">`) plus a pixel-sized
image and a centered image; show the before (dropped / fixed-size) vs. after
(correctly sized and aligned) rendering. Add `crates/integration/` coverage for opening
a Markdown file whose content includes a sized `<img>` if the viewer flow is
exercisable there.

## Risks and follow-ups

- **Scope discipline:** this PR is deliberately only `<img>` sizing. The other tags
  split out of the original bulk request #13652 (`<a>`, tables, `<details>`/`<summary>`,
  `<br>`, `<kbd>`, `<sub>`/`<sup>`, `<p/div align>`, `<picture>`/`<source>`) are tracked
  as their own focused issues. Landing this slice first delivers the motivating use case
  (sizing embedded SVG dashboards) and establishes the `FormattedImage`-field-threading +
  `<img>`-block-parser plumbing that later tags can reuse — notably #13736
  (`<picture>`/`<source>`), which is explicitly blocked on this issue for its fallback
  `<img>` path to mean anything.
- **Aspect ratio before the asset decodes:** single-dimension sizing (invariant 6) reads
  intrinsic size from `AssetCache` at layout time, exactly like `mermaid_diagram_size`
  (`mermaid_diagram.rs:85-107`). If the asset hasn't finished loading yet, the missing
  axis uses the plain default for one layout pass and self-corrects once the asset
  resolves and layout re-runs (the same self-correction Mermaid relies on today) — this
  is not a new invalidation mechanism, just a second consumer of an existing one.
- **Honoring intrinsic SVG size with no attributes** (the other half of the issue's
  repro) is intentionally deferred: it changes default behavior for existing documents
  and deserves its own spec/PR.
