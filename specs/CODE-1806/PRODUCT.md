# PRODUCT: Text selection in the TUI transcript view

Linear: CODE-1806

## Summary

Users can drag with the mouse over the Warp TUI's transcript view to select the text they see — across agent responses, tool-call rows, and terminal command output alike — and the selection is copied to the system clipboard on mouse release. Today the TUI captures the mouse globally, so neither native terminal selection nor any in-app selection works over the transcript.

## Figma

Figma: none provided. Selection highlight uses the terminal-native treatment (reverse video), matching the TUI input editor's existing selection rendering; there is no bespoke visual design.

## Behavior

### Starting and extending a selection

1. Pressing the left mouse button on the transcript and dragging selects the characters the pointer passes over, exactly as rendered on screen. Selection is linear (terminal-style, row-major): the first row selects from the press column to the row's end, intermediate rows select in full, and the final row selects from its start to the pointer column. Dragging upward/leftward of the press point works symmetrically.

2. Selected cells render in reverse video (foreground/background swapped), consistent with the input editor's selection highlight. The highlight updates live during the drag.

3. A double-click selects the word under the pointer, using the same word-boundary rules as double-click in the Warp GUI terminal (including smart-select patterns like URLs and file paths, honoring the user's smart-select settings). Dragging after a double-click extends the selection by whole words.

4. A triple-click selects the full visual row under the pointer. Dragging after a triple-click extends the selection by whole rows.

5. Continuing to drag beyond the transcript's top or bottom edge auto-scrolls the transcript in that direction and extends the selection. Dragging left/right of the transcript clamps to the first/last column of the row under the pointer.

6. A selection can span more rows than fit on screen: content that scrolls out of view during the drag remains part of the selection and is included in the copy.

7. A click that never moves (press and release on the same cell, single click) selects nothing and does not modify the clipboard.

8. The very first click that gives the TUI's host terminal window focus does not start a selection (matching the input editor).

9. After the mouse is released, the selection highlight remains visible (until cleared per invariants 13–16). Auto-follow, mouse-wheel scrolling, and drag-to-select autoscrolling preserve the absolute selection anchors, even if selected rows move or leave the visible viewport. The highlight disappears while selected rows are off-screen and reappears when they return.

### Copying

10. Releasing the mouse button at the end of a non-empty selection gesture — a drag, double-click, or triple-click — immediately copies the selected text to the system clipboard (copy-on-select). No separate copy action is required. The footer's transient-hint slot shows `copied to clipboard` in the success color, then returns to its prior persistent content after the normal transient-hint duration. The ctrl-c exit hint retains higher display priority.

11. Copied text reproduces what is visually selected: one line per selected row, with each row's trailing whitespace removed, rows joined with newlines. Blank rows inside the selection (e.g. spacing between blocks) appear as empty lines. Wide (CJK, emoji) glyphs are copied as their characters, not per-cell fragments.

12. Copy is sent through OSC 52 to the user's terminal, with tmux passthrough when needed, so supported terminals apply the write on the user's local machine even when the TUI runs over SSH. Terminals that disable OSC 52 may ignore the write, and the TUI cannot confirm whether it was accepted.

### Clearing and invalidation

13. At most one selection exists across the TUI at a time. Starting a transcript selection clears any input-editor selection; starting an input-editor selection (mouse or keyboard) clears any transcript selection.

14. Once a drag has started in one surface (transcript or input editor), that surface owns the drag until mouse-up: moving the pointer over the other surface mid-drag neither transfers the drag nor interacts with that surface.

15. Typing does not clear the transcript selection highlight — matching the Warp GUI's agent-input behavior, where an existing selection persists while the user types. The highlight clears only by starting another selection (invariant 13) or via invalidation (invariant 16).

16. Transcript updates preserve selection whenever the originally selected glyphs still exist:
    - Appending agent or terminal output below selected cells does not clear the selection, including while the mouse button is held.
    - When a block above the selection changes height, selected row coordinates are shifted by the same delta.
    - When a selected block grows below the selected range, its existing selection coordinates remain unchanged; newly appended rows are not implicitly added to the selection.
    - When selected cells are still visible after rerender, their glyphs are compared with the previously selected glyphs. The selection clears only if those glyphs changed or disappeared.
    - Shrinking/removing/collapsing content clears only when it removes selected rows; selections below surviving content are rebased.
    - Horizontal resize still clears because it can re-wrap the entire transcript.

### Non-interference

17. Existing transcript interactions are unchanged: scroll wheel scrolling, hover affordances, and clicks on interactive elements that already consume the press behave as before. Selection only engages when the press lands on non-interactive transcript content.

18. The input editor's existing selection behavior (mouse drag, shift+arrows, select-all, its own reverse-video highlight) is unchanged, apart from the mutual-exclusion rule in invariant 13.

19. Host-terminal-level escape hatches are unaffected: terminals that allow bypassing mouse capture with a modifier (e.g. shift+drag) continue to do raw screen selection through the terminal itself.

## Non-goals

- Rectangular/column selection, select-all-transcript, an explicit copy keybinding, a right-click context menu, and settings-gating of copy-on-select are all out of scope for this iteration.
- Structure-aware copy (e.g. "copy this tool call as markdown", excluding decorative chrome semantically) is out of scope; copy is faithful to the rendered screen.
- Selection does not survive a horizontal resize (invariant 16); resize-stable selection would require content-anchored (logical) selection, deferred.
