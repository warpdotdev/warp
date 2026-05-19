# Product Spec: Horizontal scrolling per terminal pane (GH-9828)

> **Spec convention.** This spec follows the repository convention
> `specs/GH<issue>/product.md` (product requirements) and
> `specs/GH<issue>/tech.md` (technical contract). It is **not**
> a single `SPEC.md` file. Downstream spec-discovery and
> implementation tooling that walks `specs/GH*/{product,tech}.md`
> will find this spec correctly.

## Problem

Wide diff hunks, long log lines, and `tree` output regularly
exceed the visible width of a narrow pane. Today, lines wrap (or
truncate, depending on settings), preventing a quick lateral
glance at content beyond the viewport. Users want trackpad-swipe
horizontal scroll to peek at long lines without resizing the pane
or switching focus.

## Goal

Add per-pane horizontal scrolling for the terminal output grid
when the longest line in the visible buffer range exceeds pane
width AND the user has line-wrap disabled. Trackpad horizontal
deltas, Shift-wheel, and a horizontal scrollbar all drive it.

## Acceptance criteria

- A1. With line-wrap off and a long line visible, a two-finger
  horizontal swipe scrolls the line; the rest of the pane scrolls
  in lockstep so columns stay aligned.
- A2. Selection across a horizontally-scrolled region copies the
  underlying logical text, not the visible-only fragment.
- A3. With line-wrap on (today's default), no horizontal scroll
  affordance appears.
- A4. While pinned to the live bottom, a completed new shell output
  row snaps back to offset 0 when
  `terminal.horizontal_scroll.snap_on_new_output = true`.
- A5. Horizontal scrollbar appears only when content exceeds pane
  width.
- A6. With `terminal.horizontal_scroll.snap_on_new_output = false`,
  completed new shell output preserves the current horizontal offset.
- A7. Resizing the pane or vertically scrolling to shorter
  visible content **clamps** `h_offset` according to B1a and the
  Offset clamping & reset rules section in `tech.md`, with no
  blank shifted columns. Toggling line-wrap follows the
  preserve-and-restore invariant (A_WRAP_TOGGLE), not a reset.
- A_WRAP_TOGGLE_PRESERVE. Turning line-wrap ON while
  `h_offset > 0` preserves the offset internally: the
  scrollbar disappears, content rewraps inside the viewport,
  and the preserved offset is held in pane state.
- A_WRAP_TOGGLE_RESTORE. Turning line-wrap OFF restores the
  preserved `h_offset`, clamped to the current
  `max_observed_columns`. If `max_observed_columns` has not
  shrunk below the preserved value's required range, the
  offset is restored exactly.
- A_WRAP_TOGGLE_NO_RESET. Toggling line-wrap is NEVER a reset
  trigger. `h_offset` is reset to 0 only when (1) the user
  explicitly scrolls to the leftmost column, (2) snap-on-new-
  output fires per B6, or (3) the pane closes. The boundary
  clamp from B1a (range collapses to `{0}`) is a clamp, not a
  reset trigger.
- A8. In alternate-screen / TUI mode (vim, htop, less), no snap
  occurs regardless of the setting; the alternate screen owns its
  own column layout and h_offset is preserved as-is for the
  duration of the alt-screen session.
- A9. Wide glyphs (CJK, emoji), combining marks, and tabs scroll
  consistently — selection at the visible-leftmost cell of an
  h-scrolled row resolves to the same logical column as the
  rendered cell, including correct handling of half-clipped wide
  glyphs at the left edge.
- A10. On Linux with libinput two-finger horizontal swipe events
  available, the gesture scrolls horizontally; on Linux platforms
  without horizontal-axis events, Shift+wheel and scrollbar drag
  are the documented inputs and tests cover those paths.

## Out of scope

- Horizontal scroll inside notebook cells (different code path,
  follow-up).
- Auto-detect long lines and offer a toast suggesting line-wrap
  toggle.
- Horizontal scrollbar styling customization.
