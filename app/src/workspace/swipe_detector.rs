//! Detects a two-finger horizontal trackpad swipe from a stream of scroll
//! events, for switching spaces from the vertical tabs panel.
//!
//! Pure logic with no UI: the panel feeds it the fields of
//! `Event::ScrollWheel`. `warpui` exposes no gesture phase (`NSEvent.phase` /
//! `momentumPhase`), so a gesture's end is inferred from a pause between
//! events. The momentum tail macOS sends after the fingers lift arrives as a
//! continuous stream, so it never produces that pause and stays inside the
//! same gesture.

use std::time::Duration;

use instant::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwipeDirection {
    Next,
    Prev,
}

/// Accumulated horizontal distance (scroll-delta units) that counts as a swipe.
pub const THRESHOLD: f32 = 60.0;

/// A pause at least this long between scroll events ends the current gesture.
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
    /// `precise` is false for mouse wheels, including Shift+wheel.
    pub fn feed(
        &mut self,
        dx: f32,
        dy: f32,
        precise: bool,
        now: Instant,
    ) -> Option<SwipeDirection> {
        if !precise {
            return None;
        }
        // Every precise event, vertical ones included, keeps the gesture alive,
        // so a diagonal momentum tail can't end the lock and fire twice.
        if self
            .last_event_at
            .is_some_and(|last| now.duration_since(last) >= IDLE_GAP)
        {
            self.accumulated_dx = 0.0;
            self.fired = false;
        }
        self.last_event_at = Some(now);
        if self.fired || dx.abs() <= dy.abs() {
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
