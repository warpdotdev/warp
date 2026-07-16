//! Deterministic tests for the IME candidate-window position abstraction: the
//! geometry derived from the text cursor and the update sequence produced to
//! work around winit's position caching.

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use winit::dpi::{LogicalPosition, LogicalSize};

use super::{ImeCandidateGeometry, ImeCandidatePositionTracker};
use crate::CursorInfo;

/// Builds a candidate geometry at `(x, y)` with an arbitrary but fixed size, so
/// tests can focus on the position-driven update sequence.
fn geometry(x: f32, y: f32) -> ImeCandidateGeometry {
    ImeCandidateGeometry {
        position: LogicalPosition::new(x, y),
        size: LogicalSize::new(16.0, 16.0),
    }
}

#[test]
fn geometry_is_positioned_below_cursor_baseline() {
    let cursor = CursorInfo {
        position: RectF::new(vec2f(10.0, 20.0), vec2f(2.0, 18.0)),
        font_size: 15.0,
    };

    let geometry = ImeCandidateGeometry::from_cursor(&cursor);

    // The candidate window sits one-and-a-fifth font heights below the cursor
    // origin, and is a square the size of the font.
    assert_eq!(
        geometry.position,
        LogicalPosition::new(10.0, 20.0 + 1.2 * 15.0)
    );
    assert_eq!(geometry.size, LogicalSize::new(15.0, 15.0));
}

#[test]
fn first_update_is_forwarded_without_a_nudge() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let geo = geometry(5.0, 6.0);

    // No cached position yet, so a single update is enough.
    assert_eq!(tracker.updates_for(geo), vec![geo]);
}

#[test]
fn unchanged_position_is_nudged_to_bust_winit_cache() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let geo = geometry(5.0, 6.0);
    let _ = tracker.updates_for(geo);

    // Requesting the same position again (e.g. the window moved but the cursor
    // did not move relative to it) must emit a one-pixel nudge first so winit
    // re-forwards the position to the platform IME, then the real position.
    assert_eq!(tracker.updates_for(geo), vec![geometry(5.0, 7.0), geo]);
}

#[test]
fn changed_position_is_forwarded_without_a_nudge() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let _ = tracker.updates_for(geometry(5.0, 6.0));

    // A different position differs from winit's cache, so a single update
    // suffices.
    let moved = geometry(8.0, 9.0);
    assert_eq!(tracker.updates_for(moved), vec![moved]);
}

#[test]
fn generates_expected_sequence_across_repeated_and_changed_positions() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let a = geometry(1.0, 2.0);
    let b = geometry(3.0, 4.0);

    // Fresh position: single update.
    assert_eq!(tracker.updates_for(a), vec![a]);
    // Same position (e.g. window moved, cursor unchanged): nudge + real.
    assert_eq!(tracker.updates_for(a), vec![geometry(1.0, 3.0), a]);
    // New position: single update.
    assert_eq!(tracker.updates_for(b), vec![b]);
    // Back to the previous position, which differs from the last one we set, so
    // a single update is enough again.
    assert_eq!(tracker.updates_for(a), vec![a]);
}

#[test]
fn nudged_only_shifts_the_position_down_by_one_pixel() {
    let geo = geometry(12.0, 34.0);
    let nudged = geo.nudged();

    assert_eq!(nudged.position, LogicalPosition::new(12.0, 35.0));
    // The nudge must not change the size, only the position.
    assert_eq!(nudged.size, geo.size);
}
