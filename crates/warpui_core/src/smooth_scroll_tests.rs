use std::time::Duration;

use instant::Instant;

use super::{
    CubicBezier, INVERSE_DELTA_RAMP_END_PX, INVERSE_DELTA_RAMP_START_PX, SmoothScrollController,
    inverse_delta_duration, velocity_based_duration_bound, velocity_preserving_duration,
};

const INVERSE_DELTA_MAX_DURATION: Duration = Duration::from_millis(200);
const INVERSE_DELTA_MIN_DURATION: Duration = Duration::from_millis(100);

// NOTE: this suite replaces the previous ease-out-cubic / additive-independent-contributions
// model's tests wholesale, rather than deleting coverage quietly. The two models make
// incompatible promises about what a same-direction composition looks like:
//   - Old model: each notch eased independently; the displayed position was the sum of every
//     active contribution's own eased progress, and `target()` was simply the sum of deltas.
//   - New model (this file): a single running segment is *retargeted* on each same-direction
//     notch, reshaping its curve to preserve the outgoing velocity instead of layering another
//     independent ease on top.
// Tests below pin the new contract's equivalents of the old guarantees (exact landing on
// target, no lost movement across a burst) plus its new one (velocity continuity across a
// retarget), rather than the old "sum of independently-eased contributions" behavior.

#[test]
fn ease_in_out_reaches_exact_target_without_overshoot() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);
    let duration = inverse_delta_duration(100.0);

    for millis in [0, 15, 30, 60, 90] {
        let displayed = controller.displayed_position(start + Duration::from_millis(millis));
        assert!(
            (0.0..=100.0).contains(&displayed),
            "displayed position {displayed} out of bounds at {millis}ms"
        );
    }

    let displayed = controller.displayed_position(start + duration);
    assert_eq!(displayed, 100.0);
    assert!(!controller.is_animating(start + duration));
}

#[test]
fn fresh_segment_eases_in_from_zero_velocity() {
    // A defining difference from the old ease-out-only model: motion starts slow and ramps up,
    // rather than launching at peak velocity. Early in the animation, displayed progress should
    // be well behind the halfway point of elapsed *time* progress.
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(120.0, start);
    let duration = inverse_delta_duration(120.0);

    let ten_percent_in = start + duration.mul_f32(0.1);
    let progress_fraction = controller.displayed_position(ten_percent_in) / 120.0;
    assert!(
        progress_fraction < 0.1,
        "expected an eased-in start to lag behind linear progress at 10% elapsed time, got \
         progress fraction {progress_fraction}"
    );
}

#[test]
fn opposing_input_discards_unrendered_remainder_and_reverses_immediately() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    let reversal_time = start + Duration::from_millis(60);
    let displayed_at_reversal = controller.displayed_position(reversal_time);
    assert!(displayed_at_reversal > 0.0 && displayed_at_reversal < 100.0);

    // Reverse direction before the first segment finishes.
    controller.add_delta(-30.0, reversal_time);

    // Reversal starts from the currently displayed position, not from 0 or from the old target.
    assert_eq!(
        controller.displayed_position(reversal_time),
        displayed_at_reversal
    );
    assert_eq!(controller.target(), displayed_at_reversal - 30.0);

    // The old (discarded) target of 100 is never reached; the new one is, exactly.
    let far_future = reversal_time + inverse_delta_duration(30.0);
    let final_position = controller.displayed_position(far_future);
    assert_eq!(final_position, displayed_at_reversal - 30.0);
    assert!(!controller.is_animating(far_future));
}

#[test]
fn same_direction_retarget_lands_exactly_on_the_combined_target() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(120.0, start);

    let midpoint = start + Duration::from_millis(60);
    let progress_before_second_notch = controller.displayed_position(midpoint);
    assert!(progress_before_second_notch > 0.0);

    // A second same-direction notch arrives mid-flight and retargets the running segment.
    controller.add_delta(120.0, midpoint);

    // The already-visible motion isn't discarded: displayed position doesn't regress.
    let just_after = controller.displayed_position(midpoint);
    assert!(just_after >= progress_before_second_notch);

    // The eventual target is the sum of both contributions, same promise as before.
    assert_eq!(controller.target(), 240.0);
    let far_future = midpoint + Duration::from_secs(1);
    let final_position = controller.displayed_position(far_future);
    assert_eq!(final_position, 240.0);
    assert!(!controller.is_animating(far_future));
}

#[test]
fn same_direction_retarget_preserves_velocity_across_the_seam() {
    // The new model's central promise, replacing the old "independent stacked eases" behavior:
    // a retarget must not create a velocity discontinuity. Sample displayed position on a fine
    // grid straddling the retarget instant and check the local slope (an approximation of
    // velocity) doesn't jump abruptly.
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(480.0, start);

    let retarget_at = start + Duration::from_millis(40);
    let step = Duration::from_micros(200);

    let before_retarget = controller.displayed_position(retarget_at - step);
    let at_retarget = controller.displayed_position(retarget_at);
    let velocity_before = (at_retarget - before_retarget) / step.as_secs_f32();

    controller.add_delta(480.0, retarget_at);

    let just_after = controller.displayed_position(retarget_at + step);
    let velocity_after = (just_after - at_retarget) / step.as_secs_f32();

    // Velocity right after the retarget should be close to velocity right before it -- not
    // reset to zero (which the old independent-contribution model didn't do either, but which a
    // naive "always restart at rest" retarget would) and not discontinuously larger.
    let relative_difference = (velocity_after - velocity_before).abs() / velocity_before.abs();
    assert!(
        relative_difference < 0.15,
        "expected velocity to stay roughly continuous across the retarget, got {velocity_before} \
         before vs {velocity_after} after (relative difference {relative_difference})"
    );
}

#[test]
fn cancel_settles_at_displayed_position_and_stops_animation() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    let cancel_time = start + Duration::from_millis(45);
    let displayed_at_cancel = controller.displayed_position(cancel_time);
    let returned = controller.cancel(cancel_time);

    assert_eq!(returned, displayed_at_cancel);
    assert!(!controller.is_animating(cancel_time));
    assert_eq!(controller.target(), displayed_at_cancel);

    // No further motion happens once cancelled, even much later.
    let later = cancel_time + Duration::from_secs(1);
    assert_eq!(controller.displayed_position(later), displayed_at_cancel);
}

#[test]
fn set_position_immediately_overrides_in_flight_animation() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    controller.set_position_immediately(250.0);

    assert!(!controller.is_animating(start));
    assert_eq!(controller.target(), 250.0);
    assert_eq!(
        controller.displayed_position(start + Duration::from_millis(60)),
        250.0
    );
}

#[test]
fn zero_delta_is_a_no_op() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(5.0);
    controller.add_delta(0.0, start);

    assert!(!controller.is_animating(start));
    assert_eq!(controller.displayed_position(start), 5.0);
}

/// Regression test for a rapid burst of clicky-wheel notches (the reported input pattern is a
/// trackball spun fast, producing dozens of same-direction notches within a short window). Many
/// consecutive retargets must sum exactly, never cancel, saturate, or drop to zero net movement.
#[test]
fn long_rapid_same_direction_burst_reaches_exact_sum_of_deltas() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    let per_notch = 40.0;
    let notch_count = 25_u32;
    let spacing = Duration::from_millis(3);

    for i in 0..notch_count {
        controller.add_delta(per_notch, start + spacing * i);
    }

    let expected_total = per_notch * notch_count as f32;
    let burst_end = start + spacing * (notch_count - 1);
    assert_eq!(controller.target(), expected_total);
    assert!(controller.is_animating(burst_end));

    // Sampling mid-burst never regresses or exceeds the running target: no cancellation or
    // saturation from having retargeted many times in quick succession.
    let mid_burst = start + spacing * (notch_count / 2);
    let displayed_mid_burst = controller.displayed_position(mid_burst);
    assert!(displayed_mid_burst > 0.0);
    assert!(displayed_mid_burst <= controller.target());

    // Once the final segment fully eases in, the displayed position lands on the exact total,
    // with no accumulated error and no lost movement.
    let long_after = burst_end + Duration::from_secs(1);
    assert_eq!(controller.displayed_position(long_after), expected_total);
    assert!(!controller.is_animating(long_after));
}

#[test]
fn inverse_delta_duration_ramps_between_the_two_reference_points() {
    // Below or at the small reference point: the slow, gentle end.
    assert_eq!(
        inverse_delta_duration(INVERSE_DELTA_RAMP_START_PX),
        INVERSE_DELTA_MAX_DURATION
    );
    assert_eq!(inverse_delta_duration(10.0), INVERSE_DELTA_MAX_DURATION);

    // At or above the large reference point: the fast, snappy end.
    assert_eq!(
        inverse_delta_duration(INVERSE_DELTA_RAMP_END_PX),
        INVERSE_DELTA_MIN_DURATION
    );
    assert_eq!(inverse_delta_duration(2000.0), INVERSE_DELTA_MIN_DURATION);

    // Strictly between the two references, duration strictly decreases as delta grows.
    let mid_low = inverse_delta_duration(200.0);
    let mid_high = inverse_delta_duration(350.0);
    assert!(mid_low < INVERSE_DELTA_MAX_DURATION);
    assert!(mid_high > INVERSE_DELTA_MIN_DURATION);
    assert!(mid_high < mid_low);
}

/// Pins the exact shape of the ramp between the two reference points, not just that it's
/// monotonic and unbroken: this is a *linear* interpolation (a direct port of Chromium's
/// `kInverseDeltaSlope`/`kInverseDeltaOffset`), not the hyperbola an earlier revision used. At
/// the exact midpoint of the ramp (300px, halfway between 120 and 480), a linear ramp lands on
/// exactly the midpoint duration (150ms, halfway between 100ms and 200ms); the hyperbola this
/// replaced would have landed on 120ms instead, so this assertion would fail under that model.
#[test]
fn inverse_delta_duration_ramps_linearly_not_hyperbolically() {
    let actual = inverse_delta_duration(300.0);
    let expected = Duration::from_millis(150);
    let tolerance = Duration::from_micros(10);
    let diff = actual.max(expected) - actual.min(expected);
    assert!(
        diff <= tolerance,
        "expected ~150ms (the midpoint of a linear ramp), got {actual:?}"
    );
}

#[test]
fn velocity_preserving_duration_bound_shrinks_when_moving_fast_toward_a_small_remaining_delta() {
    // A large remaining delta at a modest velocity: the bound shouldn't kick in, so the
    // duration matches the plain inverse-delta duration for that remaining distance.
    let unconstrained = velocity_preserving_duration(480.0, 50.0);
    assert_eq!(unconstrained, inverse_delta_duration(480.0));

    // The same velocity, but only a tiny remaining delta left: without a bound, the reshaped
    // curve's starting slope would need to be enormous to reach the target in the "natural"
    // inverse-delta duration for such a small distance (which is already at the *max*, slowest,
    // end -- exactly the scenario that would rubber-band). The bound must shrink the duration
    // well below that unconstrained value instead.
    let bounded = velocity_preserving_duration(5.0, 500.0);
    assert!(
        bounded < inverse_delta_duration(5.0),
        "expected the velocity-based bound to shrink the duration below the unconstrained \
         inverse-delta value, got {bounded:?} vs {:?}",
        inverse_delta_duration(5.0)
    );
}

#[test]
fn cubic_bezier_ease_in_out_matches_known_reference_values() {
    // Cross-check our Newton-Raphson solver against known sampled values of CSS's standard
    // `ease-in-out` (`cubic-bezier(0.42, 0, 0.58, 1)`) at a few round inputs.
    let curve = CubicBezier {
        x1: 0.42,
        y1: 0.0,
        x2: 0.58,
        y2: 1.0,
    };
    let ease = |x: f32| curve.sample_y(curve.solve_t_for_x(x));

    assert_eq!(ease(0.0), 0.0);
    assert_eq!(ease(1.0), 1.0);
    // The curve is symmetric about (0.5, 0.5).
    assert!((ease(0.5) - 0.5).abs() < 1e-4);
    // Ease-in-out starts and ends slow: progress at 25% elapsed time is well under 25%, and
    // progress at 75% elapsed time is well over 75%.
    assert!(ease(0.25) < 0.2);
    assert!(ease(0.75) > 0.8);
}

/// Direct unit coverage for the two guards `velocity_based_duration_bound` ports from
/// Chromium's `VelocityBasedDurationBound`, beyond what
/// `velocity_preserving_duration_bound_shrinks_when_moving_fast_toward_a_small_remaining_delta`
/// already exercises through the composed function.
#[test]
fn velocity_based_duration_bound_guards() {
    // Already at target: nothing left to bound.
    assert_eq!(velocity_based_duration_bound(0.0, 500.0), Duration::ZERO);

    // No velocity to preserve: the bound doesn't apply.
    assert_eq!(velocity_based_duration_bound(10.0, 0.0), Duration::MAX);

    // Velocity pointing the opposite direction from the remaining delta: the bound only makes
    // sense while already moving toward the new target, so it's a no-op here too.
    assert_eq!(velocity_based_duration_bound(10.0, -5.0), Duration::MAX);
    assert_eq!(velocity_based_duration_bound(-10.0, 5.0), Duration::MAX);

    // A same-signed, ordinary case: the bound is `(remaining_delta / velocity).abs() * 2.5`,
    // matching Chromium's documented fudge factor exactly.
    assert_eq!(
        velocity_based_duration_bound(100.0, 200.0),
        Duration::from_secs_f32(100.0 / 200.0 * 2.5)
    );
}

/// `take_increment` hoists the "track what's already been applied, return only the remainder"
/// pattern that `ScrollState`/`SmoothScrollHandle` used to hand-roll themselves.
#[test]
fn take_increment_reports_only_the_unapplied_remainder() {
    let start = Instant::now();
    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);

    // Captured before the segment ever settles: once settled, `displayed_position` for an
    // earlier timestamp would incorrectly return the final committed value instead of the
    // partial progress that was actually visible at that instant, since a settled controller no
    // longer has a segment to evaluate at an arbitrary past time. Real callers only ever pass
    // monotonically increasing timestamps, so this ordering constraint is not a real limitation.
    let checkpoint = start + Duration::from_millis(30);
    let position_at_checkpoint = controller.displayed_position(checkpoint);

    let first_increment = controller.take_increment(checkpoint);
    assert_eq!(first_increment, position_at_checkpoint);

    // A second call before any further motion reports no additional movement, since the first
    // call already advanced the baseline.
    assert_eq!(controller.take_increment(checkpoint), 0.0);

    // A later call reports only the *additional* movement since the checkpoint, not the total
    // displayed position.
    let later = start + Duration::from_secs(1);
    let position_at_later = controller.displayed_position(later);
    let second_increment = controller.take_increment(later);
    assert_eq!(second_increment, position_at_later - position_at_checkpoint);
}

/// `cancel` and `set_position_immediately` must resync `take_increment`'s baseline, so a caller
/// that applies a position change outside the incremental mechanism doesn't see a stale jump
/// reported on the next `take_increment` call.
#[test]
fn cancel_and_set_position_immediately_resync_take_increment() {
    let start = Instant::now();

    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);
    let cancel_time = start + Duration::from_millis(30);
    controller.cancel(cancel_time);
    assert_eq!(controller.take_increment(cancel_time), 0.0);

    let mut controller = SmoothScrollController::new(0.0);
    controller.add_delta(100.0, start);
    controller.set_position_immediately(250.0);
    assert_eq!(controller.take_increment(start), 0.0);
}
