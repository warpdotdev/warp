use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use winit::dpi::{LogicalPosition, LogicalSize};

use super::{ImeCursorArea, ImePositionRefreshReason, ImePositionState, ImePositionUpdate};
use crate::CursorInfo;

fn cursor(origin_x: f32, origin_y: f32, font_size: f32) -> CursorInfo {
    CursorInfo {
        position: RectF::new(vec2f(origin_x, origin_y), vec2f(0., font_size)),
        font_size,
    }
}

fn area(x: f32, y: f32, size: f32) -> ImeCursorArea {
    ImeCursorArea {
        position: LogicalPosition::new(x, y),
        size: LogicalSize::new(size, size),
    }
}

#[test]
fn from_cursor_info_places_area_below_caret_and_uses_font_size() {
    let info = cursor(10., 20., 16.);
    let derived = ImeCursorArea::from_cursor_info(&info);
    assert_eq!(derived, area(10., 20. + 1.2 * 16., 16.));
}

#[test]
fn plan_refresh_is_empty_when_ime_disabled() {
    let mut state = ImePositionState::default();
    assert!(!state.is_enabled());

    let updates = state.plan_refresh(area(1., 2., 12.), ImePositionRefreshReason::CursorMoved);
    assert!(updates.is_empty());
    assert_eq!(state.last_area(), None);
}

#[test]
fn plan_refresh_always_emits_cache_bust_then_real_position() {
    let mut state = ImePositionState::default();
    state.set_enabled(true);

    let target = area(40., 80., 14.);
    let updates = state.plan_refresh(target, ImePositionRefreshReason::CursorMoved);

    assert_eq!(
        updates,
        vec![
            ImePositionUpdate::Set(area(40., 81., 14.)),
            ImePositionUpdate::Set(target),
        ]
    );
    assert_eq!(state.last_area(), Some(target));
}

#[test]
fn plan_refresh_forces_sequence_for_geometry_changes_with_same_area() {
    let mut state = ImePositionState::default();
    state.set_enabled(true);

    let target = area(5., 10., 12.);
    let _ = state.plan_refresh(target, ImePositionRefreshReason::CursorMoved);

    for reason in [
        ImePositionRefreshReason::WindowMoved,
        ImePositionRefreshReason::WindowResized,
        ImePositionRefreshReason::ScaleFactorChanged,
        ImePositionRefreshReason::CursorMoved,
    ] {
        let updates = state.plan_refresh(target, reason);
        assert_eq!(
            updates,
            vec![
                ImePositionUpdate::Set(area(5., 11., 12.)),
                ImePositionUpdate::Set(target),
            ],
            "reason {reason:?} should still force a cache-bust sequence"
        );
        assert_eq!(state.last_area(), Some(target));
    }
}

#[test]
fn disabling_ime_clears_last_area() {
    let mut state = ImePositionState::default();
    state.set_enabled(true);
    let _ = state.plan_refresh(area(1., 2., 10.), ImePositionRefreshReason::CursorMoved);
    assert!(state.last_area().is_some());

    state.set_enabled(false);
    assert!(!state.is_enabled());
    assert_eq!(state.last_area(), None);
}
