//! Detects a two-finger horizontal trackpad swipe from a stream of scroll
//! events, for switching spaces from the vertical tabs panel.
//!
//! Pure logic with no UI: the panel feeds it the fields of
//! `Event::ScrollWheel`. With a gesture phase (macOS trackpads), `Began`
//! starts a new gesture and momentum never counts, so a swipe made while the
//! previous one's momentum tail is still streaming switches again. Without
//! one, a gesture's end is inferred from a pause between events; the momentum
//! tail arrives as a continuous stream, so it never produces that pause and
//! stays inside the same gesture.

use std::time::Duration;

use instant::Instant;
use warpui::event::ScrollPhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDirection {
    Next,
    Prev,
}

/// Accumulated horizontal distance (scroll-delta units) that counts as a swipe.
pub const THRESHOLD: f32 = 60.0;

/// A pause at least this long between scroll events without a phase ends the
/// current gesture.
pub const IDLE_GAP: Duration = Duration::from_millis(200);

#[derive(Debug, Default)]
pub struct SwipeDetector {
    /// Horizontal distance accumulated in the current gesture.
    accumulated_dx: f32,
    /// Set once the current gesture has produced a direction.
    fired: bool,
    /// When the previous scroll event arrived.
    last_event_at: Option<Instant>,
}

impl SwipeDetector {
    /// Feeds one scroll event and returns a direction at most once per gesture.
    ///
    /// `dx` / `dy` are `scrollingDeltaX/Y`, which already follow the system
    /// scroll-direction setting. Fingers moving left produce a negative `dx`
    /// and mean [`SwipeDirection::Next`], like browser back/forward swipes.
    /// `precise` is false for mouse wheels, including Shift+wheel. `phase` is
    /// `None` when the platform or device doesn't report one.
    pub fn feed(
        &mut self,
        dx: f32,
        dy: f32,
        precise: bool,
        phase: Option<ScrollPhase>,
        now: Instant,
    ) -> Option<SwipeDirection> {
        if !precise {
            return None;
        }
        let starts_gesture = match phase {
            Some(phase) => phase == ScrollPhase::Began,
            // Every precise event, vertical ones included, keeps the gesture
            // alive, so a diagonal momentum tail can't end the lock and fire twice.
            None => self
                .last_event_at
                .is_some_and(|last| now.duration_since(last) >= IDLE_GAP),
        };
        self.last_event_at = Some(now);
        if starts_gesture {
            self.accumulated_dx = 0.0;
            self.fired = false;
        }
        // Only the fingers swipe: momentum keeps coming after they lift.
        if phase == Some(ScrollPhase::Momentum) || self.fired || dx.abs() <= dy.abs() {
            return None;
        }
        // ponytail: reversals subtract rather than reset; reset-on-sign-change if it feels laggy.
        self.accumulated_dx += dx;
        if self.accumulated_dx.abs() < THRESHOLD {
            return None;
        }
        self.fired = true;
        Some(if self.accumulated_dx < 0.0 {
            SwipeDirection::Next
        } else {
            SwipeDirection::Prev
        })
    }
}

#[cfg(test)]
#[path = "swipe_detector_tests.rs"]
mod tests;
