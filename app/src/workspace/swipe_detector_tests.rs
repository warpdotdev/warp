use std::time::Duration;

use instant::Instant;

use super::{SwipeDetector, SwipeDirection};

const FRAME_MS: u64 = 16;

/// Feeds `(dx, dy)` events one frame apart, starting `start_ms` after `t0`,
/// and returns every direction the detector produced.
fn feed_frames(
    detector: &mut SwipeDetector,
    t0: Instant,
    start_ms: u64,
    events: &[(f32, f32)],
    precise: bool,
) -> Vec<SwipeDirection> {
    events
        .iter()
        .enumerate()
        .filter_map(|(i, &(dx, dy))| {
            let now = t0 + Duration::from_millis(start_ms + i as u64 * FRAME_MS);
            detector.feed(dx, dy, precise, now)
        })
        .collect()
}

fn swipe(dx_per_frame: f32, frames: usize) -> Vec<(f32, f32)> {
    vec![(dx_per_frame, 0.0); frames]
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
