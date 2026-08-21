//! A deterministic, time-based animation controller for smoothing discrete (non-precise)
//! mouse-wheel scroll input.
//!
//! See `specs/CSAT-6046/TECH.md` for the design rationale. This module intentionally lives
//! outside the GUI element tree so that both the generic WarpUI scrollables (Phase 1) and
//! `TerminalView` scrollback (Phase 2) can share the same controller.
//!
//! ## Model
//! This mirrors Chromium's wheel-scroll animation, `cc::ScrollOffsetAnimationCurve`
//! (`cc/animation/scroll_offset_animation_curve.cc`, public on chromium.googlesource.com),
//! adopted after hands-on feedback that a flat 120ms ease-out cubic did not feel smooth enough:
//! - Motion follows a cubic bezier ease-in-out, the same shape as CSS's `ease-in-out` keyword
//!   (control points `(0.42, 0)` and `(0.58, 1)`), rather than an ease-out-only curve. A notch
//!   from rest eases in before decelerating into the target, instead of launching at full speed.
//!   This matches Chromium's `EaseInOutWithInitialSlope`, which also fixes the first control
//!   point's x-coordinate at 0.42 and only varies its y-coordinate to encode a starting slope.
//! - Duration is inversely proportional to the wheel delta (`DurationBehavior::kInverseDelta`):
//!   a single small notch gets a longer, gentler animation; fast spinning collapses toward a
//!   shorter, snappier one. This is a direct port of Chromium's linear ramp between
//!   `kInverseDeltaRampStartPx`/`kInverseDeltaMaxDuration` and
//!   `kInverseDeltaRampEndPx`/`kInverseDeltaMinDuration`. See [`inverse_delta_duration`].
//! - A new same-direction notch arriving mid-flight *retargets* the single running animation
//!   rather than stacking an independent contribution on top of it: the curve is reshaped so
//!   its start velocity matches the outgoing velocity of the animation it's replacing, so
//!   velocity stays continuous across the retarget instead of jumping. A
//!   [`velocity_preserving_duration`] bound, ported from Chromium's `VelocityBasedDurationBound`,
//!   keeps that reshaping from overshooting when the controller is already moving fast and the
//!   newly retargeted distance is small.
//! - Opposite-direction input still discards the unrendered remainder and reverses immediately
//!   from the currently displayed position, as approved (this is unaffected by the model above).

use std::time::Duration;

use instant::Instant;
use warp_features::FeatureFlag;

/// Cadence at which a view with an active [`SmoothScrollController`] should request another
/// repaint. Chosen to comfortably exceed common display refresh rates (e.g. 60Hz / ~16.7ms, or
/// 120Hz / ~8.3ms) so this code's own self-scheduling is never the limiting factor on frame
/// cadence -- any remaining throttling is downstream of the platform's actual redraw cadence.
/// `ShimmeringTextElement` intentionally self-throttles to ~30fps (33ms) because a subtle color
/// animation doesn't benefit from a faster cadence; scroll position is different -- even small
/// differences in displayed position are readily perceptible in translation, so it's requested
/// as often as this display-refresh headroom allows.
pub const SMOOTH_SCROLL_FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// The number of pixels-per-line used to convert a non-precise (line-based) wheel delta into
/// the pixel-equivalent units every [`SmoothScrollController`] operates in, including its
/// duration ramp's 120/480 reference points. Every consumer of this controller that receives
/// line-based input converts through this single constant, so a given gesture animates with
/// the same feel regardless of which scrollable it lands on.
///
/// This mirrors the value cocoa scroll events without
/// [`hasPreciseScrollingDeltas`](https://developer.apple.com/documentation/appkit/nsevent/1525758-hasprecisescrollingdeltas?language=objc)
/// are converted at (see the historical rationale on the generic scrollables' wheel handlers):
/// inspired by the value Chromium and Flutter use, chosen over the OS-reported ~10px/line
/// default because that reads as too slow.
pub const NUM_PIXELS_PER_LINE: f32 = 40.0;

/// Whether a wheel input should be animated by a [`SmoothScrollController`] rather than applied
/// immediately. Precise (trackpad) input always keeps its existing continuous behavior, and
/// disabling `FeatureFlag::SmoothScrolling` preserves the pre-existing immediate-jump behavior
/// for discrete input too.
pub fn should_animate_wheel_input(precise: bool) -> bool {
    !precise && FeatureFlag::SmoothScrolling.is_enabled()
}

/// Chromium's inverse-delta ramp constants (`kInverseDelta*` in
/// `cc/animation/scroll_offset_animation_curve.cc`), expressed in 60ths of a second the way
/// Chromium expresses them (`kInverseDeltaMinDuration = 6.0`, `kInverseDeltaMaxDuration = 12.0`,
/// divided by `kDurationDivisor = 60.0`): a wheel delta at or below
/// [`INVERSE_DELTA_RAMP_START_PX`] gets [`INVERSE_DELTA_MAX_DURATION_60THS`] (slow, gentle); a
/// delta at or above [`INVERSE_DELTA_RAMP_END_PX`] gets [`INVERSE_DELTA_MIN_DURATION_60THS`]
/// (fast, snappy); between them, duration ramps linearly.
const INVERSE_DELTA_MIN_DURATION_60THS: f32 = 6.0;
const INVERSE_DELTA_MAX_DURATION_60THS: f32 = 12.0;
const INVERSE_DELTA_RAMP_START_PX: f32 = 120.0;
const INVERSE_DELTA_RAMP_END_PX: f32 = 480.0;
const DURATION_DIVISOR: f32 = 60.0;

/// Never let a retarget's reshaped duration collapse below this floor (roughly one frame at
/// 60Hz), even if the velocity-based bound would otherwise push it toward zero.
const MIN_RETARGET_DURATION: Duration = Duration::from_millis(16);

/// [`INVERSE_DELTA_MAX_DURATION_60THS`]/[`INVERSE_DELTA_MIN_DURATION_60THS`] expressed directly
/// as [`Duration`]s, so the two ends of the ramp are exact values rather than round-tripping
/// through an `f32` division by [`DURATION_DIVISOR`] (which cannot represent `1/60` exactly).
const INVERSE_DELTA_MAX_DURATION: Duration = Duration::from_millis(200);
const INVERSE_DELTA_MIN_DURATION: Duration = Duration::from_millis(100);

/// Duration for a discrete wheel notch, given the absolute magnitude of its delta in pixels.
/// A direct port of Chromium's `DurationBehavior::kInverseDelta` linear ramp: "makes fast wheel
/// flings feel snappy while preserving smoothness of slow wheel movements."
fn inverse_delta_duration(abs_delta: f32) -> Duration {
    // At or beyond either end of the ramp, return the exact `Duration` constant rather than
    // computing it via the formula below: the formula's `f32` division by `DURATION_DIVISOR`
    // (60) cannot represent `1/60` exactly, so it lands a few nanoseconds off the boundary
    // values -- harmless for anything reading a `Duration` back as a float, but visible to an
    // exact `Duration` comparison.
    if abs_delta <= INVERSE_DELTA_RAMP_START_PX {
        return INVERSE_DELTA_MAX_DURATION;
    }
    if abs_delta >= INVERSE_DELTA_RAMP_END_PX {
        return INVERSE_DELTA_MIN_DURATION;
    }

    // kInverseDeltaSlope = (kInverseDeltaMinDuration - kInverseDeltaMaxDuration)
    //                    / (kInverseDeltaRampEndPx - kInverseDeltaRampStartPx)
    let slope = (INVERSE_DELTA_MIN_DURATION_60THS - INVERSE_DELTA_MAX_DURATION_60THS)
        / (INVERSE_DELTA_RAMP_END_PX - INVERSE_DELTA_RAMP_START_PX);
    // kInverseDeltaOffset = kInverseDeltaMaxDuration - kInverseDeltaRampStartPx * kInverseDeltaSlope
    let offset = INVERSE_DELTA_MAX_DURATION_60THS - INVERSE_DELTA_RAMP_START_PX * slope;
    let duration_60ths = (offset + abs_delta * slope).clamp(
        INVERSE_DELTA_MIN_DURATION_60THS,
        INVERSE_DELTA_MAX_DURATION_60THS,
    );
    Duration::from_secs_f32(duration_60ths / DURATION_DIVISOR)
}

/// The x-coordinate of a bezier curve's first control point, fixed across every curve this
/// controller produces (both the standard ease-in-out shape and every velocity-preserving
/// reshape). Only the first control point's y-coordinate varies, to encode a desired starting
/// velocity; see [`CubicBezier::with_initial_slope`].
const BEZIER_X1: f32 = 0.42;
/// The second control point, fixed across every curve this controller produces: motion always
/// eases out to a stop at the target (the controller always knows the final rest position it's
/// animating toward), matching the tail of CSS's `ease-in-out`.
const BEZIER_X2: f32 = 0.58;
const BEZIER_Y2: f32 = 1.0;

/// The standard ease-in-out timing function -- the same curve as CSS's `ease-in-out` keyword --
/// used for a fresh segment starting from rest (zero initial velocity).
const EASE_IN_OUT: CubicBezier = CubicBezier {
    x1: BEZIER_X1,
    y1: 0.0,
    x2: BEZIER_X2,
    y2: BEZIER_Y2,
};

/// A defensive clamp on a reshaped curve's first control point's y-coordinate: without it, an
/// unreasonably large starting slope (which [`velocity_preserving_duration`] is meant to
/// prevent, but which could still arise from an edge case this code hasn't anticipated) would
/// make the curve overshoot past `y = 1` before `t = 1`. Spot-checked numerically to keep the
/// curve monotonic at this value.
const MAX_INITIAL_SLOPE_Y1: f32 = 1.0;

/// A cubic bezier timing function mapping normalized time (`x`, always in `[0, 1]`) to
/// normalized progress (`y`), evaluated the same way CSS's `cubic-bezier()` is: by numerically
/// solving `x(t) = x_input` for the bezier parameter `t` (Newton-Raphson, falling back to
/// bisection if it doesn't converge), then returning `y(t)`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CubicBezier {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl CubicBezier {
    /// A curve with the same tail shape (easing out to a stop at the target) as
    /// [`EASE_IN_OUT`], but whose start is reshaped so its initial slope (`dy/dx` at `x = 0`)
    /// matches `initial_slope`, clamped to stay within [`MAX_INITIAL_SLOPE_Y1`] once expressed
    /// as this curve's first control point.
    ///
    /// For a bezier anchored at `(0, 0)`, the slope at `t = 0` is `y1 / x1` (both the `x` and
    /// `y` component derivatives at `t = 0` are proportional to `x1` and `y1` respectively).
    /// Holding `x1` fixed and solving for `y1` gives a simple, well-defined way to encode a
    /// desired starting slope without needing to re-derive the whole curve shape.
    fn with_initial_slope(initial_slope: f32) -> Self {
        let y1 = (initial_slope * BEZIER_X1).clamp(0.0, MAX_INITIAL_SLOPE_Y1);
        Self {
            x1: BEZIER_X1,
            y1,
            x2: BEZIER_X2,
            y2: BEZIER_Y2,
        }
    }

    fn sample_component(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
        let mt = 1.0 - t;
        mt * mt * mt * p0 + 3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t * p3
    }

    fn sample_component_derivative(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
        let mt = 1.0 - t;
        3.0 * mt * mt * (p1 - p0) + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (p3 - p2)
    }

    fn sample_x(&self, t: f32) -> f32 {
        Self::sample_component(t, 0.0, self.x1, self.x2, 1.0)
    }

    fn sample_y(&self, t: f32) -> f32 {
        Self::sample_component(t, 0.0, self.y1, self.y2, 1.0)
    }

    fn sample_dx(&self, t: f32) -> f32 {
        Self::sample_component_derivative(t, 0.0, self.x1, self.x2, 1.0)
    }

    fn sample_dy(&self, t: f32) -> f32 {
        Self::sample_component_derivative(t, 0.0, self.y1, self.y2, 1.0)
    }

    /// Solves `x(t) = x_input` for `t`, via Newton-Raphson with a bisection fallback for
    /// robustness (standard technique for evaluating CSS-style cubic-bezier timing functions).
    fn solve_t_for_x(&self, x_input: f32) -> f32 {
        let x_input = x_input.clamp(0.0, 1.0);

        // Newton-Raphson, starting from the linear guess.
        let mut t = x_input;
        for _ in 0..8 {
            let x = self.sample_x(t) - x_input;
            if x.abs() < 1e-6 {
                return t.clamp(0.0, 1.0);
            }
            let dx = self.sample_dx(t);
            if dx.abs() < 1e-6 {
                break;
            }
            t -= x / dx;
            t = t.clamp(0.0, 1.0);
        }

        // Fallback: bisection, guaranteed to converge since sample_x is monotonic on [0, 1] for
        // control points with x1, x2 in [0, 1].
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..30 {
            let mid = (lo + hi) / 2.0;
            if self.sample_x(mid) < x_input {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }
}

/// A single eased motion from `start_position` (at `start_velocity`) to `target` (always ending
/// at rest), over `duration`.
#[derive(Debug, Clone, Copy)]
struct Segment {
    start: Instant,
    start_position: f32,
    start_velocity: f32,
    target: f32,
    duration: Duration,
}

impl Segment {
    fn normalized_time(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        (elapsed / self.duration.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0)
    }

    fn curve(&self) -> CubicBezier {
        let delta = self.target - self.start_position;
        if delta == 0.0 || self.start_velocity == 0.0 {
            return EASE_IN_OUT;
        }
        // Normalized slope = actual velocity * duration / delta (chain rule: normalized time is
        // elapsed/duration, normalized progress is (position - start)/delta).
        let normalized_slope = self.start_velocity * self.duration.as_secs_f32() / delta;
        CubicBezier::with_initial_slope(normalized_slope)
    }

    fn is_complete(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }

    fn position(&self, now: Instant) -> f32 {
        self.sample(now).0
    }

    /// Position and instantaneous velocity (position units per second) at `now`, solving the
    /// bezier parameter `t` only once for both rather than twice (as separate `position`/
    /// `velocity` calls would).
    fn sample(&self, now: Instant) -> (f32, f32) {
        let delta = self.target - self.start_position;
        let duration_secs = self.duration.as_secs_f32();
        if delta == 0.0 {
            return (self.target, 0.0);
        }
        let curve = self.curve();
        let t = curve.solve_t_for_x(self.normalized_time(now));
        let position = self.start_position + delta * curve.sample_y(t);
        let velocity = if duration_secs <= 0.0 {
            0.0
        } else {
            let dx = curve.sample_dx(t);
            let normalized_slope = if dx.abs() < 1e-6 {
                0.0
            } else {
                curve.sample_dy(t) / dx
            };
            normalized_slope * delta / duration_secs
        };
        (position, velocity)
    }
}

/// The fudge factor in Chromium's `VelocityBasedDurationBound`, whose source comment attributes
/// it to compensating for the ease-out tail of the curve.
const VELOCITY_DURATION_BOUND_FACTOR: f32 = 2.5;

/// A direct port of Chromium's `VelocityBasedDurationBound`: caps how long a retarget may take
/// given the controller's current velocity and the remaining distance, so a fast-moving
/// animation retargeted by a small amount doesn't get stretched out to a duration that would
/// make the reshaped curve's starting slope overshoot past the target and rubber-band back.
///
/// Returns zero when `remaining_delta` is already zero (nothing left to bound), and an
/// effectively unbounded duration when `current_velocity` is zero or points the opposite
/// direction from `remaining_delta` (the bound only makes sense while already moving toward the
/// new target).
fn velocity_based_duration_bound(remaining_delta: f32, current_velocity: f32) -> Duration {
    if remaining_delta == 0.0 {
        return Duration::ZERO;
    }
    if current_velocity == 0.0 || current_velocity.signum() != remaining_delta.signum() {
        return Duration::MAX;
    }
    Duration::from_secs_f32(
        (remaining_delta / current_velocity).abs() * VELOCITY_DURATION_BOUND_FACTOR,
    )
}

/// Chooses the duration for a same-direction retarget of `remaining_delta` (the distance still
/// left to travel to the new target, from the currently displayed position) while the
/// controller is already moving at `current_velocity`: the same [`inverse_delta_duration`]
/// every fresh notch gets, bounded by [`velocity_based_duration_bound`].
fn velocity_preserving_duration(remaining_delta: f32, current_velocity: f32) -> Duration {
    let base = inverse_delta_duration(remaining_delta.abs());
    let bound = velocity_based_duration_bound(remaining_delta, current_velocity);
    base.min(bound).max(MIN_RETARGET_DURATION)
}

/// Animates a single scroll axis toward an exact target position using Chromium's wheel-scroll
/// model: a bezier ease-in-out curve, inverse-delta duration, and velocity-preserving retargets.
/// See the module-level docs for the full rationale.
///
/// The controller is a pure function of injected time: every method that depends on "now" takes
/// an explicit [`Instant`] rather than reading the wall clock, which keeps it deterministic and
/// testable.
#[derive(Debug, Clone, Default)]
pub struct SmoothScrollController {
    /// The settled position, used whenever there's no active segment.
    committed: f32,
    /// The single in-flight motion, if any.
    segment: Option<Segment>,
    /// The displayed position as of the last [`Self::take_increment`] call (or construction,
    /// or the last [`Self::cancel`]/[`Self::set_position_immediately`]). See
    /// [`Self::take_increment`].
    last_taken: f32,
}

impl SmoothScrollController {
    pub fn new(initial_position: f32) -> Self {
        Self {
            committed: initial_position,
            segment: None,
            last_taken: initial_position,
        }
    }

    /// Folds a completed segment into `committed`, if there is one. Idempotent.
    fn settle_if_complete(&mut self, now: Instant) {
        if let Some(segment) = self.segment
            && segment.is_complete(now)
        {
            self.committed = segment.target;
            self.segment = None;
        }
    }

    /// The position that should currently be displayed/painted. Settles a completed segment as
    /// a side effect.
    pub fn displayed_position(&mut self, now: Instant) -> f32 {
        self.settle_if_complete(now);
        match self.segment {
            Some(segment) => segment.position(now),
            None => self.committed,
        }
    }

    /// The exact position this controller is animating toward, ignoring the animation's current
    /// progress. Bounds and nested-scroll-propagation decisions should use this rather than
    /// [`Self::displayed_position`], so an inner scrollable doesn't accept wheel input that
    /// belongs to its parent while its own animation is still catching up.
    pub fn target(&self) -> f32 {
        self.segment
            .map_or(self.committed, |segment| segment.target)
    }

    /// Whether a segment is still easing in. Settles a completed segment as a side effect (like
    /// [`Self::displayed_position`]), so this reports the current state even if nothing else has
    /// read the controller since the segment finished.
    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.settle_if_complete(now);
        self.segment.is_some()
    }

    /// Adds a discrete scroll contribution of `delta`, starting at `now`.
    ///
    /// A `delta` in the same direction as the controller's current motion *retargets* the
    /// running segment: the distance still left to travel grows by `delta`, and the curve is
    /// reshaped so its starting velocity matches the outgoing segment's velocity at `now`,
    /// keeping velocity continuous across the retarget rather than restarting from a fresh
    /// zero-velocity ease-in. See [`velocity_preserving_duration`] for how the new segment's
    /// duration is chosen.
    ///
    /// A `delta` in the opposite direction discards the unrendered remainder of the current
    /// motion: the currently displayed position becomes the new settled base, then a fresh
    /// segment eases from there (at zero velocity), exactly as before this model change.
    pub fn add_delta(&mut self, delta: f32, now: Instant) {
        if delta == 0.0 {
            return;
        }

        self.settle_if_complete(now);

        let Some(segment) = self.segment else {
            // Starting from rest: a fresh segment, eased in from zero velocity.
            let target = self.committed + delta;
            self.segment = Some(Segment {
                start: now,
                start_position: self.committed,
                start_velocity: 0.0,
                target,
                duration: inverse_delta_duration(delta.abs()),
            });
            return;
        };

        let (current_position, current_velocity) = segment.sample(now);
        let remaining = segment.target - current_position;

        if remaining != 0.0 && remaining.signum() != delta.signum() {
            // Opposite direction: cancel at the currently displayed position, then ease in
            // fresh from there (zero velocity), discarding the unrendered remainder.
            self.committed = current_position;
            let target = current_position + delta;
            self.segment = Some(Segment {
                start: now,
                start_position: current_position,
                start_velocity: 0.0,
                target,
                duration: inverse_delta_duration(delta.abs()),
            });
            return;
        }

        // Same direction: retarget the running segment, preserving its current velocity.
        let new_target = segment.target + delta;
        let new_remaining = new_target - current_position;
        let duration = velocity_preserving_duration(new_remaining, current_velocity);
        self.segment = Some(Segment {
            start: now,
            start_position: current_position,
            start_velocity: current_velocity,
            target: new_target,
            duration,
        });
    }

    /// Cancels any in-flight animation, settling at the currently displayed position, and
    /// returns that position. Also resyncs [`Self::take_increment`]'s baseline to that position,
    /// so a caller that applies a direct scroll immediately after cancelling (rather than
    /// through the incremental mechanism) doesn't see a stale jump reported on the next
    /// `take_increment` call.
    pub fn cancel(&mut self, now: Instant) -> f32 {
        let displayed = self.displayed_position(now);
        self.committed = displayed;
        self.segment = None;
        self.last_taken = displayed;
        displayed
    }

    /// Immediately jumps to `position`, cancelling any in-flight animation and resyncing
    /// [`Self::take_increment`]'s baseline, for the same reason [`Self::cancel`] does.
    pub fn set_position_immediately(&mut self, position: f32) {
        self.committed = position;
        self.segment = None;
        self.last_taken = position;
    }

    /// Returns the change in [`Self::displayed_position`] since the last call to this method (or
    /// since construction, or the last [`Self::cancel`]/[`Self::set_position_immediately`]),
    /// settling a completed segment as a side effect like [`Self::displayed_position`].
    ///
    /// This hoists the "track what's already been applied, return only the remainder" pattern
    /// that callers which apply the animation incrementally (rather than painting an absolute
    /// position directly) would otherwise have to hand-roll themselves.
    pub fn take_increment(&mut self, now: Instant) -> f32 {
        let current = self.displayed_position(now);
        let increment = current - self.last_taken;
        self.last_taken = current;
        increment
    }
}

#[cfg(test)]
#[path = "smooth_scroll_tests.rs"]
mod tests;
