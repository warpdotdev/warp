//! Backend-agnostic keyframe animation timing: a looping sequence of values,
//! each held for a fixed duration.
//!
//! Elements that animate through discrete frames (e.g. spinner glyphs)
//! describe their choreography as a [`KeyframeTimeline`] and ask it which
//! value is current for a given elapsed time, instead of hand-rolling frame
//! walking per element.

use std::time::Duration;

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
