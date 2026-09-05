# Smooth Scrolling for Discrete Mouse-Wheel Input

Linear: [CSAT-6046](https://linear.app/warpdotdev/issue/CSAT-6046/gh06169-smooth-scrolling-feature)

GitHub: [#6169](https://github.com/warpdotdev/warp/issues/6169)

## Summary
Warp animates discrete mouse-wheel notches so clicky wheels and trackballs move smoothly without
changing the final scroll distance. The feature covers general WarpUI scrollables and normal
terminal scrollback as two separately deliverable phases.

## Figma
Figma: none provided. This document defines the interaction.

## Behavior
1. Warp smooths line-based, non-precise wheel input when the `SmoothScrolling` feature flag is
   enabled.
   - The flag is enabled by default and acts as a rollout gate, not a user preference.
   - Pixel-based trackpad and high-resolution wheel input remains immediate.
   - When the flag is disabled, new wheel input follows the immediate path. Changing the flag does
     not itself guarantee cancellation of an animation already in flight.

2. Animation preserves the existing destination.
   - Warp applies `mouse_scroll_multiplier` once before animation.
   - General WarpUI scrollables continue to convert each resulting line unit to 40 pixels.
   - Terminal scrollback continues to apply the resulting value as lines.
   - The target is clamped to the existing scroll bounds. Motion never overshoots or bounces.

3. Each axis animates toward an exact target with a cubic bezier ease-in-out curve.
   - A delta of 120 pixel-equivalent units or less takes 200 milliseconds.
   - A delta of 480 pixel-equivalent units or more takes 100 milliseconds.
   - Duration decreases linearly between those limits. Large bursts finish sooner so rapid wheel
     input does not feel delayed.

4. Repeated input forms one continuous interaction.
   - Same-direction input extends the current target and preserves the animation's current
     position and velocity.
   - Opposite-direction input reverses immediately from the displayed position. Warp discards the
     unrendered remainder in the old direction and starts the new motion from rest.
   - The final position equals the sum of accepted same-direction deltas, subject to scroll bounds.

5. Immediate operations take precedence over animation.
   - Precise wheel or trackpad input cancels the active animation and applies immediately.
   - Scrollbar interaction and immediate shared-scroll-state setters cancel before moving.
   - Terminal keyboard navigation, find and block navigation, jump-to-bottom, filter changes,
     resize correction, clear, and autoscroll cancel terminal animation through the terminal's
     common scroll-position update path.
   - Cancellation keeps the displayed position and discards pending animated movement.

6. Existing axis and nesting rules remain in force.
   - Single-axis scrollables animate their configured axis.
   - Dual-axis scrollables animate each eligible axis independently.
   - Existing cross-axis remapping is unchanged.
   - Boundary decisions use the animation target, not the lagging displayed position. An inner
     scrollable therefore continues to pass an unhandled wheel event to its parent at a boundary.

7. Phase 1 covers general GUI scrollables built on the shared `Scrollable`, `NewScrollable`, and
   `ClippedScrollable` paths, including clipped and manually managed state.
   - Settings pages, lists, panels, tables, menus, and modals gain the behavior through those
     shared paths.
   - Terminal-owned vertical scrolling remains excluded from Phase 1.

8. Phase 2 covers normal terminal block-list scrollback.
   - The animation applies fractional-line increments through the terminal's existing scroll path.
   - Each increment uses the current content, bounds, and sticky-bottom state. Content growth
     during animation therefore follows the same rules as immediate scrolling.
   - Entering the alternate screen cancels an active normal-scrollback animation at its displayed
     position.

9. Wheel input owned by a terminal application is never animated or expanded into frame events.
   - Alternate-screen scrolling keeps its existing immediate PTY behavior.
   - Mouse-reporting applications such as Vim and `less` receive the existing wheel actions.
   - Long-running-block `AltMouseAction` forwarding emits one unchanged action for each source
     wheel event.
   - Shared-session alternate-screen behavior remains unchanged.

10. Equivalent line-based input follows the same behavior on macOS, Linux, and Windows.

## Out of scope
- Smoothing precise pixel input or changing touch momentum.
- Changing the scroll multiplier, its default, its range, or its settings UI.
- Adding a permanent smooth-scrolling preference or Command Palette action.
- Adding operating-system reduced-motion integration.
- Adding elastic overscroll, inertial continuation, or scroll acceleration.
- Changing PTY mouse protocols, alternate-screen behavior, or long-running-block forwarding.
