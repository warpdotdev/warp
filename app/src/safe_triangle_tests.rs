use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::*;

#[test]
fn no_target_rect_does_not_suppress() {
    let mut tri = SafeTriangle::new();
    assert!(!tri.should_suppress_hover(Vector2F::new(100.0, 100.0)));
}

#[test]
fn no_last_position_does_not_suppress() {
    let mut tri = SafeTriangle::new();
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    assert!(!tri.should_suppress_hover(Vector2F::new(150.0, 100.0)));
}

#[test]
fn point_inside_triangle_is_suppressed() {
    let mut tri = SafeTriangle::new();
    // Target sidecar to the right
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(100.0, 150.0));

    // A point roughly between the cursor and the left edge of the target
    assert!(tri.should_suppress_hover(Vector2F::new(150.0, 140.0)));
}

#[test]
fn point_outside_triangle_is_not_suppressed() {
    let mut tri = SafeTriangle::new();
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(100.0, 150.0));

    // A point far above the triangle
    assert!(!tri.should_suppress_hover(Vector2F::new(100.0, 10.0)));
}

#[test]
fn clearing_target_rect_clears_state() {
    let mut tri = SafeTriangle::new();
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(100.0, 150.0));
    tri.set_target_rect(None);
    assert!(!tri.should_suppress_hover(Vector2F::new(150.0, 140.0)));
}

#[test]
fn sidecar_to_the_left() {
    let mut tri = SafeTriangle::new();
    // Target sidecar to the left of cursor
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(0.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(200.0, 150.0));

    // Point between cursor and the right edge of target (near-side)
    assert!(tri.should_suppress_hover(Vector2F::new(150.0, 150.0)));
}

#[test]
fn same_position_not_suppressed() {
    let mut tri = SafeTriangle::new();
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(150.0, 140.0));

    // Same position should never be suppressed (degenerate triangle)
    assert!(!tri.should_suppress_hover(Vector2F::new(150.0, 140.0)));
}

#[test]
fn vertical_movement_not_suppressed_with_tall_sidecar() {
    let mut tri = SafeTriangle::new();
    // Very tall sidecar to the right
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 0.0),
        Vector2F::new(100.0, 500.0),
    )));
    tri.update_position(Vector2F::new(100.0, 250.0));

    // Moving mostly vertically with slight jitter should NOT be suppressed
    assert!(!tri.should_suppress_hover(Vector2F::new(102.0, 220.0)));
    assert!(!tri.should_suppress_hover(Vector2F::new(103.0, 280.0)));
}

#[test]
fn leaving_after_entering_target_is_not_suppressed() {
    let mut tri = SafeTriangle::new();
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(250.0, 140.0));

    assert!(!tri.should_suppress_hover(Vector2F::new(320.0, 140.0)));
}

#[test]
fn duplicate_event_at_same_position_preserves_existing_suppression() {
    let mut tri = SafeTriangle::new();
    tri.set_target_rect(Some(RectF::new(
        Vector2F::new(200.0, 50.0),
        Vector2F::new(100.0, 200.0),
    )));
    tri.update_position(Vector2F::new(100.0, 150.0));

    assert!(tri.should_suppress_hover(Vector2F::new(150.0, 140.0)));
    tri.update_position(Vector2F::new(150.0, 140.0));
    assert!(tri.should_suppress_hover(Vector2F::new(150.0, 140.0)));
}
