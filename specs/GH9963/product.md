# Product Spec: Shift+Click extends terminal text selection

**Issue:** [warpdotdev/warp#9963](https://github.com/warpdotdev/warp/issues/9963)

## Summary

Make Shift+Click extend an existing terminal text selection from its original anchor to the new click point, matching the behavior every other terminal emulator (and most text editors) offer. Today, Shift+Click in Warp clears or restarts the selection, which forces users to redo the original selection from scratch when they want to grow it — particularly painful on laptop touchpads where scrolling-while-selecting is unreliable.

## Problem

Per the issue: Warp supports click-drag, smart selection (double-click), and rectangular selection in terminal output, but Shift+Click *replaces* the selection rather than *extending* it. The reporter highlights that this is "especially frustrating on laptop touchpads because it's hard to scroll while actively selecting" — the workaround is to zoom out, select, then zoom back in, which is many steps for an action that should be one click.

## Goals

- Holding Shift while clicking inside terminal output extends the existing selection: the original anchor (the point where the user originally pressed down to start the selection) is preserved, the head moves to the click point.
- The behavior matches the convention used by macOS Terminal, iTerm2, GNOME Terminal, and other reference terminals — both for forward extension (click later than anchor) and backward extension (click before anchor).
- Shift+Click without an existing selection starts a new one (degenerate case: the click point becomes both anchor and head).
- The change is scoped to the terminal output surface. Selection in the editor, settings, and other surfaces is unaffected.

## Non-goals (V1 — explicitly deferred to follow-ups)

- **Shift+Click during an active drag.** V1 handles the discrete click case; a drag started with Shift held continues to extend until release, but the spec does not formalise drag-extension behavior. Existing drag-select continues to work as today.
- **Shift+Drag to extend smart-selection units.** Double-click selects a word; the desired behavior for Shift+Drag after a word-selection (do we extend by-character or by-word?) is a separate UX decision tracked elsewhere.
- **Shift+Click in rectangular-selection mode.** Rectangular selection has its own semantics; this spec does not change them.
- **Shift+Click in the editor / Markdown viewer / settings panes.** Those surfaces have their own selection state machines; bringing them into parity is a separate concern.
- **Keyboard-only selection extension** (Shift+Arrow keys for terminal output). Out of scope; this is a mouse interaction.

## User experience

### Forward extension

1. User click-drags from row 5 col 10 to row 8 col 30. Selection runs from `(5, 10)` to `(8, 30)`.
2. User holds Shift and clicks at row 12 col 5.
3. Selection now runs from `(5, 10)` to `(12, 5)` — original anchor preserved, head moved.

### Backward extension

1. User click-drags from row 8 col 20 to row 12 col 5. Selection runs from `(8, 20)` to `(12, 5)`.
2. User holds Shift and clicks at row 5 col 10.
3. Selection now runs from `(5, 10)` to `(8, 20)` — original anchor preserved, but the click point is now *before* it, so the selection's head/tail roles swap. The text spanning the new selection is what's now selected (the *content* between `(5, 10)` and `(8, 20)`); the anchor concept persists invisibly so a subsequent Shift+Click can extend in either direction relative to the same anchor.

### No prior selection

1. User holds Shift and clicks at row 7 col 15 with no existing selection.
2. Selection is a single point at `(7, 15)`. (Equivalent to a click without Shift.)
3. A subsequent click-drag or Shift+Click extends from there.

### Interaction with other selection modes

- A normal (non-Shift) click clears the selection and sets a new anchor at the click point. Subsequent Shift+Clicks extend from this new anchor.
- A double-click (smart-selection) replaces the selection with the recognized word/URL/path and sets the anchor to that selection's start. Shift+Clicks after a double-click extend from the smart-selected anchor.

## Configuration shape

No new settings. The behavior is inherent to the click handler.

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. Given an active selection from `(5, 10)` to `(8, 30)`, Shift+Click at `(12, 5)` results in a selection from `(5, 10)` to `(12, 5)`.
2. Given an active selection from `(8, 20)` to `(12, 5)`, Shift+Click at `(5, 10)` results in a selection covering the text from `(5, 10)` to `(8, 20)`.
3. Given no active selection, Shift+Click at any point produces a single-point selection at that point (degenerate selection; equivalent to a non-Shift click).
4. A non-Shift click clears any prior selection and sets a fresh anchor; a subsequent Shift+Click extends from that fresh anchor.
5. A double-click (smart-selection) sets the anchor to the smart-selected unit's start; a subsequent Shift+Click extends from that anchor.
6. The behavior is unchanged for selection in the editor, settings, and Markdown viewer surfaces — Shift+Click there continues to behave as today (no regression).
7. The behavior is unchanged for rectangular-selection mode — Shift+Click while in rectangular mode does not interact with this feature.
8. Selection extension via Shift+Click correctly updates downstream consumers of the selection (clipboard "Copy on Select" if enabled, find-in-block highlighting, etc.) — extension behaves identically to a drag that ended at the click point would.

## Open questions

- **Click-then-Shift-Click latency.** If the user click-drags, releases, then immediately Shift+Clicks, is there any debounce or "release confirms" boundary? Recommend none — the click-drag completion already commits the selection, and a Shift+Click after that completion is just an extension. Match macOS Terminal's behavior, which has no such boundary.
- **Visual feedback during the extension.** Does the cursor change during Shift+hover? Recommend matching today's hover affordance for terminal output, with no Shift-specific cursor — the mouse position is already visually clear, and a cursor change adds noise without information. Confirm with maintainers if a different signal is wanted.
- **Anchor persistence across scroll.** If the user click-drags, scrolls the buffer, then Shift+Clicks at a point that's now visible due to the scroll, the anchor is still the original buffer position (not its on-screen position). Worth calling out explicitly so implementers preserve the buffer-coordinate semantics rather than viewport-coordinate semantics.
