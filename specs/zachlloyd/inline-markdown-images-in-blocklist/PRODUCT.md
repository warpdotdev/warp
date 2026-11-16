# Inline Markdown Images in AI Block List — Product Spec
Linear: none provided
Figma: none provided

## Summary
Render supported Markdown images inline inside AI block list responses instead of always showing raw Markdown image syntax. The first release should support local file-backed images referenced by relative or absolute filesystem paths, plus Mermaid diagrams rendered on the fly in the same response flow.

This feature should make AI responses easier to read when they include screenshots, diagrams, or visual outputs, while preserving reliable copy behavior without requiring a broader block-list selection refactor in the first release. The presentation should now follow three distinct treatments from the approved mock:

- horizontal inline image rows for multiple image references that appear in the same Markdown line
- a stacked “Images N” card for consecutive block-level image references
- a framed Mermaid block with a titled header and white diagram canvas

Successfully rendered file-backed images and Mermaid diagrams should both reuse Warp’s existing fullscreen lightbox treatment for click-to-expand viewing.

When an image cannot be resolved or the format is unsupported, the block list should continue to show the original Markdown text rather than rendering a broken or partial image UI.

## Problem
AI responses increasingly include Markdown image references and Mermaid diagrams. In the AI block list today, that content is more useful as source text than as rendered output, but it is much harder to read and scan when the user actually wants to view the visual result inline.

Warp already supports Markdown image blocks elsewhere in the app and already has Mermaid-to-SVG rendering for notebook Markdown. The AI block list currently lacks the equivalent presentation and interaction model. That creates three problems:

- visual content in AI responses is harder to understand because users see raw Markdown instead of the image or diagram
- rendered output is inconsistent with other Markdown-capable surfaces in Warp
- copy and selection behavior for mixed text-and-image content is undefined unless we specify it explicitly

## Goals
- Render supported local Markdown images inline inside AI block list responses.
- Support both absolute file paths and relative file paths.
- Resolve relative file paths relative to the current working directory of the session associated with the AI block.
- Support Mermaid diagrams in the same overall rollout, rendering them inline from their Markdown source.
- Ensure the feature works across Warp’s supported platforms, including WASM.
- Use the same supported image format set that Warp’s existing Markdown/image rendering stack already supports today: JPEG/JPG, PNG, GIF, WebP, and SVG.
- Show the referenced file link below each successfully rendered file-backed image in the AI block list.
- Preserve stable copy behavior for rendered images and Mermaid diagrams without introducing a new cross-renderer selection model in this first release.
- Allow right-click copy on a rendered image or Mermaid diagram, copying the underlying Markdown source rather than image bytes.
- Fall back cleanly to the raw Markdown source when the image cannot be found, cannot be resolved, or is not a supported format.
- Gate block list image rendering behind a dedicated block-list-specific feature flag.
- Respect the existing Mermaid rendering flag in addition to the new block list image flag, so Mermaid diagrams render in the AI block list only when both flags are enabled.
- Reuse Warp’s existing fullscreen lightbox treatment for both rendered file-backed images and rendered Mermaid diagrams.

## Non-Goals
- Supporting remote image URLs in this first release.
- Supporting Markdown data URLs or other inline-encoded image sources in this first release.
- Adding image editing, resizing controls, zoom controls, or image-specific toolbars.
- Copying image bytes to the clipboard.
- Adding drag-and-drop, save-image, or open-image actions.
- Supporting additional image formats beyond the set Warp already renders today.
- Shipping file-backed images first and Mermaid later; both are part of the same implementation sequence.

## User Experience

### Scope
This feature applies to AI block list responses that render Markdown content.

It should work for:
- newly streamed AI responses
- previously rendered responses reopened from history
- restored conversation views that display the same AI block content
- all supported client platforms, including WASM

### Supported sources
The first release should support two kinds of inline visual Markdown content:

1. Standard Markdown image references that point to local files.
2. Mermaid diagrams authored in Markdown that Warp renders into an image representation at runtime.

For file-backed images, the initial supported path types are:
- absolute filesystem paths
- relative filesystem paths

Relative paths resolve against the current working directory of the session associated with the AI block, not against a notebook file path or any repository root heuristic.
That working directory should be the one recorded with the AI block when the response was rendered. Relative file paths should be re-resolved against that recorded working directory when the block is later restored or re-rendered.

### Supported formats
For file-backed images, the AI block list should support the same image formats Warp’s existing shared image renderer already supports today:
- JPEG / JPG
- PNG
- GIF
- WebP
- SVG

If the referenced file exists but is not one of those supported formats, Warp should not attempt a degraded or best-effort render. It should show the original Markdown text exactly as it does today.

### File-backed image rendering
When AI output contains a valid Markdown image whose source resolves to a supported local file, the image should render inline in the normal block list flow where the Markdown image appeared.

Rendered images should behave like block content in the response:
- they appear between the surrounding text in source order
- multiple images in one response all render in place
- images can appear alongside normal paragraphs, lists, code blocks, and tables without changing the rendering behavior of those surrounding elements

There are two distinct file-backed image layouts:

1. Inline image runs
When multiple parser-recognized Markdown image references appear on the same line, separated only by whitespace, Warp should render them as a horizontal image row:
- each image appears above its own visible source-path label
- each item preserves aspect ratio
- the row stays left-aligned in the response flow
- the path label under each image keeps the existing hover highlight and click-to-open behavior for file links
- if that path points to a binary file such as PNG, JPG, GIF, or WebP, activating the path should use the platform-default opener instead of Warp's editor or the user's configured `$EDITOR`

2. Block-level image groups
When Markdown images appear as standalone block items on separate lines, consecutive successfully rendered images should be grouped into a bordered card:
- the card header should read `Images N`, where `N` is the number of rendered images in that grouped block
- each row in the card should show a square thumbnail on the left and the source path on the right
- the source path keeps the same hover highlight and click-to-open behavior as other file links in AI output
- binary file paths in that row should open with the platform-default application rather than any editor target
- unsupported or unresolved images should fall back to raw Markdown rather than appearing inside the grouped card

For file-backed images that render successfully, Warp should also show the referenced file link/path in the layout appropriate for that treatment:
- the displayed link text should use the file path/source text from the Markdown image reference
- it should participate in the normal link-highlighting and click behavior users already expect for file links in AI block content
- binary file links should open with the operating system's default file opener; on macOS this means using `open` rather than routing through Warp's editor or `$EDITOR`
- it should remain visible in restored conversations the same way it does for newly rendered output

If a Markdown image reference cannot be resolved to an existing local file, the block list should show the original Markdown image text instead of a rendered image.

Rendered file-backed images should also support click-to-expand behavior:
- clicking a rendered image opens it in Warp’s existing fullscreen lightbox treatment
- the lightbox should reuse the same fullscreen overlay treatment already used elsewhere in Warp rather than introducing a new one-off viewer
- when the current AI message contains multiple successfully rendered visual items, the lightbox should allow previous/next navigation across those visuals in source order
- for file-backed images, the fullscreen viewer should keep showing the image source/path as descriptive text

### Mermaid rendering
Mermaid diagrams should be supported as part of the same feature and should render inline in the AI block list as visual content rather than raw Mermaid source.

Mermaid rendering in the AI block list requires two independent conditions to be true:
- the existing global Mermaid rendering flag is enabled
- the new AI block list image-rendering flag is enabled

If either flag is disabled, Mermaid content in the AI block list should continue to fall back to raw Markdown/source rendering.

From the user’s perspective:
- a Mermaid diagram in AI Markdown should appear as a rendered diagram in a titled, bordered block within the response flow
- the block should use a dark header treatment with a Mermaid-specific title and a white inner canvas for the rendered diagram
- rendering may happen asynchronously
- the UI must not block while the diagram is being generated
- clicking a rendered Mermaid diagram should open that diagram in the same fullscreen lightbox treatment used for rendered block-list images
- when there are multiple rendered visuals in the same AI message, the lightbox should support previous/next navigation between them in source order

If Mermaid rendering fails for any reason, the block list should fall back to showing the original Mermaid Markdown source rather than a broken image state.

### Layout and sizing
Rendered file-backed images and rendered Mermaid diagrams should preserve aspect ratio and remain fully visible without cropping or distortion, but the exact sizing now depends on the treatment:

- inline image rows use medium-height thumbnails with per-image widths derived from aspect ratio and capped so the row remains visually balanced
- block-level image groups use square thumbnail slots for each image row
- Mermaid uses a framed block with a padded white canvas that contains the rendered diagram

The inline/block treatments remain the default in-flow presentation. Clicking a rendered image or Mermaid diagram should open the shared fullscreen lightbox overlay as a secondary viewing mode, without changing the inline layout itself.

### Fallback behavior
Fallback behavior is important and should be predictable.

For file-backed images, if any of the following are true:
- the path cannot be resolved
- the file does not exist
- the format is unsupported
- the asset cannot be loaded successfully

then Warp should show the original Markdown image syntax inline, matching current behavior, instead of rendering an error-specific image container.

For Mermaid diagrams, if rendering fails, Warp should show the original Mermaid Markdown source instead of a rendered diagram.

### Selection and copy behavior
This first release should keep the existing block-list selection behavior for text, code blocks, tables, and other already-supported content. It should not introduce a broader refactor to make rendered images and Mermaid diagrams participate in mixed drag-selection across multiple renderer types.

Instead, rendered visual content should support copy through explicit copy surfaces:
- block-level copy actions should include the underlying Markdown source for rendered images and Mermaid diagrams
- right-click copy on a rendered image or Mermaid diagram should copy the underlying Markdown source for that visual element

For this first release, it is acceptable for copy to serialize visual content back into raw Markdown rather than attempting rich HTML or image clipboard output.

### Right-click copy behavior
Right-clicking a rendered image or Mermaid diagram should provide a copy action.

That copy action should place the underlying Markdown source on the clipboard:
- for file-backed images, the original Markdown image reference
- for Mermaid diagrams, the original Mermaid Markdown source

This copy action should not place image bytes on the clipboard in this release.

### Mixed content behavior
The AI block list must support responses that contain:
- only text
- only one image
- only one Mermaid diagram
- multiple file-backed images
- multiple Mermaid diagrams
- mixed text, images, and Mermaid diagrams in one response

Each supported visual element should render independently in the correct source position. Unsupported or unresolved items should remain raw Markdown text without preventing supported neighbors from rendering.

### Streaming and restored behavior
The feature should behave consistently for both streamed and restored responses.

For streamed responses:
- file-backed images should render once enough Markdown is present to identify the image reference
- Mermaid diagrams should render once enough source is present to identify and generate the diagram
- rendering updates should not corrupt surrounding content or unexpectedly clear an active selection

For restored responses:
- supported images and Mermaid diagrams should render the same way they do in the initial response view
- the same fallback rules should apply if the asset is unavailable at restore time

## Success Criteria
- A Markdown image in an AI block list response renders inline when it references a supported local file.
- Relative file paths resolve against the session’s working directory rather than a notebook/document location.
- Relative file paths are re-resolved against the working directory recorded with the AI block at render time when the block is restored or re-rendered.
- Absolute file paths render correctly when they point to a supported local image.
- The initial supported file format set is JPEG/JPG, PNG, GIF, WebP, and SVG.
- Successfully rendered file-backed images show their source path in the mock-aligned position for their layout, with normal link highlighting behavior.
- Activating the source path for a rendered binary asset opens it with the operating system's default app instead of Warp's editor or the user's configured `$EDITOR`.
- Mermaid diagrams in AI Markdown render inline in the AI block list only when both the existing Mermaid flag and the new block list image-rendering flag are enabled.
- Multiple image references on the same line render as a horizontal inline image row with one path label per image.
- Consecutive standalone image references render as a grouped `Images N` card with thumbnail-plus-path rows.
- Rendered images and Mermaid diagrams preserve aspect ratio and use the new mock-aligned sizing treatments.
- If a file cannot be found or the file format is unsupported, Warp shows the original Markdown text rather than a broken rendered state.
- If Mermaid rendering fails, Warp shows the original Mermaid Markdown source.
- Existing block-list text/code/table selection behavior continues to work as it does today after image rendering is added.
- Block-level copy actions write the underlying raw Markdown for rendered images and Mermaid diagrams into the clipboard.
- Right-click copy on a rendered image or Mermaid diagram copies its underlying Markdown source.
- Clicking a rendered file-backed image or Mermaid diagram opens the shared fullscreen lightbox treatment.
- The fullscreen lightbox supports previous/next navigation across the renderable visual items in the same AI message, in source order.
- Multiple supported images in one AI response render independently and do not interfere with surrounding text.
- Restored AI blocks and newly streamed AI blocks follow the same rendering and fallback rules.
- The implementation behaves correctly on all supported platforms, including WASM.
- The feature has automated verification coverage in addition to manual validation.

## Validation
- Unit tests for markdown/image section parsing and flag-gating behavior, including Mermaid requiring both flags.
- Unit tests for relative-path resolution against block metadata that stores the original working directory.
- Unit tests for fallback behavior when a file is missing, unsupported, or Mermaid rendering is disabled or fails.
- Automated tests covering block-level copy serialization for mixed content that includes text, images, Mermaid diagrams, code blocks, and other special Markdown-rendered content.
- Integration tests that exercise AI block list rendering end to end, including restored-block behavior and mixed-content copy behavior.
- Manual validation with an AI response containing a relative-path PNG image.
- Manual validation with an AI response containing an absolute-path JPEG image.
- Manual validation with one example each for SVG, GIF, and WebP image references.
- Manual validation that an inline image run renders as a horizontal row with the path label under each image and the expected link highlighting.
- Manual validation that consecutive standalone image lines render as an `Images N` card with thumbnail-plus-path rows.
- Manual validation that clicking the source path for a rendered PNG or other binary image opens it with the platform-default app rather than Warp's editor.
- Manual validation that a missing local file falls back to the original Markdown image syntax.
- Manual validation that an unsupported format falls back to the original Markdown image syntax.
- Manual validation that a Mermaid diagram renders in the new framed block treatment with a white inner canvas.
- Manual validation that Mermaid in the AI block list does not render unless both required flags are enabled.
- Manual validation that a Mermaid render failure falls back to raw Mermaid Markdown.
- Manual validation of a response containing text, one image, more text, and a Mermaid diagram.
- Manual validation of a response containing multiple images in sequence.
- Manual validation that clicking a rendered inline image opens the shared fullscreen lightbox with the expected image selected.
- Manual validation that clicking a rendered Mermaid diagram opens the same fullscreen lightbox treatment.
- Manual validation that previous/next navigation in the lightbox walks the rendered visuals from the same AI message in source order.
- Manual validation that Escape and scrim click dismiss the lightbox and return the user to the block list.
- Manual validation that existing text/code/table selection behavior is unchanged after image rendering is added.
- Manual validation that block-level copy produces the expected text plus raw Markdown image or Mermaid source in document order.
- Manual validation that right-click copy on a rendered image or Mermaid diagram copies the underlying Markdown source.
- Manual validation that the same response renders correctly after the conversation is restored from history.
- Regression validation that disabling the feature flag restores the current raw-Markdown behavior.
