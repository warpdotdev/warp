//! Time-based, exact-target scrolling for discrete wheel input, shared by GUI scrollables and
//! terminal scrollback. The inverse-delta duration ramp and velocity-bound retargeting follow
//! Chromium's `cc::ScrollOffsetAnimationCurve` (`cc/animation/scroll_offset_animation_curve.cc`).

use std::time::Duration;

use instant::Instant;
use warp_features::FeatureFlag;

/// Cadence for active smooth-scroll frames.
pub const SMOOTH_SCROLL_FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// Pixels per line-based wheel unit; the OS-reported ~10px/line feels too slow.
pub const NUM_PIXELS_PER_LINE: f32 = 40.0;

/// Animates only non-precise input while `FeatureFlag::SmoothScrolling` is enabled.
pub fn should_animate_scroll(precise: bool) -> bool {
    !precise && FeatureFlag::SmoothScrolling.is_enabled()
}

const INVERSE_DELTA_RAMP_START_PX: f32 = 120.0;
const INVERSE_DELTA_RAMP_END_PX: f32 = 480.0;
const INVERSE_DELTA_MAX_DURATION: Duration = Duration::from_millis(200);
const INVERSE_DELTA_MIN_DURATION: Duration = Duration::from_millis(100);

/// Avoid a retarget shorter than one 60Hz frame.
const MIN_RETARGET_DURATION: Duration = Duration::from_millis(16);

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

const BEZIER_X1: f32 = 0.42;
const BEZIER_X2: f32 = 0.58;
const BEZIER_Y2: f32 = 1.0;

/// CSS `ease-in-out` for motion from rest.
const EASE_IN_OUT: CubicBezier = CubicBezier {
    x1: BEZIER_X1,
    y1: 0.0,
    x2: BEZIER_X2,
    y2: BEZIER_Y2,
};

/// Keep retarget curves monotonic without overshoot.
const MAX_INITIAL_SLOPE_Y1: f32 = 1.0;

/// Maps normalized time to progress by solving the cubic bezier's x-coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CubicBezier {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl CubicBezier {
    /// At the origin, `dy/dx = y1/x1`; vary `y1` to match the outgoing velocity while keeping
    /// the ease-out tail fixed.
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

    /// Solves `x(t) = x_input`, falling back to bisection if Newton-Raphson stalls.
    fn solve_t_for_x(&self, x_input: f32) -> f32 {
        let x_input = x_input.clamp(0.0, 1.0);

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

        // x(t) is monotonic for these control points, so bisection converges.
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

/// Chromium's ease-out-tail allowance in the velocity duration bound.
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

fn velocity_preserving_duration(remaining_delta: f32, current_velocity: f32) -> Duration {
    let base = inverse_delta_duration(remaining_delta.abs());
    let bound = velocity_based_duration_bound(remaining_delta, current_velocity);
    base.min(bound).max(MIN_RETARGET_DURATION)
}

/// Animates one scroll axis toward an exact target using caller-provided time.
#[derive(Debug, Clone, Default)]
pub struct SmoothScrollController {
    committed: f32,
    segment: Option<Segment>,
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

    /// The exact destination, regardless of current animation progress.
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

    /// Adds a delta, preserving velocity on same-direction retargets. Reversals discard pending
    /// movement and restart from the displayed position at rest.
    pub fn add_delta(&mut self, delta: f32, now: Instant) {
        if delta == 0.0 {
            return;
        }

        self.settle_if_complete(now);

        let Some(segment) = self.segment else {
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

    /// Cancels at and returns the displayed position. Resets the incremental baseline so direct
    /// scrolling does not inherit unapplied movement.
    pub fn cancel(&mut self, now: Instant) -> f32 {
        let displayed = self.displayed_position(now);
        self.committed = displayed;
        self.segment = None;
        self.last_taken = displayed;
        displayed
    }

    /// Jumps to `position`, cancels animation, and resets the incremental baseline.
    pub fn set_position_immediately(&mut self, position: f32) {
        self.committed = position;
        self.segment = None;
        self.last_taken = position;
    }

    /// Emits movement since the last `take_increment`, construction, or baseline reset. Settles
    /// completed segments as a side effect; a final call is still required after
    /// [`Self::is_animating`] first returns false to emit the remaining distance.
    pub fn take_increment(&mut self, now: Instant) -> f32 {
        let current = self.displayed_position(now);
        let increment = current - self.last_taken;
        self.last_taken = current;
        increment
    }

    /// The portion of [`Self::target`] not yet returned by [`Self::take_increment`]: how much
    /// farther a caller that only applies emitted increments still needs to move before it
    /// matches where this animation will settle.
    pub fn remaining_target_delta(&self) -> f32 {
        self.target() - self.last_taken
    }
}

#[cfg(test)]
#[path = "smooth_scroll_tests.rs"]
mod tests;
