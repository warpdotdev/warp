# Spec: Interactive TUI zero-state logo

## Product

### Summary
Make the spinning object in the Warp Agent CLI zero state respond to left-mouse dragging. A horizontal drag directly scrubs the object in either direction with deliberately low sensitivity; releasing it produces a same-direction momentum flick whose strength scales with total mouse-down-to-mouse-up distance, then smoothly settles back to the configured forward idle velocity. The interaction is an undisclosed easter egg: the screen gains no hint, hover treatment, setting, or unsupported-terminal warning.

### Key design choices
- Reuse the TUI's existing session-wide mouse capture and `LeftMouseDown` / `LeftMouseDragged` / `LeftMouseUp` event path; do not change terminal mouse mode.
- Store interaction state outside the rendered element so view rebuilds and terminal resizes preserve the current angle, active drag, and momentum.
- Support signed horizontal control. Rightward motion scrubs and flicks forward; leftward motion scrubs and flicks backward. Vertical-only movement does not drive rotation, and user-driven reverse motion still settles back to the configured forward idle direction.
- Apply a high but hard velocity cap of 1.5 revolutions per second and ease a released velocity back to the configured idle velocity over 3 seconds.

### Behavior
1. With no mouse interaction, the active zero-state object and background starfield render and animate exactly as they do today, including the configured rotation period and custom ASCII object support.
2. A left-mouse press starts an interaction only when it lands inside the tight bounding rectangle of the currently rendered non-background object cells. Background stars and the rest of the zero-state panel are not drag targets. Once a valid press starts a drag, subsequent drag and release events remain captured even if the pointer leaves the object's bounds.
3. Pressing the object never changes its angle. A click with no horizontal cell movement has no effect on phase or velocity.
4. After a valid press, signed horizontal displacement directly scrubs the object at `1/96` revolution per terminal column, subject to the 1.5 revolutions-per-second held-motion cap measured from monotonic press time. Rightward displacement rotates forward and leftward displacement rotates backward. Vertical-only displacement leaves the ordinary idle solution untouched. The background starfield is not scrubbed.
5. The interaction becomes a drag after the first nonzero horizontal cell displacement. While held, the object tracks the signed mouse-down-to-current-position displacement without input-event latency beyond the next scheduled repaint. Short gestures therefore produce modest, readable turns instead of complete revolutions.
6. On release, velocity is proportional to the signed horizontal distance between mouse down and mouse up across the whole gesture. The angle represented by that distance is mapped through a 500 ms response interval, then clamped to the inclusive range from -1.5 through 1.5 revolutions per second. Pausing before release does not erase the flick; returning to the original horizontal position yields zero release velocity.
7. From release, total angular velocity transitions from the signed clamped release velocity to the configured forward idle angular velocity over exactly 3 seconds using cubic smoothstep interpolation. Angle remains continuous at release and throughout settling. A reverse flick may cross zero during the settle. At 3 seconds and afterward, the object rotates at the current configured idle velocity from its new phase.
8. All phase and velocity integration uses elapsed monotonic time and radians per second, not angle per repaint tick. Equivalent timestamped input produces the same phase and velocity at 15 fps, 30 fps, or delayed repaint schedules.
9. The current phase, active drag origin/displacement, and settling state survive a terminal resize. New layout geometry is used for later hit tests, but a drag that began before resize may continue through it.
10. Interaction state resets when the session replaces the zero state with transcript content or another surface. If the zero state later returns, it is at the ordinary idle animation with no previous drag or momentum contribution.
11. The interaction applies to whichever object the zero-state animation is already rendering: the built-in Warp mark or configured custom ASCII art. The same physics, target calculation, and reset rules apply to both.
12. If the host terminal ignores mouse reporting or otherwise sends no mouse events, the zero state remains exactly today's idle animation. There is no warning, hint, disabled state, or other visible difference.
13. Mouse events outside the current object target remain unhandled by the animation so existing or future zero-state children can receive them. A valid object drag consumes its down, drag, and up events.

### Non-goals
- Persistent reversal of the configured idle animation; reverse motion is user-driven only and always settles back to forward idle.
- Keyboard controls, scroll-wheel control, touchpad gesture abstraction, saved momentum, preferences, telemetry, sound, or reuse on other Warp logo surfaces.
- Hint copy, hover affordances, cursor changes, or other discoverability UI.
- Changes to Crossterm mouse capture, terminal capability negotiation, background-star motion, object shape, extrusion, styling, frame cadence, or the existing rotation-period setting.

## Tech

### Context
- `crates/warpui_core/src/runtime/mod.rs:696 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` enters the TUI alternate screen with `EnableMouseCapture`; `crates/warpui_core/src/runtime/mod.rs:621 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` invalidates all views on resize.
- `crates/warpui_core/src/runtime/event_conversion.rs:45 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` already converts Crossterm left-button down, up, and drag reports into `TuiEvent`.
- `crates/warpui_core/src/presenter/tui.rs:78 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` caches the last laid-out element tree for input dispatch, but invalidated views receive newly rendered elements. Interaction state cannot live only in `ZeroStateAnimationElement` if it must survive resize and unrelated zero-state re-renders.
- `crates/warp_tui/src/zero_state_animation.rs:175 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` owns current object layout and paint geometry. `object_frame_at` at line 258 derives rotation solely from `AnimationClock::elapsed()` and the configured rotation period.
- `crates/warp_tui/src/zero_state.rs:58 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` owns the animation clock across element rebuilds. Its render path stacks the animation behind a text overlay at line 159.
- `crates/warp_tui/src/terminal_session_view.rs:5175 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` is the authoritative visibility decision: an empty transcript embeds `TuiZeroStateView`; otherwise it embeds the transcript.
- `crates/warpui_core/src/elements/tui/stack.rs:132 @ 82f3dce2bd1d36b52b37eb088da79e7d37de974d` dispatches events front-to-back. The text overlay does not handle these mouse events, so the animation layer can consume only valid object drags without changing the stack.

### Constants
Keep the interaction tuning together in `zero_state_animation.rs` so later adjustment does not alter the state machine:

- `DRAG_RADIANS_PER_COLUMN = TAU / 96.0`
- `FLICK_DISTANCE_RESPONSE_DURATION = 500 ms`
- `MAX_INTERACTIVE_REVOLUTIONS_PER_SECOND = 1.5`
- `MOMENTUM_SETTLE_DURATION = 3 s`

The cap applies to total interactive velocity, not to the configured idle setting. The existing setting bounds idle rotation to at most 1 revolution per second, so the selected cap is always at or above idle.

### Design alternatives
- **Interaction model**
  - Selected: direct horizontal scrubbing while held, followed by momentum. This gives immediate, toy-like control and matches the requester's preference after they delegated the choice.
  - Rejected: drag displacement directly sets velocity. This makes small pointer corrections abruptly change speed and does not feel like grabbing the object.
  - Rejected: flick-only input. It is simpler but provides no direct response while held.
- **Direction**
  - Selected: signed scrubbing and signed release velocity. Rightward gestures rotate forward and leftward gestures rotate backward, while the fixed three-second settle always returns to the configured forward idle velocity.
  - Rejected: forward-only control. Requester testing showed that inert left drags made the interaction feel incomplete.
- **State ownership**
  - Selected: a cloneable, interior-mutable interaction handle owned by `TuiTerminalSessionView`, shared with `TuiZeroStateView` and each animation element. The session synchronizes its existing zero-state visibility decision into the handle. This preserves state through invalidation/resize and resets it exactly at a visible-to-hidden transition.
  - Rejected: state only on `ZeroStateAnimationElement`. Resize calls `invalidate_all_views`, and asynchronous zero-state updates rebuild the element, losing the drag.
  - Rejected: a new global model or setting. The state is session-local, ephemeral, and does not need persistence or cross-surface observation.
  - Rejected: infer exit from elapsed time between paints. Suspend, a busy renderer, or a hidden surface could be misclassified.
- **Hit target**
  - Selected: the tight bounding rectangle around non-background cells in the last rendered object frame. It includes sparse silhouette interiors, excludes stars, follows custom shapes and edge-on frames, and is easier to acquire than glyph-only hit testing.
  - Rejected: the entire animation panel. It would steal mouse input far away from the visible object.
  - Rejected: only occupied glyph cells. The wireframe is intentionally sparse and would be unnecessarily difficult to grab.
- **Momentum decay**
  - Selected: a 3-second cubic smoothstep from release velocity to configured idle velocity. It is continuous, deterministic, tunable, and settles in finite time.
  - Rejected: indefinite inertia. It does not restore the existing idle feel.
  - Rejected: exponential decay. It approaches idle asymptotically, making the exact settled state and tests less clear.
- **Mouse support**
  - Selected: reuse current capture and silently do nothing if events do not arrive.
  - Rejected: toggle capture only in the zero state or add capability UI. Capture is already active for the TUI session, so either change adds complexity without enabling this feature.

### Proposed changes
1. In `crates/warp_tui/src/zero_state_animation.rs`, introduce a session-local interaction state and cloneable handle with these responsibilities:
   - Track whether the zero state is visible, whether a valid press/qualified drag is active, mouse-down and current pointer positions, monotonic press time, directly manipulated phase, signed release velocity, and the start time/velocity of the current settle.
   - Expose pure time-parameterized functions for press, drag, release, visibility transition, and resolving angle/velocity at a supplied `Instant`. Production calls use `Instant::now`; tests supply deterministic timestamps.
   - Integrate configured idle velocity and the interactive contribution analytically from elapsed time. Never advance phase by a fixed amount per render.
   - On visible-to-hidden, clear drag, phase offset, and momentum. Hidden-to-visible begins a fresh interaction episode while the existing `AnimationClock` continues to define ordinary idle phase.
2. Pass the shared interaction handle into `ZeroStateAnimationElement`.
   - During paint, resolve the object angle from idle clock plus interaction state, pass that angle into frame generation, and retain the tight non-background object-cell bounding rectangle for the current frame.
   - Split frame generation so object rotation accepts an explicit angle while background stars continue to use ordinary elapsed time. Preserve the existing test helper behavior for idle frames.
   - Implement `dispatch_event` on the animation element. Down hit-tests the retained target and starts a candidate interaction without changing phase; drag handles a previously captured press even outside bounds; up settles or cancels. Each state change calls `event_ctx.notify()` and consumes only events belonging to a valid object interaction.
3. In `crates/warp_tui/src/zero_state.rs`, store/receive the shared handle alongside `AnimationClock` and pass it through every element rebuild. Do not add hover or hint UI, and keep the current `TuiStack` composition.
4. In `crates/warp_tui/src/terminal_session_view.rs`, create and own one interaction handle per terminal session, pass it when constructing `TuiZeroStateView`, and synchronize the existing `transcript_is_empty` visibility decision into it before selecting zero state versus transcript. The synchronization must be idempotent and must not itself notify or trigger a render loop.
5. Extend sibling test files, primarily `zero_state_animation_tests.rs`, `zero_state_tests.rs`, and the focused terminal-session view tests. No `warpui_core` runtime or event-conversion production change is expected because the necessary capture and event variants already exist.

### Open questions resolved
- **Is mouse capture already enabled?** Yes. The TUI enables capture when entering its alternate screen and already converts and dispatches left down/drag/up events. This feature must not alter terminal mouse mode.
- **What does drag control?** Direct horizontal phase scrubbing while held, then momentum on release. The requester delegated this choice; it is selected for immediate feedback.
- **How does momentum end?** It eases to the configured idle velocity over 3 seconds, then remains at idle from the new phase.
- **Can dragging reverse direction?** Yes. Rightward movement rotates forward and leftward movement rotates backward; reverse release momentum smoothly crosses back to the configured forward idle direction during settling.
- **How fast can it spin?** User-driven held and release motion are capped at 1.5 revolutions per second in either direction, with zero release velocity allowed.
- **How is the interaction discovered?** It is not; this is an easter egg with no hint or hover state.
- **What happens without mouse support?** No events means no interaction and no visible difference.
- **What survives resize and state changes?** Interaction survives resize, including an active drag, and resets when the zero state exits.
- **Does custom ASCII art participate?** Yes. It uses the same animation element and is the active zero-state object; special-casing it would make the existing surface inconsistent.

### Risks and mitigations
- **Input feels jumpy or laggy.** Retain the current angle on press, use the single documented `TAU / 96` sensitivity constant, rate-limit held motion with monotonic elapsed time, update on each drag event, and verify short gestures plus low-frame-rate behavior deterministically and live.
- **Fast motion aliases at the 66 ms repaint cadence.** Enforce the 1.5 revolutions-per-second cap and verify live legibility at the cap. If the cap cannot remain legible on the current cadence, lower the constant rather than increasing global repaint frequency.
- **Resize or unrelated view updates lose state.** Keep interaction state in the shared session-owned handle and test reconstruction plus resize explicitly.
- **The animation steals unrelated mouse input.** Start only inside the retained non-background target; return `false` outside it; capture drag/up only after a valid start.
- **Hidden state leaks into a later zero state.** Reset on the session's authoritative visible-to-hidden transition and test exit/re-entry.
- **Overlay composition blocks the animation event.** Preserve the current stack order; the overlay receives events first but leaves unhandled mouse events to the animation.
- **Configured idle periods behave differently.** Derive idle angular velocity from `rotation_period` on every resolution and test the 1-second and 60-second setting bounds.

## Validation and verification criteria

All criteria must pass before merge.

1. **Existing idle behavior remains bit-for-bit stable.** With no interaction, `object_frame_at` produces the same logo cells at representative phases for the built-in logo and custom ASCII fixtures, and background-star positions remain based only on elapsed time. Check with the existing `zero_state_animation_tests` plus a new `idle_frames_are_unchanged_without_interaction` regression test.
2. **Mouse plumbing needs no runtime change.** Existing `warpui_core` tests continue to prove Crossterm down/drag/up conversion, and review confirms `EnableMouseCapture` remains present with no new capture toggle or terminal capability UI. Check with `cargo nextest run -p warpui_core --features tui`.
3. **Hit testing is object-only.** A new `drag_starts_only_inside_current_object_bounds` test must show that a press on the rendered built-in object starts capture, while presses on a background star and blank panel cell return unhandled. Repeat against a custom ASCII object and an edge-on frame.
4. **Press and click never snap.** A new `press_and_click_without_horizontal_motion_preserve_phase_and_velocity` test must compare the resolved angle immediately before press, after press, and after release with no horizontal movement; values must follow the uninterrupted idle solution with no discontinuity or momentum.
5. **Direct drag is bidirectional, retuned, and deterministic.** `horizontal_drag_scrubs_both_directions_with_retuned_sensitivity` must show that eight-cell right and left gestures produce equal and opposite `1/12`-revolution turns, `held_drag_is_rate_limited_by_monotonic_elapsed_time` must prove the 1.5 revolutions-per-second held cap, vertical-only motion must preserve ordinary idle motion, and drag/up remain consumed after the pointer leaves bounds.
6. **Whole-distance velocity is signed and hard-clamped.** `release_velocity_scales_with_signed_total_drag_distance_even_after_a_pause` must cover proportional short/long forward flicks, an equal reverse flick, pause independence, and exact clamping at ±1.5 revolutions per second.
7. **Momentum returns slowly and exactly to idle.** A new `released_velocity_smoothly_settles_to_idle_in_three_seconds` test must assert continuity at release, intermediate smoothstep velocities at fixed timestamps, configured idle velocity at 3 seconds, and idle velocity thereafter for both the 1-second and 60-second configured periods.
8. **Physics is frame-rate independent.** A new `interaction_phase_is_independent_of_repaint_schedule` test must feed identical timestamped drag/release input and resolve it under 15 fps, 30 fps, and irregular/delayed render schedules; angle and velocity at common timestamps must agree within a small floating-point tolerance.
9. **Resize preserves; exit resets.** New focused tests must rebuild/layout the element at a different `TuiSize` during an active drag and during momentum without changing resolved phase/velocity, then drive a zero-state visible-to-hidden-to-visible transition and assert that drag/momentum state is cleared.
10. **No collateral event handling.** A focused presenter/event test must prove down/drag/up outside a valid interaction remain available to front/ancestor handlers, while all three events of a valid object drag are consumed by the animation even when later positions are outside the target.
11. **Small terminals and absent mouse events degrade silently.** Existing too-small layout tests plus a new interaction assertion must show no object target and no handled drag when the animation is hidden. A run with no mouse events must show unchanged idle behavior and no new copy or affordance.
12. **Targeted tests pass.** Run `cargo nextest run -p warp_tui` and `cargo nextest run -p warpui_core --features tui`.
13. **Formatting, linting, and build gates pass.** Run:
    - `./script/format --check`
    - `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings`
    - `cargo clippy -p warp --all-targets --tests -- -D warnings`
    - `cargo clippy -p warp_completer --all-targets --tests -- -D warnings`
    - `CARGO_BUILD_JOBS=2 cargo build -p warp_tui --bin warp-tui-oss`
    The PR's CI is the full-suite backstop for this bounded TUI change.
14. **The running TUI proves the complete gesture.** After the build passes, launch the authenticated zero state with `./script/run-tui` in a real terminal and use computer use to perform and durably record: a no-input idle baseline, valid press with no angle jump, a modest short forward drag, a longer faster forward flick, a reverse drag/flick, gradual 3-second return to forward idle, drag continuation outside the object, and resize during momentum. Show the pointer and contrast interaction segments against baseline so gesture-driven motion is distinguishable from the five-second idle loop.
15. **Live edge cases are checked.** In the same running-TUI verification, confirm a press on stars/blank space does nothing, a click without drag does nothing, vertical-only drag does not freeze or drive the object, the high clamp remains legible without visible flicker, and leaving/re-entering the zero state removes prior momentum. Attach durable video and representative screenshots to the PR, and state plainly which criteria remain deterministic-test-only if the capture cannot establish them visually.
