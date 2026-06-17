# Inline Markdown Images in AI Block List — Tech Spec
Product spec: `specs/zachlloyd/inline-markdown-images-in-blocklist/PRODUCT.md`

## Problem
The AI block list does not currently support the product behavior described in `PRODUCT.md`: rendering supported Markdown images and Mermaid diagrams inline inside AI responses with the approved inline-row, grouped-image, and Mermaid-card treatments.

Warp already has the underlying primitives needed to build this behavior:

- local image asset loading
- responsive image sizing
- Mermaid-to-SVG rendering
- block-list rendering of other markdown section types like code and tables

Today, block-list markdown is split into plain text, code, and table sections. Plain text is rendered with `FormattedTextElement`, which intentionally treats `FormattedTextLine::Image` as a line break rather than a visual block. As a result, Markdown image syntax degrades to raw text in the AI block list, and Mermaid fences continue to render through the existing code-path behavior rather than the product's visual treatment.

The implementation should add support for the required file-backed image and Mermaid behaviors while keeping the change narrowly scoped:

- no new cross-renderer selection model
- no broad markdown-renderer refactor
- no new dependency edge from the block list into a rendering stack it does not already use

This should be a surgical extension of the existing section-based block-list renderer, with explicit copy behavior for rendered visuals, a visible file-link label below successfully rendered file-backed images, raw-Markdown fallback when rendering is unavailable, and reuse of Warp’s existing fullscreen lightbox treatment when the user clicks a rendered visual.

## Relevant Code
- `specs/zachlloyd/inline-markdown-images-in-blocklist/PRODUCT.md` — approved product behavior
- `app/src/ai/agent/util.rs (24-186)` — `parse_markdown_into_text_and_code_sections`; current block-list markdown section splitter
- `app/src/ai/agent/mod.rs (1281-1361)` — `AgentOutputText`, `AgentOutputTable`, and `AIAgentTextSection`
- `app/src/ai/agent/mod.rs (405-505)` — `AIAgentOutput::all_text`; current message traversal used by rendering and link detection
- `app/src/ai/agent/mod.rs (1547-1628)` — `Display for AIAgentOutputMessage`; block/conversation copy formatting for markdown sections
- `app/src/ai/agent/mod.rs (2722-2779)` — `AIAgentExchange::format_output_for_copy` and `format_for_copy`
- `app/src/ai/blocklist/block/view_impl/output.rs (246-345)` — message-level block-list output rendering loop
- `app/src/ai/blocklist/block/view_impl/common.rs (869-952)` — `render_text_sections`; current section-to-UI mapping
- `app/src/ai/blocklist/block/view_impl/common.rs (986-1084)` — `render_rich_text_output_text_section`; current plain-text markdown renderer
- `app/src/ai/blocklist/block/view_impl/common.rs (1461-1646)` — `render_code_output_section`; current code-block renderer
- `app/src/workspace/lightbox_view.rs` — shared fullscreen lightbox view with Escape and left/right navigation
- `crates/ui_components/src/lightbox.rs` — reusable lightbox component and navigation controls
- `app/src/workspace/action.rs` — workspace actions for opening and updating the shared lightbox
- `app/src/workspace/view.rs` — workspace-level lightbox lifecycle and focus handling
- `app/src/ai/blocklist/block.rs (2230-2246)` — per-section handle allocation for code and tables
- `app/src/ai/blocklist/block.rs (4313-4476)` — `selected_text`, `clear_all_selections`, and `clear_other_selections`; current selection model
- `app/src/ai/blocklist/block.rs (4946-5002)` — block-level copy scope
- `app/src/ai/blocklist/controller.rs (62-86)` — `SessionContext`; current working-directory capture
- `app/src/ai/blocklist/persistence.rs (24-31)` — persisted exchange `working_directory`
- `app/src/ai/blocklist/history_model.rs (2102-2154)` — restored query history carries working directory
- `app/src/ai/blocklist/block.rs (906-919)` — `AIBlock::new` accepts `current_working_directory` and `shell_launch_data`
- `crates/markdown_parser/src/markdown_parser.rs (286-326)` — `parse_image`; current image recognition behavior
- `crates/markdown_parser/src/markdown_parser.rs (466-475)` — `parse_inline_markdown`; current inline parser surface
- `crates/markdown_parser/src/markdown_parser_test.rs (2171-2320)` — current parser coverage for images
- `crates/warpui_core/src/elements/formatted_text_element.rs (1580-1591, 1661-1671)` — images are currently treated as line breaks in the block-list rich-text path
- `crates/editor/src/content/text.rs (280-362)` — image markdown round-trip behavior in the editor stack
- `crates/editor/src/content/text.rs (526-579)` — `CodeBlockType::Mermaid` gating
- `crates/editor/src/content/edit.rs (56-91)` — native/WASM asset-source resolution rules
- `crates/editor/src/content/mermaid_diagram.rs (20-67)` — in-memory Mermaid SVG asset generation and sizing
- `crates/warpui_core/src/image_cache.rs` — supported shared image types (JPEG, PNG, GIF, WebP, SVG)
- `crates/warp_features/src/lib.rs (525-540, 817-858)` — markdown-related feature flags
- `app/src/lib.rs (2462-2471)` — app-side feature-flag wiring for markdown tables and Mermaid
- `app/Cargo.toml (667-746)` — compile-time feature declarations
- `app/src/ai/agent/util_tests.rs` — block-list markdown parser/unit test pattern
- `crates/editor/src/content/edit_tests.rs (293-404)` — Mermaid layout test pattern
- `crates/editor/src/content/text_tests.rs (101-127)` — Mermaid block-type gating tests
- `crates/integration/tests/INTEGRATION_TESTING.md` — integration test registration flow

## Current State

### Markdown parsing in the AI block list
The block list currently parses AI markdown with a line-oriented splitter in `app/src/ai/agent/util.rs (24-186)`. It recognizes:

- plain markdown text
- fenced code blocks, including linked-code metadata
- GFM tables

It does not emit explicit image or Mermaid section types today. Standard Markdown image syntax therefore remains embedded inside `AIAgentTextSection::PlainText`, and Mermaid code fences remain ordinary `Code` sections.

### Current Markdown semantics vs current parser behavior
Standard Markdown/CommonMark uses `![alt](source)` as image syntax. There is not a separate standard syntax that means “this should be shown as a literal file path instead of rendered as an image.” If the author wants literal text, they escape the Markdown or place it in code spans/blocks.

Our current `markdown_parser` does not fully implement that inline image semantic yet. `crates/markdown_parser/src/markdown_parser.rs (286-326)` only recognizes images when they occupy a standalone line; inline paragraph images like `text ![img](foo.png) more text` are currently treated as plain text by this parser. For this redesign, the block list still should not invent a generic “render some arbitrary inline image-looking text” rule. The only extension should be a narrow line parser for runs of parser-compatible image references separated only by whitespace so we can distinguish:

- consecutive standalone image lines, which stay block-level
- multiple image references on the same line, which become an inline image row

### Why images disappear today
Plain-text markdown sections are rendered with `render_rich_text_output_text_section` in `app/src/ai/blocklist/block/view_impl/common.rs (986-1084)`, which delegates to `FormattedTextElement`. In `crates/warpui_core/src/elements/formatted_text_element.rs (1580-1591, 1661-1671)`, `FormattedTextLine::Image(_)`, `FormattedTextLine::Embedded(_)`, and `FormattedTextLine::HorizontalRule` are all treated as line-break-like layout items rather than renderable content. That is the immediate reason that block-list Markdown images never show up.

### Asset and Mermaid support already exists elsewhere
Warp already has the low-level capabilities this feature needs:

- shared image format support in `crates/warpui_core/src/image_cache.rs`
- asset-source resolution, including WASM-safe behavior, in `crates/editor/src/content/edit.rs (56-91)`
- Mermaid SVG generation and sizing in `crates/editor/src/content/mermaid_diagram.rs (20-67)`
- Mermaid code-block identification in `crates/editor/src/content/text.rs (526-579)`

The app crate already depends on `warp_editor`, and the block list already embeds editor-backed code blocks. Reusing editor Mermaid/image helpers therefore does not introduce a new crate dependency edge.

Warp also already has a reusable fullscreen lightbox path at the workspace layer. `WorkspaceAction::OpenLightbox` / `UpdateLightboxImage` drive `LightboxView`, which already supports Escape dismissal, left/right keyboard navigation, and previous/next buttons. The block-list visual renderer should reuse that path rather than inventing a new fullscreen viewer.

### Working-directory metadata already exists
The product requirement for resolving relative paths against the working directory captured when the AI block rendered is already compatible with existing data flow:

- session context captures `current_working_directory` in `app/src/ai/blocklist/controller.rs (62-86)`
- persisted exchanges store `working_directory` in `app/src/ai/blocklist/persistence.rs (24-31)`
- restored history loads it back in `app/src/ai/blocklist/history_model.rs (2102-2154)`
- `AIBlock::new` already receives both `current_working_directory` and `shell_launch_data`

We do not need a new persistence field for this feature; we need to thread existing metadata into the visual-section renderers.

### Selection is intentionally out of scope
The current selection model in `app/src/ai/blocklist/block.rs (4313-4476)` is fragmented across:

- block-level text selection
- child-view-native selection for code editors and other special renderers

That architecture does not support a continuous mixed-content selection model across text, code, tables, images, and Mermaid. This feature should not attempt to fix that. Instead, it should:

- preserve existing text/code/table selection behavior
- add explicit copy affordances for rendered images and Mermaid diagrams
- ensure block-level copy/export includes the correct raw Markdown for visual sections

## Proposed Changes

### 1. Add a dedicated block-list image feature flag
Add a new feature flag dedicated to AI block-list markdown visuals, e.g. `BlocklistMarkdownImages`.

Implementation points:
- add `BlocklistMarkdownImages` to `crates/warp_features/src/lib.rs`
- add `blocklist_markdown_images = []` to `app/Cargo.toml`
- wire it in `app/src/lib.rs` alongside the existing markdown table/Mermaid flags
- enable it in `DOGFOOD_FLAGS`, but leave it out of `PREVIEW_FLAGS` until the surface is stable

Mermaid rendering in the block list should require:
- `FeatureFlag::BlocklistMarkdownImages`
- `FeatureFlag::MarkdownMermaid`

File-backed images in the block list should require only `BlocklistMarkdownImages`.

I do not recommend reusing the dormant generic `MarkdownImages` flag for this launch. It is not currently wired through `app/src/lib.rs` or `app/Cargo.toml`, and this feature needs rollout control specific to the AI block list.

### 2. Extend the AI markdown section model with visual sections
Extend `AIAgentTextSection` in `app/src/ai/agent/mod.rs` with explicit visual section variants:

- `Image { image: AgentOutputImage }`
- `MermaidDiagram { diagram: AgentOutputMermaidDiagram }`

Proposed payloads:
- `AgentOutputImage`
  - `alt_text: String`
  - `source: String`
  - `markdown_source: String`
  - `layout: AgentOutputImageLayout`
- `AgentOutputMermaidDiagram`
  - `source: String`
  - `markdown_source: String`

The key design choice is to preserve Markdown source on the section payload rather than reconstructing it later from rendered state. That keeps:

- block-level copy/export exact
- right-click copy trivial
- fallback rendering simple

For images, `markdown_source` can initially be canonicalized to `![alt](source)` because the parser only returns `alt_text` and `source`, not byte-accurate source spans. That is consistent with the editor stack’s current image markdown round-trip behavior.

### 3. Teach the section splitter to extract images and Mermaid explicitly
Update `parse_markdown_into_text_and_code_sections` in `app/src/ai/agent/util.rs` so that it can distinguish inline image runs from block-level image lines instead of treating every extracted image section identically.

The change should remain incremental:
- keep current code-fence boundary detection so linked-code metadata parsing stays unchanged
- keep current table extraction behavior
- for plain-text regions, continue scanning line-by-line
- keep using `markdown_parser` for standalone image-line extraction
- add a narrow helper that recognizes lines containing two or more parser-compatible image references separated only by whitespace and emits them as `AIAgentTextSection::Image` sections tagged with `AgentOutputImageLayout::Inline`
- emit standalone image lines as `AIAgentTextSection::Image` tagged with `AgentOutputImageLayout::Block`
- when a fenced code block language resolves to Mermaid, emit `AIAgentTextSection::MermaidDiagram` instead of `Code`
- leave everything else in `PlainText`

Two important constraints:
- image extraction should still stay narrow and markdown-driven rather than becoming a generic loose regex over arbitrary prose
- Mermaid section extraction should not be gated at parse time; the section payload should still preserve Mermaid source even when the runtime flags are off, so the renderer can fall back cleanly to raw Markdown without reparsing restored conversations

### 4. Keep the existing section renderer and add visual sections surgically
Do not replace the current block-list markdown rendering architecture for this feature. Instead, extend `render_text_sections` in `app/src/ai/blocklist/block/view_impl/common.rs (869-952)` with grouped visual renderers:

- inline image-row rendering for consecutive `AgentOutputImageLayout::Inline` sections
- grouped block-image row rendering for consecutive `AgentOutputImageLayout::Block` sections
- a Mermaid card renderer that uses the new framed treatment

These should follow the same overall section-rendering pattern already used for:
- code sections
- table sections

This keeps the implementation targeted:
- no unified markdown content view
- no mixed-content selection refactor
- no change to `AIBlock::selected_text` semantics beyond whatever is needed for explicit image/Mermaid copy affordances

As part of that renderer pass, build a source-ordered collection of the successfully renderable visual sections in the current AI message. That shared collection should include both successfully rendered file-backed images and successfully rendered Mermaid diagrams. Clicking any rendered image or Mermaid diagram should dispatch `WorkspaceAction::OpenLightbox` with that shared collection and the clicked section’s initial index, so previous/next navigation works across the other renderable visuals from the same message.

### 5. Reuse existing editor helpers without a new dependency edge
The new visual-section renderers should reuse existing editor helpers rather than inventing a separate rendering stack.

For file-backed images:
- extract or adapt the existing asset-resolution helper in `crates/editor/src/content/edit.rs (56-91)` so the block list can resolve relative paths against a base directory rather than a document path
- pass the AI block’s stored working directory as that base directory
- on native platforms, canonicalize when possible
- on WASM, preserve the existing no-canonicalization behavior

For Mermaid:
- use `mermaid_asset_source` from `crates/editor/src/content/mermaid_diagram.rs (20-67)`
- derive the rendered max width from the loaded asset's intrinsic size rather than calling a separate Mermaid layout helper from the block-list renderer
- honor `FeatureFlag::MarkdownMermaid` in addition to the new block-list image flag

For actual rendering:
- use a block-list-local renderer in the existing section flow
- use WarpUI image elements and existing block-list spacing/styling conventions
- preserve the existing right-click-to-copy Markdown behavior on every rendered visual
- for inline image runs, render medium-height image tiles with per-image labels below
- for grouped block images, render stacked thumbnail-plus-path rows using the full source path text
- for Mermaid, render a bordered card shell with a titled header and a white inner canvas that contains the diagram
- on left click, open the shared workspace lightbox instead of a block-list-specific fullscreen modal for both rendered file-backed images and rendered Mermaid diagrams
- keep the path labels on the shared link-detection pipeline so hover/click behavior remains unchanged
- allow the implementation to harden shared image/text primitives where needed for this surface, including guarding non-finite image rects, exposing intrinsic SVG dimensions through shared image metadata, and constraining single-line text hit testing to valid Y bounds

This follows the same broad pattern as the block list’s existing special markdown sections: section-specific render helpers living inside the current block-list message renderer, with reusable lower-level helpers coming from crates the app already depends on.

### 6. Render fallback content instead of broken visuals
The renderer should decide per visual section whether it can render as a visual block or must fall back to raw Markdown.

For file-backed images:
- if `BlocklistMarkdownImages` is disabled, render `markdown_source`
- if the path cannot be resolved, render `markdown_source`
- if the asset fails to load or is an unsupported image type, render `markdown_source`
- only include images in an inline row or grouped block row set when that individual image succeeds; unsupported or missing images fall back to raw Markdown in source order

For Mermaid:
- if either `BlocklistMarkdownImages` or `MarkdownMermaid` is disabled, render `markdown_source`
- if Mermaid SVG generation fails, render `markdown_source`

This keeps fallback behavior local to rendering and avoids mutating stored section payloads based on transient failures.

### 7. Make block-level copy and export section-aware
Update the markdown copy/export paths in `app/src/ai/agent/mod.rs (1547-1628, 2722-2779)` so that `Image` and `MermaidDiagram` sections serialize from their preserved Markdown source.

Rules:
- block-level output copy uses `markdown_source` for image and Mermaid sections
- conversation export continues to be markdown, not image bytes or HTML
- right-click copy on a rendered image or Mermaid diagram writes that section’s `markdown_source`

This gives us correct image/Mermaid copy behavior without changing the existing selection architecture.

### 8. Render the file-link label with local link detection
The new file-link label under rendered file-backed images should reuse the existing text/link rendering helpers rather than introducing a special-case link widget.

Implementation shape:
- keep the current global output link-detection pipeline unchanged for the underlying markdown section data
- in the successful image-render path, render the link text in the layout-specific position required by the product behavior
- keep using the shared `DetectedLinksState` keyed by the section index so the path labels get normal file-link highlighting and click behavior without a second link-detection system
- keep binary-file link opening on the standard file-open path so those labels resolve to the platform default app rather than `$EDITOR`
- keep find/highlight behavior scoped to the existing global text pipeline rather than refactoring search indexing for this label in the same change

This keeps the change narrowly targeted while still giving users a visible, interactive file link under rendered file-backed images.

### 9. Preserve existing selection behavior
Do not add a new drag-selection model for rendered visuals in this change.

The expected behavior for this implementation is:
- existing text/code/table selection behavior remains unchanged
- rendered images and Mermaid diagrams expose explicit copy behavior through right-click/context-menu affordances
- block-level copy/export includes the right raw Markdown for visual sections

That is the intended “minimal change” scope for this feature.

### 10. Keep restored conversations deterministic
Restored conversations should not depend on reparsing with different metadata than the original render.

The renderer should use:
- the section payload stored on the conversation output
- the `working_directory` already persisted with the exchange/block
- current runtime flags to decide whether to render a visual or raw fallback

That means restored blocks will:
- resolve relative file images against the original captured working directory
- render Mermaid only when both flags are currently enabled
- still show raw Markdown if the file is gone or the platform cannot load it

## End-to-End Flow
1. The AI backend streams markdown text into an `AIAgentOutputMessageType::Text`.
2. `parse_markdown_into_text_and_code_sections` in `app/src/ai/agent/util.rs` identifies plain text, code, tables, parser-recognized images, and Mermaid sections.
3. The parsed output stores explicit `Image` and `MermaidDiagram` sections alongside their raw Markdown source.
4. `app/src/ai/blocklist/block/view_impl/output.rs` renders the new visual sections through the existing section-based block-list renderer alongside plain text, code, and tables.
5. The visual-section renderers receive the AI block’s stored `current_working_directory` and `shell_launch_data`.
6. File-backed image sections resolve their source path relative to that working directory.
7. Mermaid sections build an in-memory SVG asset source with `mermaid_asset_source`.
8. The block-list renderer derives a source-ordered lightbox collection from the renderable visual sections in that message.
9. Each visual section either renders as an image block or falls back to a plain markdown block depending on feature flags and asset/render success.
10. When a file-backed image renders successfully, the renderer also shows a clickable/highlighted file link beneath or beside it:
   - inline image rows show the basename / final path segment
   - block image rows show the full source path
11. Clicking a rendered image or Mermaid diagram dispatches `WorkspaceAction::OpenLightbox` with that shared visual collection and the clicked section’s initial index.
12. Right-click copy on a rendered image or Mermaid diagram writes the section’s `markdown_source`.
13. Clicking a rendered file link continues to use the shared file-open flow, which means binary file labels open in the platform default app instead of `$EDITOR`.
14. Block-level copy and conversation export continue to emit markdown, using `markdown_source` for visual sections.
15. When the conversation is restored later, the same section payload renders again using the stored working-directory metadata and current flags.

## Risks and Mitigations

### Risk: selection expectations drift beyond the implementation
The current AI block architecture does not support unified mixed-content selection across renderer types. If the specs or implementation drift back toward that requirement, this feature would expand substantially.

Mitigation:
- keep the product and tech specs explicit that this change does not add a new cross-renderer selection model
- keep the scope focused on rendering, explicit copy affordances, and block-level copy/export correctness

### Risk: parser scope does not match full Markdown image semantics
Standard Markdown treats images as inline syntax, but our current parser only recognizes standalone-image lines, and this redesign needs a narrow same-line image-row case.

Mitigation:
- keep the new same-line parser limited to lines that consist entirely of parser-compatible image references separated only by whitespace
- do not treat arbitrary prose containing `![...]` as inline-renderable image content
- treat fuller CommonMark-style inline image support as follow-up parser work

### Risk: relative path resolution diverges between notebooks and AI blocks
The existing editor helper resolves relative image paths against a document path, while AI blocks need to resolve against the captured session working directory.

Mitigation:
- factor path resolution into a base-directory-oriented helper
- update notebook and AI-block callers to use it explicitly
- add native and WASM coverage

### Risk: regressions to existing section rendering
Adding two more section types to the current renderer could affect section indexing, link-detection offsets, or restored-block bookkeeping.

Mitigation:
- keep section ordering explicit in `AIAgentOutput::all_text` and render traversal
- follow existing per-section bookkeeping patterns for code/table sections
- add mixed-content coverage that includes ordinary code blocks and tables, not just images

## Testing and Validation

### Unit tests
- extend `app/src/ai/agent/util_tests.rs` with:
  - standalone image extraction
  - same-line inline image-run extraction
  - standalone multi-line block-image extraction
  - image + table + text ordering
  - Mermaid section extraction
- add block-list markdown copy tests covering:
  - image sections serialize to `![alt](source)`
  - Mermaid sections serialize to fenced mermaid markdown
- add native/WASM unit tests around relative path resolution against a base directory
- add focused rendering/helper coverage for:
  - section-index preservation across skipped empty text sections
  - lightbox index mapping in source order
  - inline-image basename labeling
  - visual width guard rails for invalid sizes
- add shared primitive regression tests for:
  - invalid image rect rejection
  - SVG intrinsic image sizing
  - single-line text hit testing respecting Y bounds

### Integration tests
- A reasonable first integration test is a single manual-observation test, `test_restored_ai_block_renders_mermaid_and_local_images`, in `crates/integration/src/test/agent_mode.rs`.
- That test would restore a synthetic `ConversationData` through `load_conversation_from_tasks`, set `InputContext.directory` to `crates/warpui_core/test_data`, and capture a real-display screenshot of one AI response that contains same-line local image markdown plus a Mermaid fence.
- It would be intentionally narrow: it would prove the restored AI block list can render both surfaces through the real conversation-hydration path without introducing protobuf fixtures or test-only UI hooks.
- Recommended follow-up integration coverage remains:
  - an AI response containing plain text, a linked code block, a markdown table, a relative-path image, and a Mermaid diagram
  - block-level copy serialization for mixed content
  - restored-conversation rendering and fallback behavior
  - disabled-flag fallback to raw Markdown
  - shared-lightbox initial selection and previous/next navigation in message source order

### Manual validation
- relative and absolute local image paths for PNG, JPEG, GIF, WebP, and SVG
- inline same-line image rows with path labels below each image
- grouped standalone block-image rows with thumbnail-plus-path content
- missing-file fallback
- unsupported-format fallback
- Mermaid render success and Mermaid render failure fallback in the new framed treatment
- click-to-expand from both rendered images and Mermaid diagrams into the shared lightbox
- previous/next keyboard and button navigation inside the lightbox
- right-click copy on both file-backed images and Mermaid diagrams
- block-level copy on a response containing text, code, table, image, and Mermaid content
- restored conversation rendering after reopening history
- WASM sanity pass to verify path handling and fallback behavior do not panic

## Follow-ups
- full CommonMark-style inline image support if we decide to expand `markdown_parser` beyond its current standalone-image behavior
- unified mixed-content selection across block-list renderer types
- consolidating `MarkdownImages` and `BlocklistMarkdownImages` if Warp later ships a broader app-wide markdown-image rollout
- richer image interactions such as open/save/zoom, if product wants them later
