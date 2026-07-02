# GH12243: Prevent RowIterator crash after clear resize truncates wide characters
Issue: https://github.com/warpdotdev/warp/issues/12243
## Summary
Warp Preview must not crash when terminal output containing wide characters is resized during a full-grid clear flow. Terminal rows produced by a clear-driven no-reflow resize must remain valid when later rendered, scrolled, or restored from scrollback-like storage.
## Problem
The reported crash happens after a command-enter / clear-hook / resize sequence. The terminal grid can be shrunk at a column boundary that splits a wide character from its spacer, leaving a retained row that later cannot be safely restored for rendering or scrolling.
This is a crash fix, not a user-facing feature. The user-visible requirement is that Warp continues to render, resize, and scroll terminal output without aborting, even when CJK or other double-width characters sit exactly at a resize boundary.
## Goals / Non-goals
Goals:
- Preserve terminal stability during clear-driven resize flows that truncate wide-character pairs.
- Preserve the invariant that rows produced by the clear-driven no-reflow resize path do not contain an orphaned trailing wide-character marker.
- Keep existing reflow behavior for ordinary terminal resize, line wrapping, scrollback, and wide-character continuation rows.
Non-goals:
- Reworking all terminal wide-character handling.
- Claiming to fix every public `RowIterator::next` crash report unless the same producer path is proven.
- Changing the visual semantics of valid wrapped wide characters.
## Behavior
1. When Warp receives a full-grid clear sequence and the terminal is resized before the command is finished, the active grid resizes without reflowing old terminal output into scrollback.
2. If that no-reflow resize shrinks the terminal width and the retained final cell is a wide character, the resulting row remains valid for later rendering, scrolling, and scrollback-like storage. Warp must not leave a final-column wide character that requires a spacer outside the row bounds.
3. For that invalid trailing wide character, Warp resets the retained final cell's foreground/content state instead of preserving the leading glyph as a single-cell character. The reset preserves the cell background, matching existing clear/overwrite behavior for wide-character boundary repairs.
4. Valid wide-character pairs wholly inside the new width remain unchanged. A wide character whose spacer is still retained continues to occupy two cells and must still materialize with its matching spacer.
5. Wide-character spacers wholly outside the new width are discarded with the rest of the overflow content. If discarding overflow leaves the retained row ending in `WIDE_CHAR`, that retained leading cell is reset; otherwise discarding overflow content must not mutate unrelated retained cells.
6. Rows produced by this clear-driven no-reflow resize path must not contain:
   - a `WIDE_CHAR` marker without a following `WIDE_CHAR_SPACER` in the same row, or
   - a `WIDE_CHAR_SPACER` marker without a preceding `WIDE_CHAR` in the same row.
7. Ordinary reflowing resize behavior is unchanged. When resize is allowed to reflow content, valid wide characters that cross a wrap boundary continue to use the existing leading-spacer semantics for wrapped rows rather than being cleared, flattened, or silently narrowed.
8. Screen-mode routing is unchanged. Unfinished primary grids with full-grid clear behavior continue to use the no-reflow clear path; finished primary grids continue to use normal scrollback behavior; alt-screen grids continue to resize without flat-storage scrollback.
9. The existing clear-resize scrollback-width regression remains fixed. Resizing under full-grid clear behavior continues to keep active grid width and flat-storage width in sync.
