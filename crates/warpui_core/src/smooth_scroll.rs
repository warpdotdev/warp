//! A deterministic, time-based controller that eases a scroll position toward an exact target
//! for discrete (non-precise) scroll input, instead of jumping immediately. Kept outside the GUI
//! element tree so both generic WarpUI scrollables and terminal scrollback can share it.
//!
//! ## Model
//! - Motion follows a cubic bezier ease-in-out (control points `(0.42, 0)` and `(0.58, 1)`, the
//!   same shape as CSS's `ease-in-out` keyword): a notch from rest eases in before decelerating
//!   into the target, rather than launching at full speed.
//! - Duration is inversely proportional to the delta's magnitude: a small notch gets a longer,
//!   gentler animation, while a large one collapses toward a shorter, snappier one. See
//!   [`inverse_delta_duration`].
//! - A same-direction delta arriving mid-flight *retargets* the running segment rather than
//!   stacking a second one on top of it, reshaping the curve so its start velocity matches the
//!   outgoing velocity -- motion stays continuous across the retarget instead of visibly
//!   restarting. [`velocity_preserving_duration`] bounds how long that reshaping may take, so a
//!   fast-moving retarget with only a small remaining distance can't overshoot past the target.
//! - Opposite-direction input discards the unrendered remainder and reverses immediately from
//!   the currently displayed position.
//!
//! This mirrors Chromium's wheel-scroll animation, `cc::ScrollOffsetAnimationCurve`
//! (`cc/animation/scroll_offset_animation_curve.cc`).

use std::time::Duration;

use instant::Instant;
use warp_features::FeatureFlag;

/// Cadence at which an active [`SmoothScrollController`] requests another repaint. Comfortably
/// exceeds common display refresh rates so this is never the bottleneck on frame cadence.
pub const SMOOTH_SCROLL_FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// Pixels-per-line for converting a non-precise (line-based) scroll delta into the
/// pixel-equivalent units this controller operates in. Chosen over the OS-reported ~10px/line
/// default because that reads as too slow.
pub const NUM_PIXELS_PER_LINE: f32 = 40.0;

/// Whether a scroll input should be animated by a [`SmoothScrollController`] rather than applied
/// immediately: `precise` input always applies immediately, and disabling
/// `FeatureFlag::SmoothScrolling` applies everything immediately.
pub fn should_animate_scroll(precise: bool) -> bool {
    !precise && FeatureFlag::SmoothScrolling.is_enabled()
}

/// A wheel delta at or below [`INVERSE_DELTA_RAMP_START_PX`] gets [`INVERSE_DELTA_MAX_DURATION`]
/// (slow, gentle); a delta at or above [`INVERSE_DELTA_RAMP_END_PX`] gets
/// [`INVERSE_DELTA_MIN_DURATION`] (fast, snappy); between them, duration ramps linearly.
const INVERSE_DELTA_RAMP_START_PX: f32 = 120.0;
const INVERSE_DELTA_RAMP_END_PX: f32 = 480.0;
const INVERSE_DELTA_MAX_DURATION: Duration = Duration::from_millis(200);
const INVERSE_DELTA_MIN_DURATION: Duration = Duration::from_millis(100);

/// Never let a retarget's reshaped duration collapse below this floor (roughly one frame at
/// 60Hz), even if the velocity-based bound would otherwise push it toward zero.
const MIN_RETARGET_DURATION: Duration = Duration::from_millis(16);

/// Duration for a scroll delta, given its absolute magnitude in pixels, ramping linearly
/// between the two ends of the ramp.
fn inverse_delta_duration(abs_delta: f32) -> Duration {
    if abs_delta <= INVERSE_DELTA_RAMP_START_PX {
        return INVERSE_DELTA_MAX_DURATION;
    }
    if abs_delta >= INVERSE_DELTA_RAMP_END_PX {
        return INVERSE_DELTA_MIN_DURATION;
    }

    let t = (abs_delta - INVERSE_DELTA_RAMP_START_PX)
        / (INVERSE_DELTA_RAMP_END_PX - INVERSE_DELTA_RAMP_START_PX);
    let max = INVERSE_DELTA_MAX_DURATION.as_secs_f32();
    let min = INVERSE_DELTA_MIN_DURATION.as_secs_f32();
    Duration::from_secs_f32(max + (min - max) * t)
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

/// Clamp on a reshaped curve's initial slope, preventing overshoot past `y = 1` before `t = 1`.
/// Spot-checked numerically to keep the curve monotonic at this value.
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

/// The fudge factor compensating for the ease-out tail of the curve.
const VELOCITY_DURATION_BOUND_FACTOR: f32 = 2.5;

/// Caps a retarget's duration so a fast-moving animation with a small remaining distance can't
/// overshoot the target and rubber-band back.
///
/// Zero when `remaining_delta` is already zero; unbounded when `current_velocity` is zero or
/// points away from `remaining_delta` (the bound only applies while already moving toward the
/// target).
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

/// Animates a single scroll axis toward an exact target position. See the module-level docs for
/// the model.
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
    /// The displayed position as of the last [`Self::take_increment`] call. See its doc comment.
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
    /// progress (unlike [`Self::displayed_position`]).
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
    /// segment eases from there (at zero velocity).
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
            // Opposite direction: reverse from the currently displayed position.
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
