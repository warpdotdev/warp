# APP-3857: Memory-safe animated image rendering in markdown

## Summary

Warp should continue to display animated image sources embedded in rendered markdown without causing large, unbounded memory spikes. In rendered markdown and notebook views, animated images should appear as inline static previews of their first frame, matching the current markdown UX while avoiding the multi-frame decode and resize work that currently drives excessive memory usage.

## Problem

Today, large animated images in markdown can drive Warp into multi-GB memory usage. That makes a document viewer path unreliable: opening or scrolling a markdown file can cause severe jank, memory pressure, or crashes even though the user only needs to read the document. Users need to be able to open and view markdown documents containing animated images without Warp becoming unstable.

## Goals

- Allow markdown documents containing animated images to open and render inline in Warp's markdown viewer.
- Preserve the existing markdown viewing experience for animated image sources by rendering them as inline first-frame previews.
- Bound memory use so a single image or document does not cause the multi-GB spike seen in the Petra repro.
- Degrade gracefully when even extracting a preview frame is expensive or fails.
- Preserve authored markdown, selection, copy, and export behavior.

## Non-goals

- Adding new playback controls, autoplay, zoom controls, or user preferences for markdown image animation.
- Reworking ordinary static image rendering behavior beyond what is needed to keep markdown image handling safe.
- Expanding this ticket to every image-rendering surface in Warp outside the markdown and notebook viewer.
- Rewriting the user's markdown or image assets on disk.
- Guaranteeing perfect animation fidelity for very large assets if doing so would violate the memory-safety goal.

## Figma / design references

Figma: none provided

This is a behavior-focused change. Existing markdown image visuals and layout should be reused unless the image falls back to a static preview.

## User experience

### In-scope surfaces

- Rendered markdown files and notebook markdown content that use Warp's existing markdown image blocks.
- Raw markdown and editable markdown views remain unchanged and continue to show the source markdown.

### Default behavior

- Animated image sources referenced from markdown render inline in document flow, using the same sizing and layout rules that markdown images already use.
- In rendered markdown, animated GIFs and animated WebPs are shown as a static preview of their first visible frame.
- This iteration adds no new controls, settings, badges, or toasts for normal animated-image preview rendering.

### Resource-safe rendering behavior

- Warp must not require multi-GB memory usage to display an animated image source in markdown.
- Warp should do only the work required to show a static first-frame preview rather than decode, cache, or resize the full animation.
- The document layout must remain stable. Rendering an animated image preview must not cause surrounding markdown blocks to overlap, jump unexpectedly, or reserve obviously incorrect space.

### Graceful handling for problematic animated assets

- If Warp can decode enough of an animated image to extract a first visible frame, it should show that static preview.
- The static preview should occupy the same rendered markdown block footprint that the image would otherwise use, so this behavior does not create a separate layout mode.
- If the image cannot be decoded even far enough to extract a preview frame, existing image-load failure behavior remains unchanged.
- Preview-only handling is silent in this iteration. Warp does not add a warning label or toast merely because the source asset is animated.

### Failure handling

- If an image is too expensive to animate safely but can still be decoded enough for a preview, the user sees the static preview rather than a broken or empty region.
- If the image itself cannot be loaded or decoded at all, existing image-load failure behavior remains unchanged.
- One problematic animated image must not prevent the rest of the markdown document from rendering and remaining usable.

### Editing, copy, and export behavior

- Warp does not rewrite the source markdown. The authored `![](...)` reference remains unchanged.
- Selection, copy, and export behavior continue to preserve the original markdown image reference rather than serializing a transformed static asset.
- This ticket does not introduce a new user-visible export format for rendered animated images.

### Scope boundaries for this iteration

- This spec applies to markdown document viewing, not to terminal image protocols, chat attachments, or other non-markdown image surfaces.
- This iteration optimizes for "can open and read the document safely" rather than for pixel-perfect playback of extremely large animations.

## Success criteria

1. Opening the Petra repro markdown associated with `APP-3857` no longer causes the multi-GB memory spike represented by `heap-profile-petra-1.pb`.
2. The affected markdown document remains readable instead of causing Warp to freeze, crash, or become unusably slow.
3. Animated GIFs and animated WebPs in rendered markdown display as static first-frame previews without decoding and resizing every frame into memory.
4. Large animated assets no longer cause runaway memory growth just to display a markdown preview.
5. Preview rendering preserves the same document layout footprint as the rendered image block.
6. A document containing one expensive or malformed animated image still renders the rest of its markdown content correctly.
7. Raw markdown, selection, copy, and export continue to preserve the original markdown image reference.
8. This change does not introduce a visible regression for ordinary static images or for modest animated image sources viewed as static previews in the same markdown viewer.

## Validation

- Open the Petra repro markdown document associated with `APP-3857` and verify that Warp remains usable while the document is opened and scrolled.
- Measure memory while opening and scrolling the repro document and confirm the previous multi-GB spike pattern is gone rather than merely delayed.
- Verify that a modest animated GIF or animated WebP in markdown renders as a stable first-frame preview in rendered view.
- Verify that a large animated image renders a stable first-frame preview instead of blank content, a broken-image state, or a crash.
- Verify that raw markdown and markdown export still preserve the original `![](...)` reference for animated images.
- Verify that markdown documents containing a mix of text, static images, and animated images still lay out correctly and remain selectable and copyable.
- Add automated coverage for:
  - first-frame preview rendering for animated GIF and animated WebP markdown images
  - graceful handling for an oversized or expensive animated asset
  - a regression guard against the previous all-frames-at-once memory behavior at the viewer or image-cache layer

## Open questions

- Do we want a numeric memory budget for markdown animated-image preview rendering in a follow-up, or is "no multi-GB spike on the known repro" sufficient for this iteration?
- In a future iteration, should Warp expose opt-in playback for animated images in markdown once a memory-safe animation path exists?
