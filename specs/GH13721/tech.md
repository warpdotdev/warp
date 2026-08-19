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
(`crates/warpui_core/src/elements/gui/image.rs:120-123`, `388-405`), which already
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
- `crates/editor/src/content/edit.rs:129-131` — `DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER`
  (the *unspecified-height default*, not a maximum) and a new
  `MAX_EXPLICIT_IMAGE_DIMENSION_PX` const (the sanity cap for an explicit pixel height;
  see §4 height rule).
- `crates/warpui_core/src/platform/mod.rs` — `max_texture_dimension_2d()`, the render
  model's GPU single-texture edge limit that grounds `MAX_EXPLICIT_IMAGE_DIMENSION_PX`.
- `crates/editor/src/content/mermaid_diagram.rs:54-107` — `mermaid_diagram_config` /
  `mermaid_diagram_size`: the existing precedent for layout-time intrinsic-ratio sizing
  from `AssetCache`, which §4 below reuses for `<img>` sizing.
- `crates/editor/src/render/model/mod.rs:3044-3067, 3102-3107` —
  `try_layout_pending_edits` (the gate that makes content layout a no-op on an empty
  queue) and `add_pending_edit` (how §4's relayout is queued).
- `crates/editor/src/content/buffer.rs:2611-2615` — `invalidate_layout_for_range`, the
  block-scoped no-op `EditDelta` builder §4's relayout uses.
- `crates/editor/src/content/hidden_lines_model.rs:175-178` — the existing
  `invalidate_layout_for_range` + `add_pending_edit` call shape to mirror.
- `crates/warpui_core/src/assets/asset_cache.rs:177-179` — `AssetHandle::when_loaded`,
  the completion future §4's relayout awaits.
- `crates/editor/src/search.rs:330-345` — the `ctx.spawn` precedent for async results
  updating editor state.
- `crates/warpui_core/src/elements/gui/image.rs:357-372` — `image_rect`, which centers
  the decoded image inside its box; the source of today's centered default (§4).
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
    pub align: ImageAlign,   // defaults to Center, matching today's rendering
}
```

Add two small public types in the same module:

```rust
pub enum ImageDimension {
    /// Absolute pixels, e.g. `width="640"` or `width="640px"`.
    Pixels(OrderedFloat<f32>),
    /// Percentage of the available content width, e.g. `width="90%"`.
    /// Only valid for `width`; a percentage `height` is rejected at parse time.
    Percent(OrderedFloat<f32>),
}

#[derive(Default)]
pub enum ImageAlign { Left, #[default] Center, Right }
```

`Center` is the default because Markdown images render centered today — see §4's
"Today's default is centered" for the mechanism and the behavior-preservation argument.

`FormattedImage` stays `Clone + Debug + PartialEq + Eq`-compatible with the rest of the
enum. Bare `f32` is not `Eq`, so the dimension payloads use `ordered_float::OrderedFloat<f32>`,
matching how the rest of the codebase carries `Eq`/`Hash`-able floats — `warpui_core`'s
`Lines(OrderedFloat<f64>)` (`crates/warpui_core/src/units.rs:30`),
`Scene`'s `font_size: OrderedFloat<f32>` (`crates/warpui_core/src/scene.rs:74`), and the
text-layout cache keys (`crates/warpui_core/src/text_layout.rs:319-326`). This keeps the
`Eq`/`Hash` derives on the surrounding types while leaving the parsed values in the same
float domain as the `Pixels` they resolve into, so no integer round-trip is needed.

`markdown_parser` does not currently depend on `ordered-float`
(`crates/markdown_parser/Cargo.toml`), so this change adds `ordered-float.workspace = true`
to that crate. The workspace already pins the crate for `warpui_core`
(`crates/warpui_core/Cargo.toml:60`) and `editor` (`crates/editor/Cargo.toml:31`).

`FormattedTextLine::Image` behavior is unchanged: `raw_text` stays `alt_text\n`,
`num_lines` stays `1`, and `compute_formatted_text_delta` needs no change (still a
derived structural compare).

Markdown `![alt](src)` images continue to construct `FormattedImage` with
`width: None, height: None, align: Center`, so their behavior is byte-for-byte unchanged.

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
  anything else / empty / negative → `None` (attribute ignored, invariant 12). This
  mirrors the WHATWG HTML "rules for parsing dimension values"
  (https://html.spec.whatwg.org/multipage/rendering.html#rules-for-parsing-dimension-values),
  the legacy algorithm browsers use for `<img>` `width`/`height` presentational
  attributes — percentages are part of that same algorithm (our percent support
  mirrors it for `width`), and a leading `-` is a parse error in both its absolute and
  percent forms, so no browser clamps a negative value; it's dropped, falling back to
  intrinsic/default sizing exactly as this spec does.
- **A percentage `height` is rejected**: `height` runs the shared parse and then discards
  an `ImageDimension::Percent` result, yielding `None` — the attribute is ignored exactly
  like `height="abc"` (invariant 5). Percentage *widths* are unaffected. This is a
  deliberate narrowing of the WHATWG algorithm, not an oversight: a percentage needs a
  reference dimension, and neither candidate reference is acceptable — the image block's
  default height is an internal implementation detail authors cannot reason about, and
  the viewport height would make document layout depend on window size at render time.
  Rejecting at parse means no percent-height value ever reaches layout, so §4 has no
  percent-height resolution rule to state.
- `align` parses case-insensitively to `Left`/`Center`/`Right`, defaulting to `Center`
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
  `None/None/Center` defaults (notebook images have no HTML sizing).
- Any remaining destructure sites in `edit.rs`, `render/model/mod.rs`,
  `render/model/location.rs`, `selection.rs` — extend patterns with the new fields (or
  `..`). The style guide prefers exhaustive matches over `_` wildcards, so add explicit
  bindings.

### 4. Honor sizing and alignment in layout (the core behavior change)

**The existing mechanism this reuses.** Plain Markdown images do not have a
precedent for intrinsic-ratio sizing today — `BufferBlockItem::Image`'s layout
(`edit.rs:721-746`) never queries the asset at all, it just always fills
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
hardcoded. `<img>` sizing adopts this query-at-layout-time pattern rather than inventing
a new one.

**Asset load does not, on its own, re-run content layout — this spec must add that
trigger.** Reusing Mermaid's *sizing* query is not enough, because the query only
produces a better answer when the content layout actually re-runs, and nothing in the
asset-load path makes it re-run. The measured pipeline:

- Content layout — the phase that runs `LayoutTask::from_styled_block` and builds
  `ImageBlockConfig` — executes only when a `PendingLayout` is queued.
  `RichTextElement::layout` (`crates/editor/src/render/element/mod.rs:1005-1011`) calls
  `RenderState::try_layout_pending_edits`, which early-outs when `pending_edits` is empty
  (`crates/editor/src/render/model/mod.rs:3044-3067`); only a non-empty queue reaches
  `layout_edit_delta` → `EditDelta::layout_delta` (`crates/editor/src/content/edit.rs:508-531`),
  the sole production caller of `from_styled_block`.
- The production producers of a `PendingLayout` are buffer edits
  (`EditorModel::update_content`, `crates/editor/src/model.rs:94-99`), the full-buffer
  `EditorModel::rebuild_layout` (`model.rs:103-113`, documented for font-size-class
  changes), hidden-line expand/collapse
  (`crates/editor/src/content/hidden_lines_model.rs:177, 192, 204` via
  `Buffer::invalidate_layout_for_range`, `crates/editor/src/content/buffer.rs:2611-2615`),
  and diff-view temporary blocks (`render/model/mod.rs:2928-2931`).
- The asset-load path reaches none of them. `Image::paint` calls
  `ctx.repaint_after_load(handle)` (`crates/warpui_core/src/elements/gui/image.rs:486-491`),
  which only inserts into `Presenter::pending_assets`
  (`crates/warpui_core/src/presenter.rs:631-633`). `AppContext::manage_pending_assets`
  (`crates/warpui_core/src/core/app.rs:3634-3684`) awaits `AssetHandle::when_loaded` and
  then sets `redraw_requested = true` + `update_windows()`. That reaches
  `Presenter::build_scene` (`presenter.rs:333-380`), which re-runs the **element tree's**
  `Element::layout` + `after_layout` + `paint` — but never touches `pending_edits`, so
  `ImageBlockConfig` is not rebuilt.

`repaint_after_load` is therefore necessary but **not sufficient** for this feature: it
correctly re-*paints* an already-sized box (which is why a Mermaid diagram the user
explicitly toggled to Rendered swaps its placeholder for the diagram — that path emits a
full `MermaidDiagram` with a real `ImageBlockConfig` up front, `edit.rs:796-810`, and
`RenderableMermaidDiagram::layout` registers a `.before_load()` placeholder inside the
same box, `crates/editor/src/render/element/mermaid.rs:88-115`). It cannot change the
box's *size*, which is what intrinsic-ratio derivation needs.

The `pending_mermaid_asset` field on `BlockItem::RunnableCodeBlock`
(`render/model/mod.rs:1183-1190`) was intended as this hook — its doc comment says the
view layer "can watch" it so "the layout will re-run" — but it has **no production
reader**; grep finds only the definition, the construction sites in `edit.rs`, and test
assertions. Auto-render Mermaid consequently self-corrects only when an unrelated edit
happens to queue a relayout. This spec does not inherit that gap.

**The mechanism this spec specifies.** When `LayoutTask::from_styled_block` sizes an
image that needs intrinsic dimensions (exactly one of `width`/`height` specified) and
finds the asset in `AssetState::Loading { handle }`, the editor spawns a task that awaits
the load and then queues a scoped relayout of that block:

1. Take the `AssetHandle` from the `Loading` state and build its completion future via
   `AssetHandle::when_loaded(asset_cache)`
   (`crates/warpui_core/src/assets/asset_cache.rs:177-179`) — the same future
   `manage_pending_assets` awaits, so it is proven to fire on decode completion.
2. Spawn it on the editor model's context with `ctx.spawn(future, |me, _, ctx| …)`, the
   pattern already used for async buffer search results
   (`crates/editor/src/search.rs:330-345`).
3. In the completion callback, queue a relayout scoped to the image's block:
   `Buffer::invalidate_layout_for_range(block_range)` (`buffer.rs:2611-2615`, which snaps
   the range to block boundaries) to build the no-op `EditDelta`, then
   `RenderState::add_pending_edit(delta, buffer_version)` (`render/model/mod.rs:3102-3107`)
   — the same two-call shape `hidden_lines_model.rs:175-178` uses. The next frame's
   `try_layout_pending_edits` then re-runs `from_styled_block` for that block, the
   `AssetCache` query returns `Loaded`, and the ratio-derived dimension replaces the
   fallback.

Two obligations come with owning the task rather than borrowing the presenter's:

- **Dedupe.** Spawning per layout pass would spawn a task per frame. The editor keeps a
  set of in-flight `(block, AssetHandle)` pairs and skips spawning when one is already
  pending, mirroring the `requested_repaint_after_load` guard on the `Image` primitive
  (`elements/gui/image.rs:68, 488-490`) and the `(window, asset)` dedupe in
  `manage_pending_assets` (`app.rs:3641-3644`).
- **Cancellation.** The spawned handle is dropped when the block is edited away or its
  asset source changes, so a stale completion cannot queue a relayout for a block that no
  longer exists. Storing the handle alongside the dedupe entry gives both properties from
  one structure.

Images that do **not** need intrinsic dimensions — both axes specified, or neither —
spawn nothing: their `ImageBlockConfig` is already final at first layout, and
`repaint_after_load` alone correctly fills the settled box.

**What a load *failure* looks like (`FailedToLoad` / `Evicted`), as opposed to
sizing.** The paragraph above is about how these states affect *sizing* (they fall back
to the default box). Their *visual* result is inherited unchanged from the pre-existing
Markdown-image path and is deliberately not modified by this spec: the block occupies a
box at its resolved size, but the `warpui_core` `Image` primitive paints **nothing**
into it when the load has failed and no backup element is registered
(`elements/gui/image.rs`: the `FailedToLoad` arm paints a `failed_to_load` element only
if one is set, the `Evicted` arm a `before_load` element only if one is set, and
`RenderableImage` — `render/element/image.rs` — registers neither). The user therefore
sees an **empty box at the configured size**: no placeholder graphic, no broken-image
icon, no alt text, no collapse to zero height, and no panic. It does *not* fall back to
literal `<img …>` text (that is only for a *parse*-invalid tag, invariant 10, a distinct
path). A raw-HTML `<img src="missing.png">` and a Markdown `![alt](missing.png)` behave
identically here, because both flow through the same `RenderableImage`. (This is also
distinct from the oversized-`data:`-URI guard, which replaces the image with the literal
text "Image too large to display" at the content level before layout — `core.rs:32-38`.)
Improving this to a real broken-image affordance (placeholder or alt text) is a possible
follow-up that would touch the shared Markdown-image path, out of scope here.

(Note: `Image::layout_using_paint_bounds()` in
`crates/warpui_core/src/elements/gui/image.rs:153-161` looks like a shortcut but is
not — it only affects the paint element's own internal `size`, never wired into
`ImageBlockConfig`, and `RenderableImage` in `crates/editor/src/render/element/image.rs`
does not call it. Document-flow height, selection rects, and `align` offsets are all
read from `ImageBlockConfig.width`/`.height` on the content-model `BlockItem::Image`
(`render/model/mod.rs:4064, 4125, 4149, 4415, 4474`), so the fix must land in `edit.rs`'s layout
task, exactly where Mermaid's does, not in the paint-layer element.)

In `crates/editor/src/content/edit.rs:721-746`, replace the hardcoded size with a
resolution against the new fields:

- Compute `available_width = layout.max_width() - spacing.x_axis_offset()` (as today).
- **One clamping rule, shared by every resolved-dimension path:** define
  `clamp_to_bound(px, bound) = px.clamp(1.0, bound.max(1.0))` — an absolute pixel value
  and a resolved percentage value both pass through this same function before becoming
  `ImageBlockConfig`'s field. `bound.max(1.0)` guards `f32::clamp`'s `min <= max`
  precondition for the degenerate case where the bound itself is sub-1px (a
  pathologically collapsed pane/container), so `clamp_to_bound` never panics; the result
  is still floored at `1px` in that case, consistent with the narrow-pane case below.
- Resolve `width`:
  - `Some(Pixels(px))` → `clamp_to_bound(px, available_width)` (invariant 4).
  - `Some(Percent(p))` → `clamp_to_bound(available_width * p / 100, available_width)`
    (invariant 5), where `p` is already non-negative by construction —
    `parse_image_dimension` rejects a negative percent at parse time
    (invariant 12; a negative percent never reaches this resolution step at all, it is
    `None` here exactly like an unparseable string). `width="200%"` still clamps to
    `available_width` (full width, same result as `width="100%"`), since
    `clamp_to_bound`'s upper bound is `available_width` regardless of how large the
    resolved pixel value is; `width="0%"` resolves to `0` and then floors at the
    `clamp_to_bound` minimum of `1px` (consistent with invariant 10's "never a
    blank/zero-size image box") — `0` is a valid, in-range percent, distinct from a
    negative one, which is invalid and ignored at parse.
  - `None` when the other axis is also `None` → today's default (`available_width`,
    invariant 7; already within bounds, `clamp_to_bound` is a no-op here).
- Resolve `height`. **The height and width bounds are deliberately *not* symmetric**,
  because their spatial semantics differ (guiding principle: *model any reasonable
  markdown file, not any possible HTML file*):
  - **Width** is bounded by `available_width` because horizontal space is a hard
    constraint — a pane has a finite width and horizontal overflow forces an
    unpleasant horizontal scroll. An explicit pixel width wider than the pane is
    therefore clamped down to the pane (invariant 4).
  - **Height** is *not* analogously bounded by `default_height`. `default_height`
    (`base_line_height * DEFAULT_IMAGE_HEIGHT_LINE_MULTIPLIER`) is the *fallback
    default size for an unspecified height* — it is **not** a maximum. Vertical space
    is free (the document scrolls), so an explicit, reasonable pixel height is
    **honored verbatim**: `<img height="480">` renders at 480px, not shrunk to ~200px.
    "The default when unspecified" and "the maximum when specified" are distinct roles,
    and `default_height` fills only the first.
  - `Some(Pixels(px))` → `clamp_to_bound(px, MAX_EXPLICIT_IMAGE_DIMENSION_PX)`. The
    only ceiling on an explicit pixel height is a **sanity cap for hostile/nonsense
    values** (e.g. `height="99999999"`), not a layout-driven maximum.
    `MAX_EXPLICIT_IMAGE_DIMENSION_PX` is grounded in the render model: it is the
    conservative floor of `max_texture_dimension_2d()` (the GPU's maximum single-texture
    edge; Metal guarantees ≥ 8192px, most report 16384). An image edge larger than that
    cannot be rasterized as one texture, so honoring a height beyond it is meaningless —
    that is the principled line between "reasonable" and "hostile." Recommended value:
    `8192.0` (the guaranteed floor, conservative across GPUs). Every reasonable markdown
    image — even a tall infographic — sits well under this cap; only pathological input
    reaches it. The `1px` floor from `clamp_to_bound` still applies.
  - `Some(Percent(_))` is **unreachable for height** — a percentage height is rejected in
    the parser (§2), so `height` is only ever `None` or `Some(Pixels(_))` by the time
    layout resolves it. There is no percent-height reference bound, and `default_height`
    is never used as a cap for any specified height.
  - `None` with `width` also `None` → `default_height` itself (the unspecified-height
    default; invariant 7).

  So `clamp_to_bound` is still the single shared clamp function; what differs per axis
  is only the *bound argument* — `available_width` for width (pixel or percent), and
  `MAX_EXPLICIT_IMAGE_DIMENSION_PX` for an absolute pixel height.
- **Aspect ratio when exactly one dimension is set (invariant 6):** resolve the
  specified axis per the rules above (already clamped), then derive the other axis
  from the intrinsic ratio using the Mermaid mechanism verbatim. The invariant that
  governs every sub-case below: **the author-specified dimension, once resolved and
  clamped, is never altered again by fallback or fit logic in any load state** —
  pre-decode and post-decode alike. Only the *derived* (unspecified) axis is ever
  adjusted layout-to-layout.
  - Call `AssetCache::as_ref(app).load_asset::<ImageType>(asset_source.clone())`.
    **Sequencing (implementable ordering).** This query needs two things at once: an
    `AppContext` (for `AssetCache::as_ref(app)`) and the resolved `asset_source`. In the
    pre-existing image path these were split across two phases — `asset_source` was only
    resolved later, in `LayoutTask::run`/`into_block_item` (`edit.rs:886`), a method
    with **no `AppContext`**, and *after* `ImageBlockConfig` was already constructed. So
    the config-construction phase could not have queried the cache the way this section
    requires. The fix (and the shape the implementation takes) is to **resolve the asset
    source earlier**: `LayoutTask::from_styled_block` already has `app` in scope (it is
    where Mermaid's own `mermaid_diagram_layout(&source, layout, spacing, app)` call
    lives), so `resolve_asset_source(&source, document_path)` is called there, *before*
    the `ImageBlockConfig` is built, and the resolved `AssetSource` is both (i) fed
    straight into this cache query for the intrinsic size and (ii) threaded onto the
    `LayoutTask::Image` variant so `into_block_item` reuses it instead of re-resolving.
    Net effect: the source is resolved exactly once, in the `AppContext`-bearing layout
    phase, which is the only phase that can perform this query — no second resolution,
    no `AppContext`-less call site trying to size the image.
  - **Post-decode — `AssetState::Loaded { data }` with `data.image_size()` returning
    `Some((intrinsic_w, intrinsic_h))` with both `> 0`:** derive the missing axis from
    the specified axis's *resolved* (post-clamp) value.
    - **Given `width`:** `height = width * intrinsic_h / intrinsic_w`. The derived
      *height* has no pane bound (vertical space is free — the document scrolls), and
      `width` is already ≤ `available_width` by construction, so this case cannot
      overflow the pane. The specified width keeps its resolved value.
    - **Given `height`:** `derived_width = height * intrinsic_w / intrinsic_h`. **When
      the derived width exceeds `available_width`, the box must scale down as a whole —
      it is NOT enough to clamp the width alone.** Clamping only the derived width
      (`min(derived_width, available_width)`) while leaving the specified height fixed
      would make the box `available_width × height`, which is *no longer
      aspect-ratio-correct* — a distortion. The **precedence is aspect ratio >
      pane-width bound > specified dimension**:
      - if `derived_width ≤ available_width` → honor the specified height exactly:
        `(derived_width, height)`.
      - else → **scale the whole box down uniformly** so it fits the pane:
        `width = available_width`, `effective_height = available_width * intrinsic_h /
        intrinsic_w`. The aspect ratio is preserved exactly (`width / effective_height
        == intrinsic_w / intrinsic_h`), the pane is never overflowed horizontally, and
        the *effective height is proportionally reduced below the specified value* —
        the specified height yields to the pane bound, which yields to the aspect ratio.
      This **mirrors the width side with the opposite trigger**: a too-wide specified
      *width* already clamps to `available_width` and derives the height *down* from
      there (same uniform-scale principle — the box is always the largest
      aspect-correct rectangle that fits the pane); the height-only overflow case is the
      same rule reached from the other axis. The derived height is intentionally not
      re-floored to `1px` in the extreme-ratio case, because that floor guards
      *author-specified* dimensions and re-applying it here would re-break the aspect
      ratio the scale-down exists to preserve.

      **Deliberate divergence from browsers.** A browser given `<img height="400">` on
      a narrow viewport lets the image overflow horizontally (and the page scrolls
      sideways). The Markdown-viewer pane model treats horizontal space as a **hard
      constraint** (per the width/height asymmetry rationale in the height-resolution
      rules above — horizontal overflow forces an unpleasant horizontal scroll, whereas
      vertical space is free), so it scales the box down instead of overflowing. This is
      the same "model any reasonable markdown file, not any possible HTML file"
      principle: a reasonable author height is honored right up to the point where it
      would break the pane, and past that the pane wins over the specified height rather
      than the layout breaking.

    The specified axis is otherwise not recomputed or reclamped at this point; it keeps
    the value resolved above (the height-only overflow case is the single exception,
    where the pane bound legitimately reduces it).
  - **Pre-decode — `AssetState::Loading | FailedToLoad(_) | Evicted`, or `Loaded` with
    a zero/unreadable intrinsic size:** this is the state that needs its own explicit
    contract, because a naive "derived axis gets a plain default box" description
    leaves a gap — see "Why the pre-decode fallback needs `stretch()`, not `contain()`"
    below. The specified axis keeps its resolved value unchanged (per the invariant
    above); the derived axis uses today's plain default for that axis
    (`available_width` for a derived width, `default_height` for a derived height). What
    changes is *how the element renders that box*: for this one transient layout,
    `RenderableImage::layout()` (`render/element/image.rs:39-51`) uses
    `Image::new(...).stretch()` instead of `.contain()` for this block. The relayout
    queued by the asset-load mechanism above re-resolves the box once the asset decodes,
    switching back to `.contain()` for the post-decode, aspect-ratio-correct box (which
    by construction has zero slack for `contain()` vs. `stretch()` to differ on — see
    below).
- **Why the pre-decode fallback needs `stretch()`, not `contain()`.** `Image::contain()`
  (`warpui_core/src/elements/gui/image.rs:120-123`) fit-scales the decoded image by the
  *smaller* of the box's width/height ratios — it shrinks-to-fit, it does not stretch
  either axis independently. If width is specified (say `640px`) and, pre-decode, the
  derived height falls back to `default_height`, the primitive's box is
  `640 × default_height`. Once the asset decodes on a *later* frame the box gets fixed,
  but the *fallback frame itself* renders through the exact same `contain()` call with
  no asset-size information yet — `Image::new(..).contain()` fit-scales the placeholder
  content (or, once loaded on this same pass in a race, the real decoded image) to
  whichever of the two axes is more constraining. If `default_height` happens to be
  short relative to the eventual intrinsic ratio, `contain()` can shrink the *displayed
  width* below `640px` for that frame — a visible, if transient, violation of "the
  requested width is honored" (invariant 6), not merely cosmetic letterboxing. Using
  `.stretch()` (`warpui_core/src/elements/gui/image.rs:126-129`, already a first-class
  `FitType` alongside `Contain`/`Cover`) for this one fallback frame fills the
  `640 × default_height` box on both axes independently, so the specified axis (width)
  renders at exactly its resolved value — the *only* axis this spec makes a promise
  about pre-decode — while the derived axis (height) is a guess either way and
  stretching it introduces no new distortion the fallback box wasn't already going to
  have. This makes the invariant ("specified dimension exact in every load state")
  literally true instead of true-only-once-decoded.
- **Both dimensions given:** no ratio math, and no `AssetCache` query — each axis
  resolves independently per the clamp rules above (invariant 6 only applies when
  exactly one axis is specified). `RenderableImage` uses `.contain()` as today; since
  both axes are author-specified there is no fallback frame to reason about.
- **Percentage width with intrinsic ratio:** if `width` is `Percent` and `height` is
  unspecified, the percent is still resolved (and clamped) against `available_width`
  first (per invariant 5), then the derived `height` uses that resolved pixel width in
  the ratio formula above — percent sizing and aspect-ratio derivation compose rather
  than being mutually exclusive.
- **Percentage height has no intrinsic-ratio case**, because it never reaches layout:
  `height="50%"` is rejected in the parser (§2) and arrives here as `None`. Such an image
  is therefore either width-only (if a width was given, taking the width-with-derived-height
  path above) or fully unspecified (taking the default-sizing path, invariant 7).
- **Zero/near-zero `available_width` (narrow pane or deeply nested constrained
  container):** `clamp_to_bound`'s `1.0` floor means a percent or pixel
  width never resolves to `0` or negative regardless of how small `available_width` is;
  a pathologically narrow pane renders a 1px-wide image rather than panicking on a
  degenerate `SizeConstraint` or dividing by zero in the ratio formula (the ratio
  formula's denominator is always the *intrinsic* width/height from decoded asset data,
  never `available_width`, so a narrow pane cannot introduce a divide-by-zero there
  either).

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
  dimension once the asset is `Loaded`, or today's plain default (rendered via
  `.stretch()`, not `.contain()`, per the pre-decode sub-case above) while it isn't.

**Today's default is centered, and `Center` must stay the default.** This is a
behavior-preservation constraint, not a free design choice. `RenderableImage::layout()`
(`crates/editor/src/render/element/image.rs:39-51`) constructs the primitive as
`Image::new(asset_source, CacheOption::BySize).contain()` and lays it out with
`SizeConstraint::new(vec2f(0., 0.), size)` where
`size = vec2f(config.width.as_f32(), config.height.as_f32())`. Today `config.width` is
always the full `available_width` (`edit.rs:721-746`), and `RenderableImage` sets neither
`top_aligned` nor `right_aligned`, so the primitive's `image_rect()`
(`crates/warpui_core/src/elements/gui/image.rs:357-372`) takes its final branch and
offsets the decoded image by `(size - logical_image_size) / 2.0` — **centered on both
axes** inside the full-width box. A Markdown `![alt](src)` image narrower than the pane
therefore renders horizontally centered today.

`ImageAlign::default()` is accordingly **`Center`**, not `Left`: an `<img>` with no
`align` attribute, and every existing Markdown image, must keep rendering centered.

This interacts with §4's sizing change in a way the implementation must handle
explicitly. Once sizing makes `config.width` the *author-specified* width rather than the
full `available_width`, the primitive's box shrinks to the image, `contain()` has no slack
left inside it, and the primitive's own centering stops producing any offset. The
centering that `contain()` provides today must therefore be **re-established one level
up**, at the block's paint origin, using the same offset arithmetic as `Center` below. Concretely: the block box is positioned within `available_width` by the
align offset, and the image fills the block box. For an image whose resolved width is the
full `available_width` (the no-attribute default, invariant 7) the `Center` offset is `0`
and the result is pixel-identical to today either way.

The one case where the primitive's box and the decoded image's aspect ratio still
disagree is the transient "asset not yet `Loaded`" fallback with exactly one dimension
specified — that case is switched to `.stretch()` above, so it does not letterbox or
shrink the specified axis. (An author who specifies *both* `width` and `height` with a
mismatched aspect ratio, per the "both dimensions given" case above, keeps `.contain()`
and can see legitimate letterboxing within the block box — that is direct author intent.
The block box itself is still placed by the `align` offset.)

**Where the offset is applied.** Add `align: ImageAlign` to `ImageBlockConfig`
(`render/model/mod.rs:1470-1475`). **Also store the available content width on the
config** as an `align_available_width: Pixels` field, for the same phase-crossing
reason as the asset-source sequencing above: `available_width` is computed at layout
time (`from_styled_block`), but the alignment offset is applied at *paint* time, and
the paint layer (`RenderableImage::paint`) does not otherwise have access to the layout
task's `available_width`. Capturing it on the config at construction is what lets paint
compute `available_width - config.width` without re-deriving a value it cannot see.
(`config.width` itself is already on the config, so only the available width needs to
ride along.) Then adjust `Positioned<ImageBlockConfig>`'s origin computation. Today,
`Positioned::image()` (`render/model/positioned.rs:202-204`) builds its position via the
generic `position_centered`, whose `content_origin()`
(`render/model/positioned.rs:62-64` → `bounds::content_origin`,
`render/model/bounds.rs:20-25`) returns `x = spacing.left_offset()` — a block-type-wide
constant with no per-instance horizontal offset. This is the gap: no block today can
shift itself independently within its available width. Fix: give `ImageBlockConfig`'s
positioning an `align`-aware x-origin — either a dedicated `Positioned<ImageBlockConfig>`
constructor (paralleling `image()`) that adds an alignment offset on top of
`bounds::content_origin`, or an equivalent adjustment applied where
`RenderableImage::paint` reads `positioned_image.content_origin()`
(`render/element/image.rs:66-74`), reading `config.align_available_width` for the slack. The offset itself uses the same slack-splitting
arithmetic as the align-blocks spec (GH13735; per-line/block offset applied at paint,
not at the primitive level — the same altitude this fix operates at):

- `Left` → offset `0`.
- `Center` (**the default**) → offset `(available_width - config.width) / 2`. For an
  image at the full `available_width` this is `0`, so untagged full-width images are
  pixel-identical to today.
- `Right` → offset `available_width - config.width`.

(invariant 8). This offset shifts only the block's own paint origin — selection rects
and the cursor position in `RenderableImage::paint` (`render/element/image.rs:75-95`),
which are derived from the same `content_origin()`, automatically follow the aligned
position with no separate change needed.

### 5. Serialization / round-trip

`BufferBlockItem::Image::as_markdown` and the HTML serializer
(`crates/editor/src/content/markdown.rs`) must preserve enough to reproduce the image
(invariant 14). Recommended canonical form:

- If `width` and `height` are both `None` and `align` is the default (`Center`) — the
  shape every Markdown `![alt](src)` image has — serialize as today:
  `![alt](src "title")`.
- If any sizing attribute is present, or `align` is `Left`/`Right`, serialize as a
  canonical `<img>` tag: `<img src="…" alt="…" title="…" width="…" height="…" align="…">`,
  emitting only the attributes that are set. Because `Center` is the default, an explicit
  `align="center"` round-trips to the Markdown form when no dimension is set; the
  rendering is identical, so no author intent is lost. Values go through the existing HTML
  attribute-escaping path so `"`, `<`, `>` are escaped, not interpolated raw
  (invariant 13). This mirrors how §6 of `specs/GH849/` handled title-aware
  serialization.

Add buffer-round-trip coverage that `<img src=… width="90%">` survives
markdown → `BufferBlockItem::Image` → markdown, and that a plain `![alt](src)` still
round-trips to the Markdown form (regression guard).

### 6. Security / sanitization

The parser is an **attribute allowlist**: only `src`, `alt`, `title`, `width`,
`height`, `align` are read; every other attribute (`onerror`, `onload`, `style`,
`usemap`, …) is parsed-and-discarded (invariant 13). No attribute value is
ever executed or used to navigate. (`srcset` is likewise not read, but that is a
*feature deferral* to `<picture>`/`<source>` (#13736), not a security exclusion like the
event-handler attributes — see the responsive-image non-goal in product.md. Mechanically
it is discarded by the same allowlist, but it does not belong in the same conceptual
bucket as `onerror`/`onload`.) `src` is resolved exclusively through the existing
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

Covers invariants 1–3, 5, 8–13:

- `<img src="a.svg">` on its own line → `FormattedTextLine::Image` with that source,
  empty alt, `width/height = None`, `align = Center` (the default).
- `<img src="a.svg" alt="Chart" title="T" width="90%">` → percent width, alt, title.
- `<img src="a.png" width="640" height="480">` → pixel width/height.
- `<img src="a.png" width="640px">` → `px` suffix parsed as pixels.
- `WIDTH`/`Width`/`ALIGN="Center"` → case-insensitive names and `align` value.
- `align="left|center|right"` → each alignment; unknown value → `Center` (the default).
- `width="abc"`, `width=""`, `width="-40"`, `width="-10%"` → dimension ignored
  (`None`), image still parses (invariant 12; negative is rejected uniformly for both
  the pixel and percent forms — there is no negative-percent special case).
- `height="50%"`, `height="150%"` → `height` is `None` (percent heights rejected,
  invariant 5), while `<img src="a.png" width="90%" height="50%">` still yields
  `width = Some(Percent(90))` — the rejection is height-only and does not poison the
  width on the same tag.
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
- **`width="200%"`** → clamps to `available_width` (invariant 5, same result as
  `width="100%"`), not an unclamped `2 * available_width` overflow.
- **`width="0%"`** → resolves to a valid, in-range `0` percent, then floors at the
  `clamp_to_bound` minimum of `1px`, never a zero-size box (invariant 10's "never a
  blank/zero-size image box" applies to percentages too, not just the
  unparseable-attribute case).
- **`width="-10%"`** — a parser-level case, not a layout-level one: per invariant 12,
  `parse_image_dimension` rejects any negative value (percent or pixel) at parse time,
  so the attribute never reaches this resolution step at all — it is `None`, identical
  to `width="abc"`, and the image falls back to default sizing for that axis
  (invariant 7). See the parser test coverage above; this file does not re-clamp a
  negative percent to `1px`.
- **`height="150%"`, `height="50%"`** — parser-level cases, not layout-level ones: a
  percentage height is rejected at parse (§2, invariant 5), so `height` arrives at layout
  as `None` and the image falls back to default height sizing (invariant 7) or to the
  aspect-derived height when a width is given. A percentage *width* on the same tag is
  unaffected. See the parser test coverage above.
- **Explicit pixel height is honored, NOT clamped to `default_height`** — the
  reasonable-markdown boundary cases, each of which gets an explicit unit test with a
  justification comment stating why the boundary sits where it does:
  - `height="480"` with `default_height` ≈ 200px → resolves to **480** (honored
    verbatim, not clamped down to `default_height`). Justification: 480px is a
    reasonable image height and vertical space is free (the doc scrolls), so there is
    no reason to shrink it.
  - `height="8192"` (== `MAX_EXPLICIT_IMAGE_DIMENSION_PX`) → resolves to **8192**
    (the sanity cap is inclusive; the largest guaranteed single-texture edge is still
    a real, renderable height).
  - `height="99999999"` (hostile) → clamps to `MAX_EXPLICIT_IMAGE_DIMENSION_PX`
    (**8192**). Justification: beyond the GPU single-texture ceiling the height cannot
    be rasterized as one texture, so it is the principled "unreasonable/hostile" line —
    not a layout-driven maximum.
  - `height="1"` → resolves to **1** (the `clamp_to_bound` `1px` floor; a 1px image is
    degenerate-but-valid, never a zero box).
- **`width="200%"` with `height="600"`** → the width clamps to `available_width` and the
  pixel height is honored at 600; each axis resolves against its own rule and no ratio
  math applies (both dimensions given).
- **No `align` attribute, image narrower than the pane** → renders **centered**, matching
  the pre-change rendering. This is the load-bearing regression guard for the alignment
  default: assert the drawn image's x-origin equals
  `(available_width - config.width) / 2`, and compare it against a baseline capture of
  today's `contain()`-centered output for the same image and pane width. A test asserting
  a left-flush origin here would be asserting a behavior regression.
- **No `align` attribute, image at full `available_width`** →
  `Positioned<ImageBlockConfig>`'s x-origin is pixel-identical to the
  no-`align`-field baseline (the `Center` offset is `0` at full width).
- `align = Center` → x-origin offset equals `(available_width - config.width) / 2`.
- `align = Right` → x-origin offset equals `available_width - config.width`.
- `align = Left` → x-origin offset equals `0` (flush to `bounds::content_origin`); assert
  this differs from the default for a narrower-than-pane image, so `align="left"` is
  proven to be a real opt-out from the centered default rather than a no-op.
- `align = Center/Right` with a narrower-than-pane pixel width → offset uses the
  resolved (post-clamp) `config.width`, not the raw requested width.
- **Width-only + `AssetState::Loaded` intrinsic size** → `height` equals
  `width * intrinsic_h / intrinsic_w` (mirror the existing
  `mermaid_diagram_size` test coverage in `mermaid_diagram_tests.rs`, same formula,
  different block type).
- **Height-only + `AssetState::Loaded`, derived width fits the pane** → height honored
  exactly; `width` equals `height * intrinsic_w / intrinsic_h` (≤ `available_width`).
  A justification-commented boundary test asserts the specified height is unchanged.
- **Height-only + `AssetState::Loaded`, derived width would overflow** → the box scales
  down uniformly: `width == available_width`
  and `effective_height == available_width * intrinsic_h / intrinsic_w` (below the
  specified height). A boundary test (justification-commented, on the 3:1 wide shape)
  asserts: `width == available_width`, aspect ratio preserved (`width / height ==
  intrinsic_w / intrinsic_h`), and the effective height is strictly less than the
  specified height. A second test at the exact boundary (`derived_width ==
  available_width`) asserts the height is honored (the `<=` branch, not scale-down).
- **Width-only + `AssetState::Loading`** (asset not yet decoded) → `height` falls back
  to the plain default (`default_height`), not a placeholder cap; re-running layout
  after the asset transitions to `Loaded` produces the ratio-derived height (regression
  guard against silently freezing on the fallback).
- **Width-only + `AssetState::Loaded` with zero/invalid intrinsic size** → falls back to
  `default_height` exactly like the `Loading` case (invariant 6 degenerate case).
- **Both `width` and `height` given** → no ratio math is applied; each axis resolves
  independently even if it does not match the intrinsic aspect ratio (regression guard
  against accidentally overriding an explicit two-dimension author intent). `RenderableImage`
  uses `.contain()`, not `.stretch()`, for this case (no fallback frame to reason about).
- **`width="90%"` + intrinsic ratio** → `height` is derived from the *resolved pixel*
  width (`available_width * 90 / 100`), not from the unresolved percentage.
- **`width="90%" height="50%"`** → the percent width resolves normally; the percent
  height is `None` from the parser, so the image takes the width-only path and derives
  its height from the intrinsic ratio (invariant 5 + 6 composed).
- **Width-only + `AssetState::Loading` → element fit mode** → assert
  `RenderableImage::layout()` constructs the primitive with `.stretch()`, not
  `.contain()`, while the derived height is still the plain `default_height` fallback;
  assert the *specified* width equals the resolved value exactly (not shrunk by any
  fit-scaling) even when `default_height` implies a different aspect ratio than the
  eventual intrinsic size. This is the regression guard for the pre-decode
  width-guarantee hole.
- **Same asset transitions `Loading` → `Loaded` across two layout passes** → first pass
  uses `.stretch()` with the plain-default derived axis; second pass uses `.contain()`
  with the intrinsic-ratio-derived axis; the specified axis's value is identical across
  both passes (never recomputed once resolved).
- **Zero/near-zero `available_width`** (e.g. a deeply nested constrained container) →
  a percent or pixel width still resolves to at least `1px`, no panic, no
  `NaN`/divide-by-zero in the ratio-derivation formula.

### Unit tests — asset-load relayout trigger (`crates/editor/src/content/edit_tests.rs`)

Covers the §4 relayout mechanism. `edit_tests.rs:301, 531, 592` already drive
`AssetState::Loading { handle } => handle.when_loaded(asset_cache)` to completion inside a
test, so this pattern has working precedent to build on.

- **Width-only image, asset `Loading` → completes** → a pending edit is queued for that
  block after the `when_loaded` future resolves, and the subsequent layout produces the
  ratio-derived height. This is the core test: without the trigger the block would keep
  its `default_height` fallback forever, so an assertion on the *queue* (not just on a
  manually re-run layout) is what proves the mechanism.
- **Both dimensions specified, asset `Loading`** → **no** task is spawned and **no**
  pending edit is queued; the box is already final. Guards against relayout churn for
  images that do not need intrinsic size.
- **No dimensions specified, asset `Loading`** → likewise no spawn, no pending edit
  (today's behavior, unchanged).
- **Repeated layout passes while the same asset is still `Loading`** → exactly one task is
  in flight; the second and third passes do not spawn duplicates (the dedupe guard).
- **Block edited away before the asset resolves** → the completion does not queue a
  pending edit for the removed block, and does not panic (the cancellation guard).
- **Asset `FailedToLoad`** → no task spawned (there is nothing to wait for) and the block
  keeps its fallback box, per the load-failure visual described in §4.

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
  (derived) axis uses the plain default for one layout pass, rendered via `.stretch()`
  rather than `.contain()` so the *specified* axis is never shrunk by fit-scaling during
  that transient frame, then resolves to the intrinsic-ratio-derived value (and back to
  `.contain()`) on the relayout the asset-load mechanism in §4 queues.
- **This PR adds the editor's first asset-load-driven relayout trigger**, and that is the
  riskiest part of the change. `repaint_after_load` re-paints but does not rebuild
  `ImageBlockConfig`, and the `pending_mermaid_asset` field that was meant to fill this
  role has no production reader, so there is no existing hook to borrow — §4 specifies
  spawning a `when_loaded` task that queues a block-scoped `add_pending_edit`. The
  failure modes to watch in review are a spawn-per-frame loop (guarded by the in-flight
  dedupe set) and a stale completion queuing a relayout for a block that has since been
  edited away (guarded by dropping the task handle). A follow-up could generalize this
  into the shared hook `pending_mermaid_asset` anticipated, which would let auto-render
  Mermaid stop depending on an incidental relayout — out of scope here.
- **Honoring intrinsic SVG size with no attributes** (the other half of the issue's
  repro) is intentionally deferred: it changes default behavior for existing documents
  and deserves its own spec/PR.
