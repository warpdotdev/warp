# Smooth Scrolling for Discrete Mouse-Wheel Input

Linear: [CSAT-6046](https://linear.app/warpdotdev/issue/CSAT-6046/gh06169-smooth-scrolling-feature)

GitHub: [#6169](https://github.com/warpdotdev/warp/issues/6169)

## Summary
Warp will animate scroll movement from discrete mouse-wheel notches. This change makes clicky
wheels and trackballs feel continuous without changing the final scroll distance. The work ships
in two phases: general WarpUI scrollables first, then terminal scrollback.

## Amendment: Chrome-style easing and duration (post-Phase-1-implementation)
Hands-on use of the Phase 1 implementation surfaced two pieces of feedback from the requester:
a hover-highlight bug during the animation (fixed as an implementation defect; it is not a
behavior change to this spec), and that the originally approved 120ms flat cubic ease-out did
not feel smooth enough. Given a choice between tuning the existing curve, a velocity-preserving
retarget, and a full mass-spring-damper model, the requester chose to mirror Chromium's actual
wheel-scroll animation (`cc::ScrollOffsetAnimationCurve`). This amendment supersedes every
reference to "120 milliseconds" and "cubic ease-out" below with the model described here.
Everything else this spec approved is unchanged.

- **Easing**: cubic bezier ease-in-out (control points `(0.42, 0)` / `(0.58, 1)`, i.e. the same
  curve as CSS's `ease-in-out` keyword), replacing the ease-out-only cubic. A notch's motion now
  eases in before decelerating into the target, rather than launching at peak velocity.
- **Duration**: inversely proportional to the wheel delta rather than a flat value. A small
  notch (at or below 120px, e.g. one default-multiplier line) gets ~200ms; a large one (at or
  above 480px) gets ~100ms; duration ramps between those points for deltas in between. This
  mirrors Chromium's `DurationBehavior::kInverseDelta`, which the Tech Spec's amendment section
  documents is intentional: fast wheel flings should feel snappy, while a single small notch
  should read as smoother and less abrupt than uniform 120ms allowed.
- **Composition**: a same-direction notch arriving mid-flight now retargets the single running
  animation -- reshaping its curve so its starting velocity matches the outgoing animation's
  velocity at that instant -- rather than adding an independent, separately-eased contribution
  on top of it. This keeps velocity continuous across the retarget, which hands-on feedback and
  engineering investigation both identified as a source of "uneven" motion in the original
  model during continuous scrolling.

## Problem
Warp currently applies each discrete wheel notch immediately. A notch therefore moves a generic
scrollable by a fixed pixel distance and moves terminal scrollback by a fixed line distance in one
frame. This behavior is working as designed, but it creates visible jumps for users whose mouse or
trackball emits line-based wheel input.

Trackpads and high-resolution wheels emit precise pixel deltas. Warp already applies those deltas
continuously. They are not the source of this design gap.

## Figma
Figma: none provided. The behavior in this document is the source of truth.

## Goals and non-goals
- Phase 1 covers general GUI scrollables built with the WarpUI `Scrollable`, `NewScrollable`, and
  `ClippedScrollable` paths.
- Phase 2 covers normal terminal block-list scrollback.
- Phase 1 can ship before Phase 2.
- The existing mouse-wheel multiplier continues to control scroll distance.
- Precise wheel and trackpad input keeps its existing behavior.
- Touch momentum keeps its existing behavior.
- This work does not change scroll speed defaults.
- This work does not add overscroll, bounce, or momentum to a clicky wheel.
- This work does not add a permanent user-facing setting.

## Behavior
### Shared behavior
1. Smooth scrolling is on by default when the `SmoothScrolling` feature flag is enabled.
   - The flag is a rollout gate.
   - The flag is not a user preference.
   - When the flag is disabled, all covered surfaces use their current immediate-scroll behavior.

2. Warp smooths only non-precise wheel events.
   - A winit `LineDelta` event is eligible.
   - A winit `PixelDelta` event is not eligible.
   - Warp does not infer input type from device name, platform, or delta size.

3. One eligible wheel notch creates a target scroll position at the same destination Warp uses
   today. Warp animates the displayed position to that target using a cubic bezier ease-in-out
   curve, over a duration that scales inversely with the notch's delta (see the amendment above
   for the exact model and its rationale).
   - The movement eases in, then decelerates into the target.
   - The movement does not overshoot, bounce, or continue after it reaches the target.
   - Hands-on validation evaluates the duration and curve. Any further change must update this
     spec before the implementation merges.

4. The final distance does not change.
   - Warp applies `mouse_scroll_multiplier` to a non-precise delta before it creates the target.
   - General WarpUI scrollables continue to convert each resulting line unit to 40 pixels.
   - Terminal scrollback continues to interpret the resulting value as lines.
   - Animation must not apply the multiplier or line-to-pixel conversion more than once.

5. Repeated notches in the same direction compose into one continuous interaction.
   - Each notch's distance extends the current target.
   - A new notch does not restart, pause, or discontinuously speed up movement that is already
     visible; per the amendment above, the running animation retargets to the new, larger
     destination while preserving its current velocity.
   - The displayed position reaches the accumulated target without losing distance.

6. A notch in the opposite direction reverses immediately.
   - Warp first keeps the currently displayed position.
   - Warp discards any unrendered remainder from the old direction.
   - Warp creates a new target from the new input.
   - The user does not have to wait for the old target to be reached before reversal starts.

7. Direct scroll operations take precedence over animation.
   - Scrollbar thumb dragging and scrollbar track clicks cancel animation.
   - Keyboard scrolling, including page, home, and end actions, cancels animation.
   - Jump-to-bottom, scroll-to-item, find-result navigation, and equivalent programmatic position
     changes cancel animation.
   - Warp keeps the currently displayed position, then applies the direct operation immediately.
   - A completed direct operation does not inherit unrendered distance from the cancelled tween.

8. Precise input takes precedence over animation.
   - When a precise wheel or trackpad event arrives during an animation, Warp cancels the
     animation at its currently displayed position.
   - Warp applies the precise delta immediately through the existing precise-input path.
   - Warp does not ease, coalesce, multiply, or rewrite the precise delta.

9. Scroll bounds do not change.
   - Targets clamp to the existing minimum and maximum positions.
   - Warp does not render beyond a scroll boundary.
   - An inner scrollable at its target boundary preserves the existing decision to propagate an
     otherwise unhandled wheel event to its parent.
   - Animation does not cause both a nested child and its parent to consume the same distance.

10. Horizontal and vertical discrete input use the same behavior.
    - A single-axis scrollable animates its configured axis.
    - A dual-axis scrollable animates each eligible axis independently.
    - Existing cross-axis remapping remains unchanged.
    - Small trackpad drift is not reclassified as discrete dual-axis input.

### Phase 1: general WarpUI scrollables
11. Phase 1 covers GUI surfaces whose wheel movement is owned by the generic WarpUI scrollable
    layer. Examples include settings pages, lists, panels, tables, menus, and modals that use the
    shared scrollable types.

12. Phase 1 covers clipped and manually managed generic scrollables. A surface does not lose
    smooth scrolling because it supplies its own scroll state to a shared scrollable wrapper.

13. Phase 1 excludes terminal-owned vertical scrolling. The terminal block list, alternate screen,
    and wheel events forwarded to a PTY do not become smooth as a side effect of Phase 1.

### Phase 2: terminal scrollback
14. Phase 2 applies the same target animation (per the amendment above: cubic bezier ease-in-out,
    inverse-delta duration, velocity-preserving retarget) to normal terminal block-list
    scrollback.
    - Intermediate positions use the terminal's existing fractional-line support.
    - The final position is the same position produced by the current non-precise wheel path.
    - Repeated input, reversal, precise-input interruption, direct jumps, and boundaries follow
      the shared behaviors above.

15. A terminal scroll animation cancels when another terminal operation changes scroll position.
    This includes keyboard navigation, jump-to-bottom behavior, find navigation, block navigation,
    and autoscroll that follows new output.

16. Phase 2 never smooths or rewrites wheel input that belongs to the PTY.
    - Alternate-screen scrolling remains immediate and keeps its existing mouse-reporting or
      arrow-key behavior.
    - Mouse-reporting applications such as Vim and `less` receive the existing wheel events.
    - The long-running-block `AltMouseAction` forwarding path receives the existing wheel events.
    - Warp does not convert one PTY-bound notch into multiple fractional or precise events.

17. A shared-session alternate screen keeps its current scrolling behavior in Phase 2. Terminal
    smooth scrolling is limited to normal block-list scrollback.

### Rollout and follow-up
18. Phase 1 and Phase 2 must behave consistently on macOS, Linux, and Windows for equivalent winit
    line-delta input.

19. A later product decision may add a user-facing preference and reduced-motion integration.
    Neither is part of Phase 1 or Phase 2. Until then, disabling the rollout flag is the only
    opt-out mechanism.

## Assumptions
- The Chrome-style bezier ease-in-out, inverse-delta duration, and velocity-preserving retarget
  described in the amendment above are the current tuning target, chosen by the requester after
  hands-on feedback that the original flat 120ms cubic ease-out was not smooth enough.
- No external mock, prototype, or recording defines a different animation curve.
- The standard Warp compile-time and runtime feature-flag plumbing is sufficient. A remote kill
  switch is not required.

## Out of scope
- Smoothing trackpad or other precise pixel input.
- Changing touch momentum.
- Changing `mouse_scroll_multiplier`, its default, its range, or its settings UI.
- Adding a smooth-scrolling settings row or Command Palette action.
- Adding operating-system reduced-motion plumbing.
- Adding elastic overscroll, inertial continuation, or scroll acceleration.
- Changing PTY mouse-reporting protocols or alternate-screen behavior.
