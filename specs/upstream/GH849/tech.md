# CommonMark Image Title and Alt Text Fallback in Block-List Image Rendering — Tech Spec
Product spec: `specs/GH849/product.md`
GitHub issue: https://github.com/warpdotdev/warp-external/issues/849

## Context
The AI block-list already renders markdown images behind the `BlocklistMarkdownImages` flag via the architecture described in `specs/zachlloyd/inline-markdown-images-in-blocklist/TECH.md`. That rollout parses images through `markdown_parser`, carries them on `AgentOutputImage`, and renders them through a small set of section renderers in `view_impl/common.rs`. Two pieces of the pipeline currently drop information required by the titled-image flow:

- The image parser must accept titled images without changing link parsing. `parse_link_target` remains link-only, so the image path needs its own `parse_image_target` / `parse_image_destination` / `parse_image_title` helpers that can parse a destination, optional title, and closing `)` without broadening link behavior.
- `FormattedImage` in `crates/markdown_parser/src/lib.rs:330-334` models only `alt_text` and `source`. Every downstream consumer — the buffer item in `crates/editor/src/content/text.rs:431-498`, the editor HTML serializer at `crates/editor/src/content/markdown.rs:787-796`, the AI block-list section model in `app/src/ai/agent/mod.rs:1355-1396`, and the renderer at `app/src/ai/blocklist/block/view_impl/common.rs:1700-1900` — therefore cannot see the title, cannot serialize it back into markdown or HTML, and cannot use it as a tooltip. The renderer's load-failure branches also always emit the raw markdown source regardless of whether alt was authored.

The narrowest fix is to extend the shared parser model with a `title: Option<String>`, thread that field through every producer/consumer of `FormattedImage` and `AgentOutputImage`, and wire the block-list renderer to use title as a tooltip and alt as a load-failure fallback. The final implementation keeps all image-line parsing in `markdown_parser`; the AI block-list layer consumes the shared helper rather than maintaining a second inline-image parser.

Relevant code:

- `crates/markdown_parser/src/markdown_parser.rs` — `parse_image`, `parse_image_prefix`, `parse_image_run_line`, `parse_image_target`, `parse_image_destination`, and `parse_image_title`.
- `crates/markdown_parser/src/markdown_parser.rs (978-1075)` — `parse_link_target`, which remains unchanged because link titles are still out of scope.
- `crates/markdown_parser/src/markdown_parser_test.rs (2171-2320)` — current parser coverage for images.
- `crates/markdown_parser/src/lib.rs (330-334)` — `FormattedImage` model.
- `crates/markdown_parser/src/lib.rs (161-294)` — `FormattedTextLine::Image` raw-text and line-count behavior.
- `crates/editor/src/content/text.rs (420-500)` — `BufferBlockItem::Image`, `as_markdown`, `as_rich_format_text`, `to_formatted_text_line`.
- `crates/editor/src/content/text.rs (573-583)` — `BufferText::BlockItem` debug formatting for images.
- `crates/editor/src/content/markdown.rs (787-796)` — HTML serialization of buffer image blocks.
- `crates/editor/src/content/core.rs:843`, `crates/editor/src/content/edit.rs:689`, `crates/editor/src/content/edit.rs:779`, `crates/editor/src/render/element/image.rs`, `crates/editor/src/render/model/mod.rs (~3488-3900)` — editor render/layout sites that destructure `BufferBlockItem::Image` and must continue to compile.
- `app/src/ai/agent/mod.rs (1355-1396)` — `AgentOutputImage`, `AIAgentTextSection::Image`.
- `app/src/ai/agent/util.rs` — `flush_plain_text_sections`, `image_section`, and `markdown_source_for_image`, which now consume the shared parser helpers instead of implementing image parsing locally.
- `app/src/ai/agent/util_tests.rs` — parser tests for block-list markdown section extraction.
- `app/src/ai/blocklist/block/view_impl/common.rs (1640-1970)` — inline-image row, block-image group, mermaid card, and shared `render_visual_markdown_block` helpers.
- `app/src/ai/blocklist/block/view_impl/common_tests.rs` — block-list section renderer tests.
- `app/src/ai/blocklist/block.rs (360-430, 2291-2316)` — `AIBlockStateHandles` and the per-section handle-allocation fold where new image tooltip handles must also be pre-allocated so they persist across frames.
- `app/src/ai/blocklist/block/view_impl/output.rs (278-317)` — the `TextSectionsProps` construction that threads the pre-allocated handles into the renderer.
- `crates/ui_components/src/tooltip.rs` — reusable tooltip primitive to wrap rendered images. Backed by `warp_core::ui::builder::tool_tip_on_element`, which constructs a `Hoverable::new(mouse_state_handle, ...)` that reads `state.is_hovered()`. The mouse state handle must therefore live across frames; passing a fresh `MouseStateHandle::default()` per render causes `is_hovered()` to always return `false` and the tooltip to never appear.
- `app/src/ai/agent_sdk/driver/output.rs (1260-1290)` — image section plumbing used by the agent SDK driver (needs field parity).

## Proposed changes

### 1. Extend the shared parser model with a `title` field
Add `pub title: Option<String>` to `FormattedImage` in `crates/markdown_parser/src/lib.rs`. Treat `None` as "no title" and `Some(empty)` as equivalent to no title; normalize an empty parsed title to `None` inside `parse_image` so all downstream consumers can use a single `Option::is_some` check to decide whether to render a tooltip or emit an HTML `title="..."`.

Update `FormattedTextLine::Image` handling at the same time:

- `FormattedTextLine::raw_text` stays as `alt_text\n` — title is not part of the plain-text projection.
- `FormattedTextLine::num_lines` stays at 1.
- `compute_formatted_text_delta` needs no special-case work because `FormattedImage` is derived `Eq`.

### 2. Teach the image parser to accept an optional CommonMark title
Update the image parser in `crates/markdown_parser/src/markdown_parser.rs` so the image path uses its own `parse_image_target` / `parse_image_destination` / `parse_image_title` helpers instead of routing through `parse_link_target`. This keeps link parsing unchanged while allowing images to parse an optional title before the closing `)`. The image-specific helpers should:

- consume the opening `(` at the image call site, parse the destination, optionally parse the title, and then consume the closing `)`.
- consume at least one space/tab between the destination and the title delimiter. This is an intentional narrowing for this feature; a line ending between destination and title remains unsupported and falls back to plain text.
- accept one of the three CommonMark title delimiters — `"…"`, `'…'`, `(…)` — with matching closing delimiter.
- honor the CommonMark escape rule (`\"`, `\'`, `\)`) by decoding into the literal string.
- reject titles that cross a line ending without closing (fall back to plain text for the whole image, matching invariant 5 from the product spec).
- consume optional trailing whitespace and then the closing `)`.
- normalize only the truly empty title to `None`; whitespace inside a non-empty title is preserved literally.

`parse_link_target` stays unchanged and continues to own only the link destination grammar. This keeps link-title parsing out of scope, which matches product invariant 13.

Add tests to `crates/markdown_parser/src/markdown_parser_test.rs` covering:

- double-quoted, single-quoted, and paren-wrapped titles
- empty titles in each form
- titles with escaped matching delimiters
- titles with unclosed delimiters (fall back to plain text)
- titles with embedded line endings (fall back to plain text)
- no-title case continues to parse identically to today (regression guard)

### 3. Update every `FormattedImage` construction and destructure site
`grep`-driven work: update every place that constructs or pattern-matches `FormattedImage { alt_text, source }` to also carry `title`. The known sites are in `Relevant code` above; none of them have interesting logic to change — they just need to preserve the field. Notable details:

- `crates/editor/src/content/text.rs (471-484)` — `BufferBlockItem::Image { alt_text, source, title }` must be added; `as_markdown` re-serializes with the title when present, `as_rich_format_text` does the same, `to_formatted_text_line` forwards the field, and `BufferText::BlockItem` Debug formatting includes it when present (so buffer debug output disambiguates titled images from untitled images).
- `crates/editor/src/content/markdown.rs (787-796)` — emit `title="…"` on the `<img>` tag only when the parsed title is non-empty. Use the existing attribute-building pattern to avoid introducing a second serialization path.
- Editor render sites that destructure `BufferBlockItem::Image` (`core.rs`, `edit.rs`, `render/element/image.rs`, `render/model/mod.rs`, `render/model/location.rs`, `render/model/debug.rs`) only need to accept the new field (`..` patterns or explicit unused binding); the editor render of the inline image itself is out of scope for this issue.

Add buffer-round-trip unit coverage (in the existing `crates/editor/src/content/text_tests.rs` / `content/edit_tests.rs` pattern) confirming that `![alt](src "title")` survives the markdown → buffer → markdown path.

### 4. Extend `AgentOutputImage` and the block-list parser helpers
Add `pub title: Option<String>` to `AgentOutputImage` in `app/src/ai/agent/mod.rs:1355-1361` and make sure `image_section` in `app/src/ai/agent/util.rs` forwards it. Update `markdown_source_for_image` so that when `title` is present the serialized form is `![alt](source \"title\")`. The current implementation uses the shared `format_image_markdown` helper from `crates/editor/src/content/text.rs`, which canonically re-serializes titled images with double quotes and escapes literal `\"` so markdown round-trips preserve the parsed title content.

The AI block-list layer no longer owns separate image parsing helpers. `flush_plain_text_sections` now delegates to the shared `parse_image_run_line` helper in `markdown_parser`, then chooses block vs. inline layout based on the returned image count. That keeps all image-line parsing logic in one place.

Update the block-list parser tests in `app/src/ai/agent/util_tests.rs` with:

- a single-line inline image with a title
- an inline run of two images where only one has a title
- a standalone block image with a title
- an image whose title is empty (`""`) — parses as titled, normalized to `None`
- an image whose title is unclosed — parses as plain text

### 5. Render title as a tooltip and alt as load-failure fallback
Update the block-list renderers in `app/src/ai/blocklist/block/view_impl/common.rs` so that every successfully rendered image is wrapped with Warp's existing tooltip helper when the section's `title` is `Some(non_empty)`. Concretely:

- `render_visual_markdown_block` (current inline-image/Mermaid shared helper) gains an optional `tooltip: Option<String>` and an optional `tooltip_mouse_state: Option<MouseStateHandle>` in its `VisualMarkdownBlockOptions`. When `tooltip` is `Some`, wrap `content` with `appearance.ui_builder().tool_tip_on_element(...)` using the passed-in `tooltip_mouse_state`; if the caller does not supply one, fall back to `MouseStateHandle::default()` (used only by call sites that do not have stable per-image handles, such as the collapsible reasoning path).
- The `MouseStateHandle` used for the tooltip must persist across frames. `tool_tip_on_element` wraps the element in `Hoverable::new(mouse_state_handle, ...)` and the tooltip is rendered only when `state.is_hovered()` returns true; a fresh `MouseStateHandle::default()` per render resets the hover state every frame, which is why image tooltips never appear today. Allocate a stable handle per `AIAgentTextSection::Image` section on `AIBlockStateHandles` (new `image_section_tooltip_handles: Vec<MouseStateHandle>` field next to `normal_response_code_snippet_buttons` and `table_section_handles`), populated in the same per-section fold at `app/src/ai/blocklist/block.rs (~2291-2316)`, and thread that slice through `TextSectionsProps` alongside the existing code / table handle slices. The renderer walks the slice in source order using a new `starting_image_section_index` counter (mirroring `starting_code_section_index`), incremented by `image_group.images.len()` for grouped renders and by 1 for the single-image fallback path.
- Call sites that build `VisualMarkdownBlockOptions` — the inline image row builder, the block-image-group row builder, and the Mermaid card builder — pass through `title` from `AgentOutputImage` and, for image paths, the per-image handle fetched from the threaded slice. Mermaid has no CommonMark title, so `tooltip` is `None` for that call site and its behavior is unchanged; it may pass `tooltip_mouse_state: None`.
- The block-list load-failure fallback currently in `render_visual_markdown_fallback` (`common.rs (1888-1902)`) continues to render a `Text` element. Change the input from "always `markdown_source`" to "alt if non-empty, otherwise `markdown_source`" by having the section renderers compute the fallback text once and pass it in, rather than reading `markdown_source` deep inside the fallback helper. This keeps the helper dumb and moves the alt-vs-markdown decision to exactly one place per renderer.
- For inline image rows (`render_inline_image_section_group`-equivalent, `common.rs (~1700-1760)`), grouped block images (`render_block_image_section_group`, `common.rs (1771-1836)`), and individual fallback rendering, each failed image falls back independently using the same alt-vs-markdown decision. This matches product invariant 7.

Accessibility-label changes are intentionally out of scope for this issue. The alt text work here is limited to the visible load-failure fallback defined in the product spec; do not attempt to add new image accessibility APIs as part of this change.

### 6. HTML and copy serialization
Three serialization sites need to be made title-aware:

- `crates/editor/src/content/markdown.rs:787-796` emits `src` and `alt`. Add `title` when non-empty, wired through the new `BufferBlockItem::Image { title, .. }` field. The title value must continue to flow through the existing HTML serializer/attribute path so characters such as `\"`, `<`, and `>` are escaped rather than interpolated raw.
- `BufferBlockItem::Image::as_markdown` (`crates/editor/src/content/text.rs:457`) emits `![alt](source "title")` when the field is present; otherwise emits the existing `![alt](source)` form. Use `"` as the canonical re-serialized delimiter for the editor round-trip (matches the CommonMark canonical output and the HTML-export field).
- `markdown_source_for_image` in `app/src/ai/agent/util.rs:370-372` does the same for the block-list path.

Right-click copy on a rendered image already writes `AgentOutputImage::markdown_source` verbatim; once the section payload carries title-aware markdown serialization, right-click copy automatically preserves the alt/source/title content while using the same canonical double-quoted form as the rest of markdown serialization.

### 7. Feature gating
The title suffix is accepted by the shared `markdown_parser` unconditionally. The AI block-list tooltip and alt-fallback behavior continues to live behind `FeatureFlag::BlocklistMarkdownImages`; nothing in this change alters that gating. When the flag is disabled, image sections still fall back to `markdown_source`, now including the title suffix (matching product invariant 12).

No new feature flag is required.

## Testing and validation

Each numbered invariant in `specs/GH849/product.md` maps to at least one test or manual step below.

### Unit tests
- `crates/markdown_parser/src/markdown_parser_test.rs` — parser coverage for invariants 1, 2, 3, 4, 5, 13. Each of the three title delimiters, empty-title normalization, escaped closing delimiters, unclosed-title fallback, and line-ending-in-title fallback have their own test cases. A regression test for `![alt](source)` keeps the no-title default from drifting.
- `crates/editor/src/content/text_tests.rs` / `content/edit_tests.rs` — buffer-round-trip tests that confirm `![alt](source "title")` survives markdown → `BufferBlockItem::Image` → markdown serialization, covering invariants 8, 9.
- `crates/editor/src/content/markdown_tests.rs` (or the existing HTML-serialization test module) — HTML export includes `title="…"` when a title is present and omits it when the field is `None` or empty, covering invariant 9.
- `app/src/ai/agent/util_tests.rs` — block-list section extraction for titled block-level images, titled inline runs, empty-title normalization, and unclosed-title fallback, covering invariants 2, 3, 4, 5 at the AI-block layer.
- `app/src/ai/blocklist/block/view_impl/common_tests.rs` — section-renderer coverage:
  - a successful render carries the tooltip string it was given (invariant 6)
  - a successful render with `None`/empty title attaches no tooltip (invariant 6)
  - a failed render with non-empty alt falls back to alt (invariant 7)
  - a failed render with empty alt falls back to markdown source (invariant 7)
  - restored blocks follow the same decision as freshly parsed blocks (invariant 11)

### Integration tests
- Extend the existing restored-AI-block integration test (`crates/integration/src/test/agent_mode.rs::test_restored_ai_block_renders_mermaid_and_local_images` or a sibling) with a synthetic response that includes a titled inline image, a titled block image, and an untitled image. Assert through the existing screenshot/observation path that:
  - both titled images render their image element (no regression vs. today's untitled form)
  - the untitled image continues to render identically
  - a block image whose file is removed falls back to the alt string, not the raw markdown, when alt is non-empty

### Manual validation
- Author a dogfood AI response that includes `![A dog](docs/dog.png "Rex, my dog")` and confirm the image renders and hovering shows "Rex, my dog" in the Warp tooltip (invariant 6).
- Trigger all three delimiter forms (`"…"`, `'…'`, `(…)`) and verify the tooltip text matches the authored title (invariant 3).
- Trigger an empty title (`""`) and verify no tooltip appears (invariants 4, 6).
- Point the image at a missing file and verify the inline fallback shows the alt string when alt is non-empty and the raw markdown when alt is empty (invariant 7).
- Right-click copy a rendered titled image and paste into a plain-text editor; verify the clipboard content preserves the alt, source, and title text while normalizing the title delimiter to the double-quoted form (invariant 8).
- Export the block to HTML (via the existing export affordance) and verify the emitted `<img>` element has both `alt` and `title` attributes when the source had a title, only `alt` when it did not, and escapes special characters in the title attribute value (invariant 9).
- Restore the same AI conversation from history and confirm the tooltip and fallback behavior match the initial render (invariant 11).
- Disable `BlocklistMarkdownImages` locally and confirm titled images render as raw markdown, including the title suffix (invariant 12).
- Author prose like `something ("not a title")`, a link with a title (`[text](url "title")`), and a fenced code block containing `![alt](src "title")`; verify none of those trigger image rendering or tooltip behavior (invariant 13).

## Risks and mitigations

### Risk: changing image parsing without accidentally changing link parsing
`parse_link_target` is also called from `parse_link` in `crates/markdown_parser/src/markdown_parser.rs`. Extending that shared helper directly would silently add link-title parsing to the link path, which is explicitly out of scope (product non-goal, invariant 13).

Mitigation: keep `parse_link_target` unchanged and route only the image path through the new image-specific target parser. Add a link-regression test asserting that a link with a title renders as today (no title field on the link, link text unchanged).

### Risk: field-parity churn across editor and agent SDK
`FormattedImage` and `BufferBlockItem::Image` are destructured in many files across the editor crate and the agent SDK driver. Missing a call site produces a compile error rather than a behavior bug, but the blast radius is large.

Mitigation: add the field as a named struct field (not a tuple variant), rely on Rust's exhaustive-destructure checking, and include a short review checklist in the PR description listing every file in "Relevant code" so the reviewer can verify field parity at a glance.

### Risk: tooltip regression for the no-title default
The renderer currently has no tooltip wrapper. If the new wrapper attaches even when `tooltip` is `None`, hovering untitled images could show an empty tooltip or steal focus.

Mitigation: branch at the caller — only wrap in the tooltip helper when `tooltip: Some(non_empty_string)`. Cover this with the \"successful render with `None`/empty title attaches no tooltip\" unit test above.

## Follow-ups
- Link titles (`[text](url \"title\")`) are out of scope here; the image-specific target parser keeps that as a localized follow-up if product later wants it.
- Reference-style images (`![alt][ref]`) remain unsupported, as today.
- Surfacing title and alt inside the shared lightbox overlay can be a later enhancement if product wants it.
