use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use winit::dpi::{LogicalPosition, LogicalSize};

use super::{ImeCandidateGeometry, ImeCandidatePositionTracker};
use crate::CursorInfo;

fn cursor_at(origin_x: f32, origin_y: f32, font_size: f32) -> CursorInfo {
    CursorInfo {
        position: RectF::new(vec2f(origin_x, origin_y), vec2f(1., font_size)),
        font_size,
    }
}

#[test]
fn geometry_places_candidate_window_below_cursor_baseline() {
    let geometry = ImeCandidateGeometry::from_cursor(&cursor_at(10., 20., 12.));

    assert_eq!(
        geometry.position,
        LogicalPosition::new(10., 20. + 1.2 * 12.)
    );
    // Size is computed even though X11 ignores it, so platforms that support sizing can use it.
    assert_eq!(geometry.size, LogicalSize::new(12., 12.));
}

#[test]
fn nudge_only_shifts_position_y() {
    let geometry = ImeCandidateGeometry {
        position: LogicalPosition::new(4., 8.),
        size: LogicalSize::new(16., 16.),
    };

    let nudged = geometry.nudged();
    assert_eq!(nudged.position, LogicalPosition::new(4., 9.));
    assert_eq!(nudged.size, geometry.size);
}

#[test]
fn first_update_is_a_single_geometry() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let geometry = ImeCandidateGeometry::from_cursor(&cursor_at(0., 0., 10.));

    let updates = tracker.updates_for(geometry);
    assert_eq!(updates, vec![geometry]);
}

#[test]
fn unchanged_position_emits_nudge_then_real_position() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let geometry = ImeCandidateGeometry::from_cursor(&cursor_at(5., 15., 14.));

    let _ = tracker.updates_for(geometry);
    let updates = tracker.updates_for(geometry);

    assert_eq!(updates, vec![geometry.nudged(), geometry]);
}

#[test]
fn changed_position_emits_single_update() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let first = ImeCandidateGeometry::from_cursor(&cursor_at(0., 0., 10.));
    let second = ImeCandidateGeometry::from_cursor(&cursor_at(20., 40., 10.));

    let _ = tracker.updates_for(first);
    let updates = tracker.updates_for(second);

    assert_eq!(updates, vec![second]);
}

#[test]
fn mixed_sequence_tracks_last_requested_position() {
    let mut tracker = ImeCandidatePositionTracker::default();
    let a = ImeCandidateGeometry::from_cursor(&cursor_at(1., 2., 8.));
    let b = ImeCandidateGeometry::from_cursor(&cursor_at(3., 4., 8.));

    assert_eq!(tracker.updates_for(a), vec![a]);
    // Same as last: nudge + real.
    assert_eq!(tracker.updates_for(a), vec![a.nudged(), a]);
    // Different: single update.
    assert_eq!(tracker.updates_for(b), vec![b]);
    // Back to a previous-but-not-last position is still a single update.
    assert_eq!(tracker.updates_for(a), vec![a]);
    // Repeat of last again needs a nudge.
    assert_eq!(tracker.updates_for(a), vec![a.nudged(), a]);
}
