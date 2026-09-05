# Smooth Scrolling for Discrete Mouse-Wheel Input — Technical Design

See [`PRODUCT.md`](./PRODUCT.md) for user-visible behavior.

Implementation snapshots:
- [Phase 1, generic scrollables at `c0d11956`](https://github.com/warpdotdev/warp/tree/c0d11956d76695299e569ec0fe153c516efdc90d)
  ([#15053](https://github.com/warpdotdev/warp/pull/15053))
- [Phase 2, terminal scrollback at `2104bbc6`](https://github.com/warpdotdev/warp/tree/2104bbc6826ef19cb656ac2a1f65fc380c291c32)
  ([#15206](https://github.com/warpdotdev/warp/pull/15206))

## Context
Winit preserves the input device's granularity:
- [`LineDelta` becomes `precise: false`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui/src/windowing/winit/event_loop/mod.rs#L1254-L1268).
- [`PixelDelta` becomes `precise: true`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui/src/windowing/winit/event_loop/mod.rs#L1254-L1268).
- [`apply_scroll_multiplier`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/app/src/lib.rs#L729-L735)
  multiplies only non-precise input before consumers receive it.

The shared scroll consumers convert non-precise line units to pixels. Terminal scrollback converts
precise pixels to fractional lines and keeps non-precise input in lines. Smooth scrolling preserves
those boundaries instead of rewriting wheel events globally.

## Animation controller
[`SmoothScrollController`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui_core/src/smooth_scroll.rs#L338-L499)
is a deterministic one-axis controller. Every time-dependent method receives an `Instant`.

The controller stores:
- A committed position.
- One optional segment with its start time, start position, start velocity, target, and duration.
- The position returned by the last incremental read.

Consumers use one controller per axis. Clipped scrollables read its absolute displayed position.
Manual scrollables and terminal scrollback call `take_increment` and apply only movement that has
not already been emitted. `cancel` and `set_position_immediately` clear the segment and reset the
incremental baseline.

### Motion model
A fresh segment uses the cubic bezier `(0.42, 0, 0.58, 1)`. The implementation solves the bezier's
time component before sampling progress and velocity.

Let `d` be the absolute delta in pixel-equivalent units. The base duration is:
- 200 milliseconds when `d <= 120`.
- 100 milliseconds when `d >= 480`.
- A linear interpolation between 200 and 100 milliseconds otherwise.

This is the inverse-delta ramp from Chromium's
[`cc/animation/scroll_offset_animation_curve.cc`](https://chromium.googlesource.com/chromium/src/+/b815e97cab28c0ea7276d9e78a06f75f5610fc8f/cc/animation/scroll_offset_animation_curve.cc#28).
Small movements remain visible and smooth. Large bursts finish sooner instead of accumulating lag.

For same-direction input, `add_delta` samples the current position and velocity, extends the target,
and starts one replacement segment. The new curve encodes the outgoing normalized slope in its
first control point. Its duration is capped by Chromium's `VelocityBasedDurationBound`:

`abs(remaining_delta / current_velocity) * 2.5`

The bound is zero at the target and unbounded when velocity is zero or points away from the target.
Warp then applies a 16-millisecond minimum retarget duration. This floor, scalar sign checks, and a
`y1` clamp to `[0, 1]` are Warp-specific safeguards; the linear ramp and velocity bound formulas
come from the cited Chromium source.

For opposite-direction input, `add_delta` commits the sampled displayed position, discards the old
remainder, and starts a zero-velocity segment toward the new target.

The controller uses a target tween instead of velocity decay because a target preserves the exact
existing scroll distance. Velocity decay would add inertial travel and make the destination depend
on frame timing.

## Input eligibility and rollout
[`should_animate_scroll`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui_core/src/smooth_scroll.rs#L60-L66)
returns true only for non-precise input while `FeatureFlag::SmoothScrolling` is enabled. Eligibility
is checked by the consumer; no new global wheel-event type or frame event exists.

A single `FeatureFlag::SmoothScrolling` gates both phases:
- It appears in `RUNTIME_FEATURE_FLAGS` for local and development control.
- It is not a separate Phase 1/Phase 2 flag, a remote kill switch, or a user setting.

The gate is read for each input event. Flag-off input uses the immediate path. Toggling the flag
does not walk every scrollable to cancel existing controllers.

## Phase 1: generic WarpUI scrollables
Phase 1 integrates the controller into persistent shared scroll state:
- [`ClippedScrollStateHandle`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui_core/src/elements/gui/clipped_scrollable.rs#L40-L124)
  owns an absolute controller position.
- [`ScrollState`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui_core/src/elements/gui/scrollable.rs#L28-L63)
  owns relative state for legacy and manually managed children.
- [`NewScrollable`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui_core/src/elements/gui/new_scrollable/mod.rs#L721-L897)
  keeps current axis projection, 40-pixel conversion, hit testing, and nested propagation.

For eligible input, a consumer converts the accepted delta once and adds it to its controller.
Bounds checks use the controller target rather than its displayed position. Precise input and
integrated immediate operations cancel before applying their movement. Shared clipped setters use
`set_position_immediately`; manual and legacy scrollbar operations explicitly cancel.

Generic scrollables schedule frames with `PaintContext::repaint_after` at
[`SMOOTH_SCROLL_FRAME_INTERVAL = 8 ms`](https://github.com/warpdotdev/warp/blob/c0d11956d76695299e569ec0fe153c516efdc90d/crates/warpui_core/src/smooth_scroll.rs#L37-L45):
- Clipped axes read the absolute animated position during paint.
- Manual and legacy axes apply `take_increment` through the existing child scroll API when the
  repaint cycle dispatches its synthetic mouse-move event.
- Repaint scheduling stops when the controller settles.

This design reuses existing redraw and child-notification paths. It does not add routed frame
events, controller IDs, generations, or permanently live wrappers.

## Phase 2: terminal scrollback
Terminal input remains routed before animation:
1. [`BlockListElement::scroll_internal`](https://github.com/warpdotdev/warp/blob/2104bbc6826ef19cb656ac2a1f65fc380c291c32/app/src/terminal/block_list_element.rs#L1331-L1397)
   converts precise pixels to fractional lines and keeps discrete input in lines.
2. A long-running block that owns the wheel emits one `AltMouseAction` and returns.
3. Only normal block-list scrolling reaches `TerminalView::scroll`.

This consumer-local decision is required. Global wheel rewriting could split or alter input that a
PTY expects to receive once.

[`SmoothScrollHandle`](https://github.com/warpdotdev/warp/blob/2104bbc6826ef19cb656ac2a1f65fc380c291c32/app/src/terminal/block_list_viewport.rs#L35-L118)
wraps the shared controller in `Arc<Mutex<_>>`. It converts line deltas to 40-pixel-equivalent
controller units for consistent timing, then converts emitted increments back to lines.
`ScrollPosition` remains the only absolute terminal position.

[`TerminalView::scroll`](https://github.com/warpdotdev/warp/blob/2104bbc6826ef19cb656ac2a1f65fc380c291c32/app/src/terminal/view.rs#L9681-L9707)
starts at most one driver for an active animation. The driver is one long-lived
`futures::stream::unfold` that waits eight milliseconds per tick with `Timer::after`.
[`advance_smooth_scroll`](https://github.com/warpdotdev/warp/blob/2104bbc6826ef19cb656ac2a1f65fc380c291c32/app/src/terminal/view.rs#L9710-L9758)
applies each increment through `ScrollPositionUpdate::AfterScrollEvent`.

The terminal driver is independent of paint, pointer events, and cached mouse position. Each tick
reuses existing clamping and sticky-bottom logic against the current block list, so content growth
does not require a captured absolute target.

### Terminal cancellation
[`TerminalView::update_scroll_position_locking`](https://github.com/warpdotdev/warp/blob/2104bbc6826ef19cb656ac2a1f65fc380c291c32/app/src/terminal/view.rs#L9273-L9306)
is the common update path. It cancels an active animation for every update except
`AfterScrollEvent`, which the animation itself uses.

Precise and flag-off input cancels explicitly before its immediate `AfterScrollEvent`. Entering the
alternate screen also cancels. Cancellation causes the timer stream to end on its next check and
allows a later animation to start a new stream.

`TerminalAction::Scroll { precise: true }` also marks non-wheel direct sources that must apply
immediately, such as scrollbar and keyboard single-line operations. At the winit boundary,
`precise` still reflects line-versus-pixel input.

## Testing and validation
Automated coverage must preserve these guarantees:
- Controller tests pin exact targets, no overshoot, the linear duration ramp, bezier reference
  values, velocity-preserving retargets, reversal, cancellation, and incremental emission.
- Generic scrollable tests pin precision and flag branching, multiplier/conversion behavior,
  direct cancellation, target-based boundary propagation, independent axes, clamping, hover
  updates, and the real repaint chain.
- Terminal tests pin deferred animation, exact landing, direct cancellation, sticky-bottom state,
  independent timer driving without a cached pointer, content growth, and alternate-screen
  cancellation.

Run the focused suites:

```sh
cargo nextest run -p warpui_core smooth_scroll
cargo nextest run -p warpui_core scrollable
cargo nextest run -p warp smooth_scroll
```

Before merge, run:

```sh
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
./script/presubmit
```

Visual proof must be a video because screenshots cannot show timing or interruption:
- Phase 1: use a clicky wheel or injected line-delta input on long, nested, and dual-axis generic
  surfaces. Show one notch, a rapid burst, reversal, precise-input interruption, scrollbar
  interaction, boundary propagation, and flag-off behavior.
- Phase 2: repeat on long normal terminal scrollback. Include content arriving during animation.
- Confirm unchanged behavior in Vim, `less`, a long-running PTY-owned block, alternate screen, and
  shared-session alternate screen.
- Verify equivalent line/pixel behavior on one current macOS, Linux, and Windows runner. Prioritize
  Windows because #6169 was reported there.

## Risks and mitigations
- **Frame cost:** request frames only while a controller or terminal driver is active. Time-based
  progress prevents missed frames from changing the destination.
- **Nested scrolling:** use targets for boundary acceptance so a lagging child does not consume
  input that belongs to its parent.
- **PTY regressions:** decide PTY ownership before animation and never synthesize PTY frame input.
- **Direct navigation:** cancel through shared immediate setters and the terminal update choke
  point so pending increments cannot move the view later.
- **Platform differences:** verify real line and pixel devices on each desktop operating system.

## Delivery
- Phase 1 owns the shared controller, flag wiring, and generic scrollables in
  [#15053](https://github.com/warpdotdev/warp/pull/15053), based on this spec branch.
- Phase 2 adds terminal integration in [#15206](https://github.com/warpdotdev/warp/pull/15206),
  based on Phase 1.
- The phases remain separate because terminal input routing and frame driving are independent of
  generic scrollable delivery. Cross-platform verification can run in parallel after each phase's
  focused tests pass.

## Follow-ups
- Decide whether to add a user-facing preference.
- Add reduced-motion integration when Warp has a shared policy.
- Remove the rollout flag and immediate fallback after the feature is stable.
