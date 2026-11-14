//! Regression tests for position-aware multi-click detection.
//!
//! See <https://github.com/warpdotdev/warp/issues/15706>: quickly clicking two different tab rows
//! (different position, same button, within [`MULTI_CLICK_INTERVAL`]) was incorrectly treated as
//! a double-click on the second row, since click-count detection only considered button and
//! timing, unlike macOS's native `NSEvent.clickCount`, which also resets on excessive movement.

use pathfinder_geometry::vector::vec2f;
use winit::event::MouseButton;

use super::*;

#[test]
fn test_repeated_click_at_same_position_increments_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let position = vec2f(10., 20.);

    let first =
        window_state.determine_click_count_and_update_button_state(MouseButton::Left, position);
    let second =
        window_state.determine_click_count_and_update_button_state(MouseButton::Left, position);
    let third =
        window_state.determine_click_count_and_update_button_state(MouseButton::Left, position);

    assert_eq!(first, 1);
    assert_eq!(second, 2);
    assert_eq!(third, 3);
}

#[test]
fn test_click_within_multi_click_distance_still_increments_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    // Small jitter, well within `MULTI_CLICK_DISTANCE`, that should still count as the same spot.
    let second_position = first_position + vec2f(1., 1.);

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(second, 2);
}

#[test]
fn test_click_exactly_at_axis_boundary_still_increments_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    // Exactly `MULTI_CLICK_DISTANCE` away on each axis: the check is inclusive (`<=`).
    let second_position = first_position + vec2f(MULTI_CLICK_DISTANCE, -MULTI_CLICK_DISTANCE);

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(second, 2);
}

#[test]
fn test_click_just_past_axis_boundary_on_a_single_axis_resets_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    // Just past the boundary on the x-axis only; y is unchanged.
    let second_position = first_position + vec2f(MULTI_CLICK_DISTANCE + 0.1, 0.);

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(
        second, 1,
        "exceeding the bound on a single axis must reset the click count"
    );
}

#[test]
fn test_diagonal_click_within_axis_bounds_on_both_axes_still_counts_as_multi_click() {
    // The check is axis-aligned (an independent bound per axis, like `MAX_TAP_DISTANCE` and the
    // OS metrics it approximates), not a circular radius. A diagonal move can therefore have a
    // Euclidean distance greater than `MULTI_CLICK_DISTANCE` while still passing, as long as
    // each axis individually stays within the bound.
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    let second_position = first_position + vec2f(MULTI_CLICK_DISTANCE, MULTI_CLICK_DISTANCE);
    assert!(
        (second_position - first_position).length() > MULTI_CLICK_DISTANCE,
        "this test only demonstrates the intended behavior if the diagonal distance exceeds \
         MULTI_CLICK_DISTANCE"
    );

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(second, 2);
}

#[test]
fn test_click_on_sufficiently_separated_position_resets_click_count() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    // Simulates quickly clicking a different tab row well outside `MULTI_CLICK_DISTANCE`, even
    // though it happens within `MULTI_CLICK_INTERVAL` of the first click.
    let second_position = first_position + vec2f(0., 40.);

    let first = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    let second = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(first, 1);
    assert_eq!(
        second, 1,
        "a click far from the previous one must not be counted as a double-click"
    );
}

#[test]
fn test_click_after_separated_click_can_start_a_new_multi_click_sequence() {
    let mut window_state = WindowState::new(crate::WindowId::new());
    let first_position = vec2f(10., 20.);
    let second_position = first_position + vec2f(0., 40.);

    window_state.determine_click_count_and_update_button_state(MouseButton::Left, first_position);
    window_state.determine_click_count_and_update_button_state(MouseButton::Left, second_position);
    // A third click at the same (new) position as the second click should count as the start of
    // a fresh multi-click sequence at that position, i.e. a double-click there.
    let third = window_state
        .determine_click_count_and_update_button_state(MouseButton::Left, second_position);

    assert_eq!(third, 2);
}
