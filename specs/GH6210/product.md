# Product Spec: Kitty images in Unicode placeholder cells

**Issue:** [warpdotdev/warp#6210](https://github.com/warpdotdev/warp/issues/6210)
**Implementation:** [warpdotdev/warp#16312](https://github.com/warpdotdev/warp/pull/16312)
**Figma:** none provided

## Summary

Warp draws kitty graphics images through [Unicode placeholders](https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders). A program gives an image a *virtual placement* (`a=T,U=1` or `a=p,U=1`), then prints the placeholder character U+10EEEE wherever the image should appear, and Warp draws the image in those cells. Today Warp refuses `U=1`, so these images never appear.

## Problem

A placeholder is an ordinary character, so the image it stands for travels with the text. It works through tmux and other multiplexers, from programs that redraw the screen with a text renderer, and it scrolls, wraps and clears with the lines around it. Some programs draw images only this way, such as Claude Code's inline images; others offer it as an option, such as `kitten icat --unicode-placeholder`. In Warp those images show nothing. The program usually can't tell, because it suppresses replies (`q=2`), and the user sees blank cells or missing-glyph boxes.

## Goals

- A program that draws images through placeholders shows them in Warp as it does in kitty.
- With kitty images on, a placeholder cell never draws as a missing-glyph box.
- Re-sending frames under one image id plays as an animation, at 60 frames a second.

## Non-goals

- Placement ids carried by the underline color: Warp cells store no underline color (see 5).
- Kitty's animation commands (`a=f`, `a=a`, `a=c`), tracked in #13737.
- Images sent as a file or shared memory (`t=f`, `t=t`, `t=s`), which fail today for every kind of placement. The implementation PR fixes them in separate commits, because the programs that animate through placeholders send frames that way.
- Windows, where kitty images stay off.

## Behavior

### Virtual placements

1. A kitty command with `U=1` and action `a=T` (transmit and display) or `a=p` (display a stored image) gives its image a virtual placement instead of failing. Warp replies `OK`, unless `q` suppresses the reply, as it does for other placements.
2. A virtual placement draws nothing by itself and never moves the cursor, whatever `C` says.
3. The placement's box is `c` columns by `r` rows of cells when both are given. When only one is given, the other follows from the image's aspect ratio. When neither is given, the box is the image's size in pixels, counted in cells and rounded up. Unlike a direct placement, the box isn't limited to the space right of and below the cursor, since nothing is drawn there.
4. `c=0` or `r=0` creates no placement, as for a direct placement. An `a=p,U=1` naming an image Warp doesn't have fails with the same error reply as a direct placement would.
5. An image has at most one virtual placement. A later `U=1` command for the same image replaces the earlier placement, box size included. Placeholder cells can't name a placement (see Non-goals), so every cell for an image shows its newest virtual placement.

### Placeholder cells

6. A cell whose character is U+10EEEE is a placeholder cell. Its foreground color names the image: a 256-color index gives the id directly, and a 24-bit color gives the id's low 24 bits. Up to three combining marks from kitty's [row/column diacritics](https://sw.kovidgoyal.net/kitty/_downloads/f0a0de9ec8d9ff4456206db8e0814937/rowcolumn-diacritics.txt) give, in order, the row of the cell within the box, its column, and the id's most significant byte.
7. A cell can leave out trailing marks, inheriting them from the placeholder cell immediately to its left, as kitty specifies:
   - No marks, and the left cell has the same foreground color: the same row, the next column, and the same id byte.
   - A row only, and the left cell has the same foreground color and row: the next column and the same id byte.
   - A row and a column only, and the left cell has the same foreground color and row, and the column just before: the same id byte.
   - Anything left out and not inherited is 0.
8. A placeholder cell shows one cell's worth of its image's virtual placement. The image is fitted into the box, keeping its aspect ratio, and centered in it; the cell shows the part of the box at its row and column. Printing every cell of the box in order, row after row, shows the whole image; printing some of them shows that part of it.
9. Each cell shows its own part, whatever surrounds it. The same part may be printed many times, anywhere in the grid, next to other images or text on the same line, and each copy shows it.
10. Where the fitted image doesn't cover the box, and where the image is transparent, the cell shows its background color.
11. A placeholder cell never draws a glyph. A cell whose image or virtual placement doesn't exist, whose foreground is the default or a named color, whose id is 0, or whose row or column lies outside the box shows only its background.
12. Text attributes other than the two colors (bold, underline, strikethrough, and so on) don't change how a placeholder cell draws. Kitty reserves them for future use.
13. Placeholder cells draw the same with font ligatures on and off.

### Over time

14. When the program sends the image again under the same id, every cell showing it draws the new image in Warp's next frame, without the program printing the cells again. Sending 60 images a second plays as a 60 fps animation.
15. Deleting the image (`a=d` with `d=i` or `d=I`), or its virtual placement by placement id, blanks its placeholder cells. They draw again once the image has a virtual placement again.
16. Placeholder cells are text in the grid. They scroll with the output, stay in scrollback, move when lines are inserted or deleted, and are erased by clearing the line or screen, like any other character.
17. Changing the font size or the window size redraws the image at the new cell size. The box keeps its number of rows and columns.
18. A virtual placement belongs to the grid that received the command: a block's output, or the alt screen. Placeholder cells in another block, or on the other screen, show only their background, as Warp's direct kitty placements do.

### Unchanged

19. With kitty images off (the `KittyImages` feature flag, as on Windows), Warp ignores kitty commands as it does today, and U+10EEEE cells draw as ordinary characters.
20. Direct kitty placements (without `U=1`) and iTerm images draw as before.
21. Selection, copy and find treat placeholder cells as the characters they contain.
    - **Open question:** the selection highlight is drawn beneath the grid, so an image's opaque pixels hide it. Should selected placeholder cells show the highlight over the image instead?
