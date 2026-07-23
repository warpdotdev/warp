use crate::{MouseButton, PointerEventKind, PointerSession, Vector2I};

#[test]
fn release_reuses_last_point_from_a_prior_press_and_move() {
    let session = PointerSession::new();
    // A press records the active button and last point.
    session.record_press_or_move(
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Vector2I::new(10, 20),
    );
    // A move updates the last point while the button is held.
    session.record_press_or_move(PointerEventKind::Move, None, Vector2I::new(30, 40));
    // A matching release returns the last point and clears the active button.
    assert_eq!(
        session.record_release(MouseButton::Left),
        Some(Vector2I::new(30, 40))
    );
    // A second release (button now cleared) returns None.
    assert_eq!(session.record_release(MouseButton::Left), None);
}

#[test]
fn release_for_a_different_button_is_ignored() {
    let session = PointerSession::new();
    session.record_press_or_move(
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Vector2I::new(1, 2),
    );
    // A Right release does not match the active Left press.
    assert_eq!(session.record_release(MouseButton::Right), None);
    // The Left press is still active, so a Left release still returns the point.
    assert_eq!(
        session.record_release(MouseButton::Left),
        Some(Vector2I::new(1, 2))
    );
}

#[test]
fn release_with_no_prior_press_is_ignored() {
    let session = PointerSession::new();
    assert_eq!(session.record_release(MouseButton::Left), None);
}

#[test]
fn clear_resets_active_press_so_a_later_release_is_ignored() {
    let session = PointerSession::new();
    session.record_press_or_move(
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Vector2I::new(5, 6),
    );
    // A failed/cancelled call clears the session so a later call cannot
    // inherit an abandoned press.
    session.clear();
    assert_eq!(session.record_release(MouseButton::Left), None);
}

#[test]
fn new_press_while_held_replaces_active_button() {
    let session = PointerSession::new();
    session.record_press_or_move(
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Vector2I::new(1, 1),
    );
    // A new press while a button is held replaces the active button; the
    // classifier closes the prior incomplete gesture as a held drag.
    session.record_press_or_move(
        PointerEventKind::Down,
        Some(MouseButton::Right),
        Vector2I::new(2, 2),
    );
    assert_eq!(session.record_release(MouseButton::Left), None);
    assert_eq!(
        session.record_release(MouseButton::Right),
        Some(Vector2I::new(2, 2))
    );
}

#[test]
fn move_without_press_records_point_but_no_release_matches() {
    let session = PointerSession::new();
    // A move with no active press updates the last point but sets no button,
    // so a following release does not match and is ignored.
    session.record_press_or_move(PointerEventKind::Move, None, Vector2I::new(9, 9));
    assert_eq!(session.record_release(MouseButton::Left), None);
}
