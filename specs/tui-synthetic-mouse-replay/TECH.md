# TUI synthetic mouse replay — Tech Spec

Branch: `harry/tui-synthetic-mouse-replay`, stacked on `harry/fix-hoverable-hit-box`
(see [`specs/tui-hoverable-hit-area/TECH.md`](../tui-hoverable-hit-area/TECH.md)).

## Context

TUI hover state only updates when a real `MouseMoved` arrives. When layout shifts
under a stationary pointer — e.g. clicking a collapsed thinking header expands the
block and pushes the header out from under the mouse — the header keeps rendering
as hovered until the mouse physically moves.

The GUI solves this with synthetic mouse moves: it caches the last `MouseMoved`
per window and, after every scene build, redispatches it with `is_synthetic: true`
inside a loop capped at three iterations
([`crates/warpui_core/src/core/app.rs (2894-2961) @ e77426f0`](https://github.com/warpdotdev/warp/blob/e77426f092c84df02a519f43821ac13aa51a7c97/crates/warpui_core/src/core/app.rs#L2894-L2961)).
Its `Hoverable` additionally suppresses two consecutive synthetic hover flips via
`MouseState::last_event_is_synthetic_hover`, breaking layout↔hover feedback loops.
`TuiEvent::MouseMoved` already carries an `is_synthetic` flag, but nothing emitted
synthetic moves in the TUI. This change ports the GUI mechanism.

## Changes

- `crates/warpui_core/src/runtime/mod.rs`:
  - `TuiScreen` records `last_mouse_position` from every positional event in
    `dispatch_event`.
  - `TuiScreen::draw` now mirrors the GUI's `build_scene` loop: up to three
    iterations of take-invalidations → layout/paint → `replay_mouse_position`
    (a synthetic `MouseMoved` at the cached position through the freshly
    rendered tree). A hover flip invalidates its view, so the frame is rebuilt
    within the same call; the first iteration always paints, and the loop breaks
    once the replay stops invalidating.
- `crates/warpui_core/src/elements/tui/hoverable.rs`: on a hover transition,
  `TuiHoverable` applies the GUI's guard — the flip is suppressed when both it
  and the previous flip came from synthetic moves; any non-move event re-arms
  the guard. Uses the shared `MouseState::last_event_is_synthetic_hover`
  (visibility widened to `pub(crate)` in `gui/hoverable.rs`).

Known divergence from the GUI: the replay dispatches with default modifiers,
while the GUI's cached event preserves cmd/shift held at move time. Nothing in
the TUI reads modifiers off `MouseMoved` today; cache them alongside the
position if that changes.

## Testing and validation

- `crates/warpui_core/src/runtime/mod_tests.rs`:
  `synthetic_mouse_move_after_redraw_updates_hover` — a real move hovers a
  target, a keypress shifts it down a row, and the post-draw replay unhovers it
  with no further mouse input.
- `crates/warpui_core/src/elements/tui/hoverable_tests.rs`:
  `consecutive_synthetic_hover_flips_are_suppressed` — a synthetic flip in is
  allowed, an immediately following synthetic flip out is suppressed, and a real
  move flips normally.
- Run with `cargo nextest run -p warpui_core --features tui -E 'test(tui::hoverable) or test(runtime)'`
  (the TUI module is gated behind the `tui` feature).
