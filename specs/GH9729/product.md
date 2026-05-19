# Image preview in the Code Editor file pane: Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/9729
Figma: none provided

## Summary

Clicking a supported image file (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`) in the File Tree currently routes through the binary-file fallback and hands the file to the OS default app, or, for SVG, opens raw XML in the Code Editor. v1 fixes this by routing those extensions through the existing **Lightbox** overlay component, which already renders a single image, dims the workspace behind a scrim, and dismisses on Escape, scrim-click, or close button.

v1 is intentionally minimal: a **single, fully modal, single-image** Lightbox. No sibling navigation, no zoom, no animated playback, no thumbnail strip. Those are tracked as follow-ups so v1 can ship without coupling to features that need more design and implementation work.

## Problem

For image files the File Tree either bumps the user out to Finder/Preview (raster formats hit `is_binary_file` and fall through to `FileTarget::SystemGeneric`) or shows raw XML (SVG is text-classified and lands in the Code Editor). There is no way today to glance at an image asset (an icon, a screenshot, a logo, an SVG) from inside Warp without leaving the window.

The maintainer-preferred shape for v1 is the existing Lightbox component. It already renders an image with `Image::new(asset_source).contain()`, has open/close lifecycle driven by `WorkspaceAction::OpenLightbox`, and overlays the active workspace without disturbing tab and split state. Reusing it gives users a fast preview affordance in one release without committing the codebase to a parallel "image tab" surface that would need its own restore, dragging, splitting, and persistence story.

## Goals

- Clicking a supported image file (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`) in the File Tree opens the existing Lightbox overlay over the active workspace window, showing only that one image.
- The Lightbox uses the existing image rendering pipeline (`Image::new(...).contain()` backed by `ImageType` in `crates/warpui_core/src/image_cache.rs`); no new image-rendering element is introduced.
- Loading and decode-error states for the image surface inside the Lightbox without crashing.
- Pre-read and per-decode size limits so a maliciously large or malformed file cannot exhaust process memory before or during decode.
- Telemetry distinguishes the image-preview open path from the existing `MarkdownViewer`, `CodeEditor`, and `SystemGeneric` open paths via the existing `CodePanelsFileOpened.target` field.

## Non-goals

- **Sibling navigation.** Left/Right arrow keys do not step through other images in the directory in v1. The Lightbox is single-image only. Sibling navigation is tracked as a follow-up.
- Zoom (`Cmd+=`, `Cmd+-`, `Cmd+0`), trackpad pinch-zoom, click-drag pan.
- Status footer (filename, pixel dimensions, file size, format string).
- Continuous playback of animated GIF/WebP in the Lightbox. v1 reuses today's Lightbox rendering, which already shows only the first frame because it never calls the GPUI `Image::enable_animation_with_start_time` plumbing. v1 hardens the global decoder against pathological animated input (frame-count and total-pixel budgets), but the changelog section continues to animate exactly as today; no animated surface that exists today is regressed.
- Animation play/pause control and persistent pause state.
- EXIF orientation rotation and embedded ICC color profile honoring.
- Visible thumbnail strip.
- Format support beyond `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`. HEIC, HEIF, BMP, TIFF, AVIF, ICO are out of scope for v1.
- Magic-byte content sniffing when extension and content disagree.
- Right-click context menu on the image (Copy Image, Copy file path, Reveal in Finder, Attach as Agent context).
- Drag-out from the Lightbox into the Agent input as an attachment.
- Disk-backed thumbnail cache and configurable cache size cap.
- Image diff across git revisions, remote URL preview, image editing, PDFs.
- Terminal-grid inline images (tracked separately in #1693, #5286).
- Retroactive replacement of the OS handoff for image extensions outside the supported set; those continue to land in `SystemGeneric`.

## User Experience

### Opening

- Single-clicking a supported image file in the File Tree opens the Lightbox over the active workspace window. No Code Editor tab is opened and no persistent tab of any kind is created.
- Keyboard-only entry uses the same path: navigating to the file in the tree with the arrow keys and pressing Enter opens the Lightbox identically to a click. (The File Tree's keyboard activation already resolves through the same `open_file` function as the click path.)
- The Lightbox attaches to the workspace window that initiated the click; multi-window users see the overlay only on the originating window, since `Workspace.lightbox_view` is per-workspace.

### Modality (the Lightbox is fully modal in v1)

While the Lightbox is open, **the entire workspace window is inert**, including the File Tree, terminal panes, Code Editor panes, and tab bar. The scrim covers the workspace and intercepts pointer input. Keyboard input is routed to the Lightbox.

To open another file, the user dismisses the Lightbox first (Escape, scrim click, or close button), then clicks the next file. There is no "click another file in the File Tree to swap the Lightbox image" path in v1; that interaction is reserved for the sibling-navigation follow-up, where it can be designed deliberately rather than emerging from focus-stealing as a side effect.

This is the same modality contract the Lightbox already enforces in its existing screenshot/artifact use cases. v1 does not loosen it.

### Inside the Lightbox

- The image renders centered, fit to the viewport via `Image::new(...).contain()`: aspect ratio preserved, never upscaled past the image's native pixel dimensions.
- The Lightbox renders as a child of the workspace's main render `Stack`, so the scrim covers the whole workspace (all panes are dimmed, not just the active pane). This matches today's screenshot/artifact Lightbox behavior.
- Underlying panes remain visible behind the scrim and do not receive input while the Lightbox is open.
- While bytes are still being read or decoded, the existing loading indicator is shown.
- If the file fails the pre-read size cap, fails to read, fails to decode, or exceeds the decoded-pixel cap, the Lightbox shows a non-blocking error state that includes the filename and a short reason. The Lightbox never crashes the workspace.
- SVG renders via the existing `usvg` + `resvg` pipeline used by `ImageType`.
- Animated GIF and animated WebP files render their **first frame only**. The Lightbox today never calls `Image::enable_animation_with_start_time`, so animated raster formats already render as a static first frame in this surface; v1 reuses that behavior unchanged. Continuous playback in the Lightbox is tracked as a follow-up. v1 hardens the shared `ImageType::try_from_bytes` decoder against pathological inputs (oversized dimensions, animated frame-count and total-pixel budgets, SVG intrinsic-dimension cap), and these caps are sized well above any in-tree consumer's legitimate inputs; the changelog section's animated-image rendering is unchanged. The only user-visible effect of the global hardening is that pathological or malicious files now surface as decode errors at every consumer of `ImageType::try_from_bytes` instead of silently allocating gigabytes of memory.

### Dismiss and focus restoration

- The Lightbox dismisses on Escape, on a click outside the image (scrim click), and via its close (×) button. All three paths emit `LightboxViewEvent::Close`.
- On dismiss, focus returns to the previously-active tab pane via the existing `focus_active_tab` call in the `OpenLightbox` close handler. If the previously-active pane has been closed in the meantime, focus falls back to whatever `focus_active_tab` resolves to at that moment.

### Filesystem mutations during preview

- If the file is deleted, renamed, or moved while the Lightbox is open, the entry stays on whatever bytes the asset cache already loaded and renders normally. The Lightbox does not refetch on its own; the user can dismiss and reopen.
- If the read started but had not completed when the file was removed, the read fails and the Lightbox surfaces the per-entry error state described above.

### Performance posture

- Image bytes are read on the background executor; the bytes-to-RGBA decode itself runs on the foreground executor (`AssetCache::load_asynchronously` invokes `try_from_bytes` on the foreground executor before publishing the loaded asset). v1 does not change this. The decoder caps below bound the worst-case decode time, and the loading indicator is shown until decode completes. There are two distinct caps that fire at two different points and they are complementary, not redundant: a **pre-read file-size cap** rejects oversize files before any byte is read, and **decode-time dimension and pixel caps** reject files whose declared image dimensions or decoded pixel count exceed the budget. A file that exceeds the pre-read cap never reaches the decoder; a file that passes the pre-read cap (e.g. a 3 MB JPEG header that decodes to 100000×100000 pixels) is still rejected by the decode-time caps.
- Moving decode to the background executor is a follow-up that affects every Lightbox surface, not just file-tree previews; it is intentionally not entangled with this v1.
- The pre-read metadata check also runs on the foreground executor in v1. On a stalled NFS/sshfs/FUSE mount this can briefly freeze the workspace until the FS timeout; v1 accepts this tradeoff to keep the new code path small. Moving the metadata check off-thread is enumerated as part of the same follow-up.

### Limits (visible to the user only as the per-entry error state)

The caps below fire at distinct points in the load pipeline and bound distinct failure modes. They do not overlap: a file can fail any one of them while passing the others.

- **Pre-read file-size cap.** Rejects files whose on-disk size exceeds a fixed cap **before any byte is read into memory**. Fires before the asset-cache load even begins. Bounds disk-to-memory I/O. Example: a 100 MB PNG with normal pixel content fails this cap; the Lightbox opens directly into the per-entry error state with the filename shown.
- **Bounded read.** The actual file read in the asset cache is bounded by a per-content ceiling: the first 1 KB of the opened file is peeked, and if the peek looks like XML/SVG content the read is capped at the tighter SVG-specific ceiling, otherwise at the uniform raster ceiling. This prevents a file that grows past the pre-read metadata check, or a path swapped to a special device or FIFO between the metadata check and the open, from bypassing the limit. Fires during the read step.
- **Decode-time dimension and pixel caps.** Rejects files whose declared image **dimensions** or `width × height` **pixel count** exceed fixed maximums. Fires during decode, after the bytes have been read; the file size may be small (and within the pre-read cap) while the decoded image would still exceed the pixel budget. Example: a 3 MB JPEG header that decodes to 100000×100000 pixels passes the pre-read cap but fails this cap.
- **Animated frame budget.** Rejects animated GIF and animated WebP files whose total frame count or whose summed pixels-across-frames exceed fixed maximums. Fires mid-iteration during decoding so pathological inputs do not first materialize every frame into memory.
- **SVG-specific bounded-read cap.** Rejects SVG **content** whose byte count exceeds a tighter SVG-only cap (smaller than the raster cap), because XML parsers have super-linear cost in input size and a large SVG can consume parser memory disproportionate to its file size. Fires **at the asset-cache bounded read step**, not at the pre-read metadata stage: after the file has been opened, the first 1 KB is peeked and tested for XML/SVG shape. If the peek looks like SVG XML, the read is capped at the tighter SVG ceiling regardless of the file's extension; otherwise the uniform raster ceiling applies. The pre-read metadata cap above is uniform across formats because at that stage no bytes have been read and the format cannot be known. The two caps are complementary: the pre-read cap bounds disk-to-memory I/O for any oversize file, and the SVG-specific cap is defense-in-depth for files that pass the pre-read cap but whose content would feed the XML parser. Content-keying (rather than extension-keying) closes the case where SVG content hides under a non-`.svg` extension.
- **SVG content-sanity check.** Rejects SVG files whose contents are not XML-like (e.g. a binary blob renamed `.svg`) before the parser runs. Fires before parse, after read.
- **SVG intrinsic-dimension cap.** Rejects SVG files whose declared `<svg width=... height=...>` exceeds a fixed maximum render dimension. Fires at parse time, before rasterization.

The exact cap values are defined in the tech spec. From the user's perspective, the contract is:

- A normal photo (4000×3000), a typical screenshot, a typical 4K image, a typical icon SVG, and a typical app-asset PNG all open.
- A 3 MB JPEG with normal pixel count opens (passes both pre-read and decode-time caps).
- A 100 MB PNG with normal pixel content does not open (fails the pre-read cap, never reaches decode).
- A 3 MB JPEG header that decodes to 100000×100000 pixels does not open (passes pre-read, fails decode-time).
- A 100 MB PNG that also decodes to 100000×100000 does not open (fails pre-read first; decode is never reached).
- A many-thousand-frame animated WebP, a 200-byte SVG declaring 200000×200000 dimensions, and a 4.5 MB SVG containing 50000 deeply nested `<g>` elements all do not open and do not crash. The 4.5 MB SVG case is rejected at the asset-cache bounded read step (above the content-keyed SVG ceiling) before any parse runs, regardless of whether the file is named `.svg` or hides under another extension.

### Accessibility

- The keyboard-only entry path (tree navigation + Enter, then Escape to dismiss) is the documented a11y flow and is supported via existing keyboard navigation.
- The active image's filename is rendered as the visible `description` slot in the Lightbox, so it is reachable via standard text-focus a11y traversal.
- v1 does **not** ship a screen-reader label on the rendered image element itself. The GPUI `Image` element does not currently expose an accessibility-label API; adding that plumbing (so VoiceOver announces "image, screenshot.png" rather than a generic "image") is the first accessibility follow-up. The product spec was previously narrower on this point; verification against the codebase showed the API does not exist, so v1 is honest about what ships.
- High-contrast mode: the existing scrim color (RGBA 0,0,0,230) and existing close-button styling are unchanged in v1; behavior is identical to today's Lightbox usage in screenshot/artifact previews.
- Reduced-motion preference: v1 does not add new motion. Any open/close fade is whatever the existing Lightbox already does.

### Unaffected surfaces

- Terminal escape-sequence handling, inline-image protocols on the terminal grid, and the agent-mode image-attach pipeline are unchanged.
- The changelog section's animated-image rendering is unchanged.
- The artifact / screenshot Lightbox call sites are unchanged in behavior; they continue to render via the same component.
- Files outside the supported extension set continue to follow their existing target (Code Editor for text, `SystemGeneric` for binary, external editor per user preference, etc.).

## Success criteria

1. Clicking each of `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg` in the File Tree opens the Lightbox showing that single image.
2. While the Lightbox is open, clicks on the File Tree, terminal panes, Code Editor panes, and tab bar do nothing; the user must dismiss first.
3. Escape, scrim click, and × button all dismiss the Lightbox and restore focus to the previously-active tab pane.
4. A corrupt image, an unreadable file, and a mislabeled file (e.g. a `.png` containing tarball bytes) each surface as the per-entry error state with the filename shown; no crash and no permanent spinner.
5. A file above the pre-read file-size cap (e.g. a 100 MB `.png`) opens directly into the per-entry error state via the pre-read cap without being read into memory or decoded.
6. A symlink to a special file (e.g. `/dev/zero`) and a path swapped to a FIFO between the pre-read stat and the open syscall both open directly into the per-entry error state; the `is_file()` rejection fires before any read is attempted, and the open does not hang.
7. A file below the pre-read size cap but above the decode-time dimension or pixel cap (e.g. a small-on-disk JPEG header declaring 100000×100000, or a 10000×10000 PNG that fits inside the pre-read cap) surfaces as the per-entry error state at decode time via the decoded-dimension or decoded-pixel cap; no partial render, no OOM. Critically, this is a different failure point from criterion 5: the bytes are read, then the decoder rejects.
8. An animated GIF and an animated WebP with normal frame counts open and render their first frame statically. The changelog section continues to animate as it does today (unchanged).
9. An animated file with a pathological frame count or pathological per-frame size surfaces as the per-entry error state mid-decode, without first materializing every frame.
10. SVG renders via `usvg` / `resvg`. A small SVG declaring a pathological `<svg width="200000" height="200000">` surfaces as the per-entry error state via the SVG intrinsic-dimension cap at parse time, before rasterization. Separately, an over-cap SVG file (e.g. a 4.5 MB SVG containing 50000 deeply nested `<g>` elements) surfaces as the per-entry error state via the SVG-specific content-keyed cap at the asset-cache bounded read step, before the parser runs at all. The same cap applies whether the file is named `.svg` or `.png` — what matters is that the first 1 KB peek looks like XML/SVG. A non-XML binary blob renamed `.svg` surfaces via the content-sanity check, also before the parser runs.
11. Telemetry events for image opens are distinguishable from `MarkdownViewer` / `CodeEditor` / `SystemGeneric` opens via the `target` field on `CodePanelsFileOpened`.
12. Non-image binary files (`.zip`, `.mp3`, `.exe`, `.pdf`, `.bmp`, `.tiff`, `.ico`, etc.) continue to route to `SystemGeneric` exactly as before; no regression.

## Validation

Tied 1:1 to the success criteria above and detailed in the tech spec's Testing and validation section. In summary:

- Unit tests for the resolver returning `FileTarget::ImagePreview` for each supported extension and `SystemGeneric` for each non-image binary extension.
- Unit tests for the pre-read size cap rejecting an oversize file before any decode, and for the bounded read in the asset cache rejecting a file that grows past the cap after the metadata stat.
- Unit tests for the decoder caps: rejecting a synthesized PNG header declaring dimensions above the cap, rejecting a header declaring a pixel count above the pixel cap, rejecting an animated fixture above the frame-count cap, rejecting an animated fixture above the total-pixel-budget cap, rejecting an SVG declaring intrinsic dimensions above the SVG render cap.
- Regression unit tests confirming the existing global `ImageType::try_from_bytes` continues to construct `AnimatedBitmap` for legitimate animated WebP and GIF (no regression for the changelog).
- Manual: each behavior listed under User Experience above against fixtures including a small image, a 4000×3000 photo, a 5000×5000 PNG (below caps), a 10000×10000 PNG (above the decode-time dimension and pixel caps), a 100 MB sparse-file `.png` (above the pre-read cap), a symlink to `/dev/zero` (rejected for non-regular-file), a FIFO created via `mkfifo` then named with an image extension (rejected fast via Unix `O_NONBLOCK` + post-open `is_file()`, no hang), a normal animated GIF, a 500-frame animated GIF (above the frame-count cap), a normal animated WebP, a small SVG (well below the SVG cap, parses and renders normally), a `<svg width="200000" height="200000">` SVG (above the intrinsic-dimension cap), a 4.5 MB SVG with 50000 deeply nested `<g>` elements (above the SVG-specific content-keyed cap, rejected at the bounded read before any parse runs), 4.5 MB of SVG XML in a file named `evil.png` (above the same cap, content-keyed selection picks the SVG ceiling despite the `.png` extension), a 5 MB binary blob renamed `.svg` (rejected by either the SVG byte cap or the content-sanity check), a `.png` containing tarball bytes (mislabeled), a corrupt PNG, and the workspace's existing changelog page (regression check on animation).

## Alternatives considered

- **A new `ImagePreview` tab variant** with its own restore/drag/split story. Rejected by the maintainer in the issue thread; the Lightbox overlay matches the v1 surface area better and avoids committing the codebase to a parallel image-tab system.
- **Routing image files through the existing Code Editor with a binary-buffer renderer.** Rejected because the Code Editor's tab/pane model does not have an image-rendering element and would need substantially more new code than dispatching the existing Lightbox.
- **Including sibling navigation in v1.** Earlier drafts of this spec included Left/Right arrow navigation across the directory's image files. That introduced a long tail of design work (modal contract conflicts, lazy-preload window, sibling-list cap algorithm, NFS scan responsiveness) that is unjustified for the first release. The single-image v1 ships the user-visible win (no more bounce to Finder, no more raw SVG) without those entanglements; sibling navigation is tracked as the first follow-up.
- **Including animated playback in v1.** The existing Lightbox does not animate, and wiring `Image::enable_animation_with_start_time` requires per-frame redraw plumbing and a play/pause UX. First-frame static is a defensible v1 because it is unambiguously the image the user clicked, and it matches the Lightbox's existing behavior for the same formats today.
- **Always opening through the OS default app.** This is today's behavior for raster formats and is precisely the user complaint in the issue; rejected.
- **Only fixing SVG (since SVG is the worst-looking case today, opening as raw XML).** Rejected as a half-fix; the issue explicitly calls out raster formats and SVG together.

## Open questions

- The tech spec lands on concrete values for the pre-read cap, decoded-dimension cap, decoded-pixel cap, animated-frame budget, and SVG intrinsic-dimension cap. Product is comfortable with these values as long as the user-visible contract holds: a 4000×3000 photo opens, a typical screenshot opens, a typical app-asset PNG/SVG opens, a 100 MB or larger file is rejected before being read, and a 10000×10000 PNG is rejected at decode time. The follow-up for moving decode and metadata to the background executor may relax the dimension/pixel caps once the foreground-stall risk is removed.
