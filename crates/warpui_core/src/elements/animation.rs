//! Backend-agnostic animation timing: a monotonic-safe elapsed clock
//! ([`AnimationClock`]) and a looping sequence of values each held for a
//! fixed duration ([`KeyframeTimeline`]).
//!
//! Elements that animate through discrete frames (e.g. spinner glyphs)
//! describe their choreography as a [`KeyframeTimeline`] and ask it which
//! value is current for a given elapsed time, instead of hand-rolling frame
//! walking per element.

use std::time::Duration;

use instant::Instant;

/// A monotonic-safe animation clock: the animation's elapsed time captured
/// when the clock was built, advanced by the monotonic time since.
///
/// Exists because an animation's starting offset often comes from the wall
/// clock (e.g. an exchange's start timestamp), and reconstructing a monotonic
/// anchor from it (`Instant::now() - elapsed`) panics when the duration
/// exceeds the monotonic clock's range — e.g. after suspend (the monotonic
/// clock doesn't tick while asleep) or wall-clock skew. Tracking the offset
/// as a plain `Duration` and *adding* monotonic time since build avoids the
/// underflow entirely while keeping the animation continuous across element
/// rebuilds.
#[derive(Clone, Copy)]
pub struct AnimationClock {
    /// How far into the animation the caller already was at build time.
    initial_elapsed: Duration,
    /// When this clock was built.
    anchor: Instant,
}

impl AnimationClock {
    /// A clock that is already `initial_elapsed` into the animation and
    /// advances with the monotonic clock from now.
    pub fn starting_at(initial_elapsed: Duration) -> Self {
        Self {
            initial_elapsed,
            anchor: Instant::now(),
        }
    }

    /// The animation's total elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.initial_elapsed + self.anchor.elapsed()
    }
}

/// One keyframe: a value shown for a fixed hold duration.
pub struct Keyframe<T> {
    value: T,
    hold: Duration,
}

impl<T> Keyframe<T> {
    /// A keyframe showing `value` for `hold`.
    pub const fn new(value: T, hold: Duration) -> Self {
        Self { value, hold }
    }

    /// A keyframe showing `value` for `hold_ms` milliseconds.
    pub const fn from_millis(value: T, hold_ms: u64) -> Self {
        Self::new(value, Duration::from_millis(hold_ms))
    }
}

/// A looping keyframe timeline: an ordered sequence of values, each held for
/// its own duration, repeating once the final keyframe's hold elapses.
pub struct KeyframeTimeline<T> {
    /// Each keyframe's value paired with its cumulative end offset from the
    /// start of the loop; offsets are non-decreasing, ending at `period`.
    frames: Vec<(T, Duration)>,
    /// The total duration of one loop.
    period: Duration,
}

impl<T> KeyframeTimeline<T> {
    /// Builds a timeline from [`Keyframe`]s.
    ///
    /// Panics if no keyframe has a non-zero hold, since the timeline would
    /// have no current value.
    pub fn new(keyframes: impl IntoIterator<Item = Keyframe<T>>) -> Self {
        let mut period = Duration::ZERO;
        let frames: Vec<_> = keyframes
            .into_iter()
            .map(|keyframe| {
                period += keyframe.hold;
                (keyframe.value, period)
            })
            .collect();
        assert!(
            period > Duration::ZERO,
            "a keyframe timeline needs at least one keyframe with a non-zero hold"
        );
        Self { frames, period }
    }

    /// The keyframe value current `elapsed` into the looping animation, found
    /// by binary-searching the precomputed end offsets.
    pub fn value_at(&self, elapsed: Duration) -> &T {
        let into_loop = Duration::from_nanos((elapsed.as_nanos() % self.period.as_nanos()) as u64);
        let index = self.frames.partition_point(|(_, end)| *end <= into_loop);
        &self.frames[index].0
    }

    /// The values of every keyframe, in timeline order.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.frames.iter().map(|(value, _)| value)
    }
}

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
