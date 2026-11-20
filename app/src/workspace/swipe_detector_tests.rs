use std::time::Duration;

use instant::Instant;
use warpui::event::ScrollPhase;

use super::{SwipeDetector, SwipeDirection};

const FRAME_MS: u64 = 16;

/// Feeds `(dx, dy)` events without phase info one frame apart, starting
/// `start_ms` after `t0`, and returns every direction the detector produced.
fn feed_frames(
    detector: &mut SwipeDetector,
    t0: Instant,
    start_ms: u64,
    events: &[(f32, f32)],
    precise: bool,
) -> Vec<SwipeDirection> {
    let events = events.iter().map(|&(dx, dy)| (dx, dy, None));
    feed(detector, t0, start_ms, events, precise)
}

/// Like [`feed_frames`], for trackpad events that carry their gesture phase.
fn feed_phased(
    detector: &mut SwipeDetector,
    t0: Instant,
    events: &[(f32, f32, ScrollPhase)],
) -> Vec<SwipeDirection> {
    let events = events.iter().map(|&(dx, dy, phase)| (dx, dy, Some(phase)));
    feed(detector, t0, 0, events, true)
}

fn feed(
    detector: &mut SwipeDetector,
    t0: Instant,
    start_ms: u64,
    events: impl Iterator<Item = (f32, f32, Option<ScrollPhase>)>,
    precise: bool,
) -> Vec<SwipeDirection> {
    events
        .enumerate()
        .filter_map(|(i, (dx, dy, phase))| {
            let now = t0 + Duration::from_millis(start_ms + i as u64 * FRAME_MS);
            detector.feed(dx, dy, precise, phase, now)
        })
        .collect()
}

fn swipe(dx_per_frame: f32, frames: usize) -> Vec<(f32, f32)> {
    vec![(dx_per_frame, 0.0); frames]
}

/// Fingers on the trackpad: `Began`, `Changed` frames, then a zero-delta `Ended`.
fn phased_swipe(dx_per_frame: f32, frames: usize) -> Vec<(f32, f32, ScrollPhase)> {
    let mut events = vec![(dx_per_frame, 0.0, ScrollPhase::Began)];
    events.extend(vec![(dx_per_frame, 0.0, ScrollPhase::Changed); frames - 1]);
    events.push((0.0, 0.0, ScrollPhase::Ended));
    events
}

/// The decaying momentum tail macOS streams after the fingers lift.
fn momentum(dx: f32, frames: i32) -> Vec<(f32, f32, ScrollPhase)> {
    (0..frames)
        .map(|i| (dx * 0.92_f32.powi(i), 0.0, ScrollPhase::Momentum))
        .collect()
}

#[test]
fn fingers_left_switches_to_next_space_once() {
    let mut d = SwipeDetector::default();
    let fired = feed_frames(&mut d, Instant::now(), 0, &swipe(-10.0, 12), true);
    assert_eq!(fired, vec![SwipeDirection::Next]);
}

#[test]
fn fingers_right_switches_to_previous_space_once() {
    let mut d = SwipeDetector::default();
    let fired = feed_frames(&mut d, Instant::now(), 0, &swipe(10.0, 12), true);
    assert_eq!(fired, vec![SwipeDirection::Prev]);
}

#[test]
fn momentum_tail_does_not_fire_again() {
    let mut d = SwipeDetector::default();
    let mut events = swipe(-10.0, 12);
    // Decaying momentum, still one frame apart: no pause, same gesture.
    events.extend((0..40).map(|i| (-8.0 * 0.92_f32.powi(i), 0.0)));
    let fired = feed_frames(&mut d, Instant::now(), 0, &events, true);
    assert_eq!(fired, vec![SwipeDirection::Next]);
}

#[test]
fn a_pause_starts_a_new_gesture() {
    let mut d = SwipeDetector::default();
    let t0 = Instant::now();
    let mut fired = feed_frames(&mut d, t0, 0, &swipe(-10.0, 12), true);
    fired.extend(feed_frames(
        &mut d,
        t0,
        12 * FRAME_MS + 300,
        &swipe(-10.0, 12),
        true,
    ));
    assert_eq!(fired, vec![SwipeDirection::Next, SwipeDirection::Next]);
}

#[test]
fn small_movements_do_not_add_up_across_gestures() {
    let mut d = SwipeDetector::default();
    let t0 = Instant::now();
    // 50 + 50 would cross the threshold, but a pause separates them.
    let mut fired = feed_frames(&mut d, t0, 0, &swipe(-10.0, 5), true);
    fired.extend(feed_frames(
        &mut d,
        t0,
        5 * FRAME_MS + 300,
        &swipe(-10.0, 5),
        true,
    ));
    assert!(fired.is_empty(), "got {fired:?}");
}

#[test]
fn vertical_scrolling_never_switches() {
    let mut d = SwipeDetector::default();
    let events = vec![(-6.0, -20.0); 40];
    let fired = feed_frames(&mut d, Instant::now(), 0, &events, true);
    assert!(fired.is_empty(), "got {fired:?}");
}

#[test]
fn mouse_wheel_never_switches() {
    let mut d = SwipeDetector::default();
    // Shift+wheel on a mouse: horizontal, but not precise.
    let fired = feed_frames(&mut d, Instant::now(), 0, &swipe(-30.0, 12), false);
    assert!(fired.is_empty(), "got {fired:?}");
}

#[test]
fn second_swipe_during_momentum_switches_again() {
    let mut d = SwipeDetector::default();
    // The second swipe starts while the first one's momentum is still
    // streaming, with no pause between events.
    let mut events = phased_swipe(-10.0, 12);
    events.extend(momentum(-8.0, 10));
    events.extend(phased_swipe(-10.0, 12));
    events.extend(momentum(-8.0, 40));
    let fired = feed_phased(&mut d, Instant::now(), &events);
    assert_eq!(fired, vec![SwipeDirection::Next, SwipeDirection::Next]);
}

#[test]
fn momentum_does_not_complete_a_short_swipe() {
    let mut d = SwipeDetector::default();
    // 40 from the fingers, then ~96 of momentum: only the fingers count.
    let mut events = phased_swipe(-10.0, 4);
    events.extend(momentum(-8.0, 40));
    let fired = feed_phased(&mut d, Instant::now(), &events);
    assert!(fired.is_empty(), "got {fired:?}");
}

#[test]
fn without_phase_info_a_swipe_during_the_tail_stays_in_the_same_gesture() {
    let mut d = SwipeDetector::default();
    // The same stream as above with the phases stripped: only a pause can end
    // a gesture, so the second swipe is still merged into the first.
    let mut events = phased_swipe(-10.0, 12);
    events.extend(momentum(-8.0, 10));
    events.extend(phased_swipe(-10.0, 12));
    let events = events
        .into_iter()
        .map(|(dx, dy, _)| (dx, dy))
        .collect::<Vec<_>>();
    let fired = feed_frames(&mut d, Instant::now(), 0, &events, true);
    assert_eq!(fired, vec![SwipeDirection::Next]);
}
