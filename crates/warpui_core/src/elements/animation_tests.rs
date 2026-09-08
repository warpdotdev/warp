use std::time::Duration;

use super::{AnimationClock, Keyframe, KeyframeTimeline};

/// Millisecond [`Duration`] shorthand.
fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[test]
fn holds_each_keyframe_for_its_duration_and_loops() {
    let timeline = KeyframeTimeline::new([
        Keyframe::from_millis("a", 100),
        Keyframe::from_millis("b", 50),
    ]);
    assert_eq!(*timeline.value_at(ms(0)), "a");
    assert_eq!(*timeline.value_at(ms(99)), "a");
    assert_eq!(*timeline.value_at(ms(100)), "b");
    assert_eq!(*timeline.value_at(ms(149)), "b");
    // The timeline loops from its 150ms period.
    assert_eq!(*timeline.value_at(ms(150)), "a");
    assert_eq!(*timeline.value_at(ms(400)), "b");
}

#[test]
fn skips_zero_hold_keyframes() {
    let timeline = KeyframeTimeline::new([
        Keyframe::new("a", ms(100)),
        Keyframe::new("b", Duration::ZERO),
        Keyframe::new("c", ms(100)),
    ]);
    assert_eq!(*timeline.value_at(ms(99)), "a");
    assert_eq!(*timeline.value_at(ms(100)), "c");
}

#[test]
fn values_are_in_timeline_order() {
    let timeline =
        KeyframeTimeline::new([Keyframe::from_millis("a", 1), Keyframe::from_millis("b", 1)]);
    assert_eq!(timeline.values().copied().collect::<Vec<_>>(), ["a", "b"]);
}

#[test]
#[should_panic(expected = "non-zero hold")]
fn rejects_a_timeline_with_no_duration() {
    KeyframeTimeline::<&str>::new([]);
}

#[test]
fn clock_starts_at_its_initial_elapsed_and_advances() {
    // An initial offset far beyond any plausible process uptime must not
    // panic (the underflow `Instant::now() - elapsed` would) and must be
    // preserved in the reported elapsed time.
    let initial = Duration::from_secs(60 * 60 * 24 * 365 * 100);
    let clock = AnimationClock::starting_at(initial);
    assert!(clock.elapsed() >= initial);
}
