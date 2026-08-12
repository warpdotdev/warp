# Kitty Graphics Protocol: Unicode Placeholders

GitHub issue: https://github.com/warpdotdev/Warp/issues/6210
Protocol reference: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders

## Summary

Warp accepts kitty graphics commands that create *virtual placements* (`U=1`) and renders images in the cells that contain the Unicode placeholder character `U+10EEEE`. A program transmits an image, creates a virtual placement, and writes placeholder text into the grid. Warp then shows the correct part of the image in each placeholder cell. The image behaves as text: it scrolls, reflows, and disappears with the cells that hold it.

## Problem

Modern image-display tools use Unicode placeholders because that is the only kitty placement method that survives scrollback, reflow, and multiplexers. Today Warp parses the `U` key and then rejects the command. Because these tools typically set `q=2` (suppress replies), the failure is silent: the image transmission succeeds, the placement is dropped, and the user sees rows of missing-glyph boxes ("tofu") where the image should be.

## Goals / Non-goals

Goals:

- Accept virtual placements on the two placement paths (`a=T` and `a=p`).
- Render placeholder cells as image fragments wherever Warp already renders kitty images.
- Never show tofu for placeholder cells, in any state.

Non-goals:

- New kitty transmission mediums, animation (`a=f`, `a=a`), or z-index compositing changes.
- Placement-id selection through the cell underline color (SGR 58). Warp does not track underline color in the grid; this is a follow-up.
- Relaying placeholders through SSH/tmux passthrough beyond what plain cell text already provides.
- Changes to Warp's existing non-virtual kitty placements or iTerm inline images.

## Behavior

In this section, "the client" is the program that writes kitty graphics escape sequences to the terminal, and "the user" is the person who looks at the Warp window.

### Accepting virtual placements

1. When a graphics command carries `U=1`, Warp creates a virtual placement instead of a visible one. This applies to both `a=T` (transmit and display) and `a=p` (display a stored image). Warp must not reject `U=1` on either path.
2. A virtual placement draws nothing by itself. Creating one causes no visible change until placeholder cells exist.
3. A virtual placement does not move the cursor, independent of the `C` key.
4. The `c` (columns) and `r` (rows) keys give the placement size in grid cells. The image is scaled to exactly fill that `c × r` cell rectangle, and each cell of the rectangle corresponds to one tile of the scaled image. When the client omits `c` or `r`, Warp computes the missing values from the image pixel size and the current cell size, the same as kitty.
5. Warp answers virtual-placement commands with the protocol's normal success and error replies, honoring the `q` key (`q=1` suppresses success, `q=2` suppresses both). An invalid command (for example an unknown image id) with `q=0` gets an error reply, not silence.
6. A second virtual placement with the same image id and placement id replaces the first. Placeholder cells that reference it show the new placement's scaling from then on.
7. A delete command (`a=d`) that matches a virtual placement removes it, and a delete that removes the underlying image also removes its virtual placements. Placeholder cells that referenced a removed placement render blank from then on. Deleting a virtual placement never clears or rewrites the placeholder text cells themselves.

### Placeholder cells

8. A placeholder cell is a grid cell whose base character is `U+10EEEE`. Placeholder cells arrive as ordinary printed text and occupy the grid like any other single-width character: printing them moves the cursor, they can be overwritten, erased, scrolled, and stored in scrollback.
9. Each placeholder cell resolves to an image and a tile as the kitty protocol defines:
   - The image id comes from the cell's foreground color: a 24-bit RGB foreground supplies the low 24 bits; a 256-indexed foreground supplies ids 0–255. A third combining diacritic, when present, supplies the most significant byte of the id.
   - Warp ignores the placement-id encoding (the cell's underline color) in this version. Every placeholder cell resolves against the most recently created virtual placement of its image, which the kitty protocol permits when the placement id is absent or zero.
   - The tile row and column come from the first and second combining diacritics, using the kitty row/column diacritic table.
10. When a placeholder cell omits diacritics, Warp infers the tile position with the same continuation rules as kitty: the cell continues horizontally from the placeholder cell directly to its left when that cell has the same image id; a cell with only a row diacritic starts that row at column 0 and continues from there.

### Rendering

11. A placeholder cell whose image and virtual placement both exist renders exactly its tile of the scaled image, aligned to the cell rectangle. A complete `c × r` block of placeholder cells is visually identical to the whole image scaled to that block.
12. Placeholder cells are independent. A subset of the block renders only those tiles. Duplicated, reordered, or split cells each render the tile their diacritics name. Placeholder runs for different images can share a row.
13. A placeholder cell renders blank — no glyph, no tofu, no error marker — in every case where a tile cannot be shown: the image id is unknown, no virtual placement matches, the image or placement was deleted, the tile row or column is outside the placement's `c × r` rectangle, or the diacritic encoding is invalid. The cell's background still paints normally.
14. Warp never paints a missing-glyph box for `U+10EEEE` or for the row/column diacritics in a placeholder cell, in any state, including before the image arrives and after it is deleted.
15. Because placeholder cells are text, the image follows the text. When the terminal scrolls, resizes, or reflows, the fragments move with their cells, including cells in scrollback that scroll back into view. Warp keeps showing the image for placeholder cells as long as the referenced image and placement exist, without a time or scroll-distance limit, subject to the existing image storage quota.
16. Image tiles respect transparency: the cell background shows through transparent pixels.
17. The cursor may sit on a placeholder cell. Cursor rendering and cursor movement are unchanged.
18. Selection treats placeholder cells as text cells: they highlight when selected, and copying them copies the underlying characters (the placeholder codepoint and its diacritics), the same as kitty. Find-in-terminal never matches image content.
19. Unicode-placeholder images render on every surface and mode where Warp's existing kitty image rendering works today (blocks, alternate screen, prompt output), and are gated by the same availability as that rendering. A viewport that contains no placeholder cells has no measurable rendering cost from this feature.

### End-to-end check

20. Running `kitten icat --unicode-placeholder <file>` inside Warp shows the image inline, occupying the columns and rows the tool requested, with no tofu, and the image scrolls away with the surrounding output. This is the acceptance flow for the feature.
