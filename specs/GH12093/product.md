# Product Spec: Render image previews in the code review panel

**Issue:** [warpdotdev/warp#12093](https://github.com/warpdotdev/warp/issues/12093)
**Figma:** none provided

## Summary

When a change includes image files, the code review panel shows each one as "Binary file — no diff available." Instead, the panel should render an inline image preview so a reviewer can inspect the visual change — added, removed, or modified — without leaving Warp.

## Behavior

1. A changed file whose bytes decode as a supported format (PNG, JPEG/JPG, GIF, WebP) renders an **inline image preview** in place of the "Binary file — no diff available" placeholder. Its row in the file list/sidebar appears exactly as other changed files do (path, status indicator, expand/collapse); no special affordance is needed to reach the preview.

2. The preview content depends on the file's change status:
   - **Added / new / untracked:** the new image, labeled added.
   - **Deleted:** the old image (base revision), labeled removed.
   - **Modified:** both the old (base revision) and new (working tree) image, shown together for comparison.
   - **Renamed/copied, no content change:** a single image; the rename is reflected by the existing path/status presentation, not duplicated as a fake before/after.
   - **Renamed/copied with content change:** treated as a modification (old and new together) alongside the existing rename presentation.

3. Each image shows lightweight metadata — at minimum pixel dimensions (width × height) and file size, per side for a modification, so dimension/size changes are visible even when the visual difference is subtle. No line-based additions/deletions count is shown for image files (no misleading "0/0").

4. Animated GIF/WebP render a static first frame in the list rather than autoplaying (multiple simultaneous animations are distracting and costly).

5. **Size limit:** an image whose source bytes exceed a dedicated image-preview byte cap (≈10 MB, independent of the text diff-size limits) is not decoded and falls back to the placeholder noting it is too large to preview. For a modification the cap is evaluated per side; one oversized side shows the note while the other still previews.

6. **Decode/read failure:** if an image can't be read (missing blob, git error) or decoded (corrupt/truncated), it falls back to the placeholder. A failed preview never blocks the rest of the panel and never shows a broken-image artifact.

7. **Partial availability:** if only one side's bytes are obtainable (e.g. the base blob can't be read), the preview shows that side and indicates the other is unavailable, rather than failing the row.

8. **Misclassification guard:** a file with an image-looking extension whose bytes don't decode as a supported image (a `.png` that's actually text, a Git LFS pointer) falls back to the placeholder. Formats outside the supported list are never previewed even if the bytes are technically valid images.

## Non-goals

- **Remote / PR diffs** — those arrive over an RPC carrying only text content, so image bytes need a new proto + server path. Remote image files keep the placeholder.
- **SVG** — SVGs are text/XML and already render as a normal text diff, so they don't hit this placeholder.
- **Full-size / zoom (lightbox)** — v1 ships the inline preview only, as it is possible to open the full image with an existing "Open file" button.
