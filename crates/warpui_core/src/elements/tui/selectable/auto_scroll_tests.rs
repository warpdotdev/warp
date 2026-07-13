use std::time::Duration;

use instant::Instant;

use super::{TuiAutoScrollDragUpdate, TuiAutoScrollState, TuiPoint, TuiRect, AUTO_SCROLL_INTERVAL};

/// Same-edge pointer refreshes preserve the original cadence and hold duration.
#[test]
fn same_edge_refresh_preserves_cadence_and_start_time() {
    let mut state = TuiAutoScrollState::default();
    let area = TuiRect::new(0, 2, 10, 4);
    let now = Instant::now();

    assert_eq!(
        state.track_drag(TuiPoint::new(1, 7), area, now),
        TuiAutoScrollDragUpdate::Armed
    );
    assert_eq!(state.take_due_step(now).unwrap().rows, 1);

    assert_eq!(
        state.track_drag(TuiPoint::new(3, 9), area, now + Duration::from_millis(10)),
        TuiAutoScrollDragUpdate::Refreshed
    );
    assert!(state
        .take_due_step(now + AUTO_SCROLL_INTERVAL - Duration::from_millis(1))
        .is_none());
    let step = state.take_due_step(now + AUTO_SCROLL_INTERVAL).unwrap();
    assert_eq!(step.position, TuiPoint::new(3, 9));
    assert_eq!(step.rows, 2);
}

/// Scroll steps accelerate with hold duration and pointer overshoot.
#[test]
fn step_accelerates_with_hold_time_and_distance() {
    let mut state = TuiAutoScrollState::default();
    let area = TuiRect::new(0, 2, 10, 20);
    let now = Instant::now();
    state.track_drag(TuiPoint::new(1, 22), area, now);

    assert_eq!(state.take_due_step(now).unwrap().rows, 1);
    assert_eq!(
        state
            .take_due_step(now + Duration::from_millis(500))
            .unwrap()
            .rows,
        2
    );
    assert_eq!(
        state
            .take_due_step(now + Duration::from_millis(1_500))
            .unwrap()
            .rows,
        4
    );
    assert_eq!(
        state
            .take_due_step(now + Duration::from_millis(3_000))
            .unwrap()
            .rows,
        8
    );

    state.track_drag(TuiPoint::new(1, 100), area, now);
    assert_eq!(
        state
            .take_due_step(now + Duration::from_secs(10))
            .unwrap()
            .rows,
        10
    );
}

/// Direction changes rearm immediately and returning in bounds stops scrolling.
#[test]
fn edge_change_rearms_and_in_bounds_stops() {
    let mut state = TuiAutoScrollState::default();
    let area = TuiRect::new(0, 2, 10, 4);
    let now = Instant::now();

    state.track_drag(TuiPoint::new(1, 7), area, now);
    state.take_due_step(now);
    assert_eq!(
        state.track_drag(TuiPoint::new(1, 0), area, now + Duration::from_millis(10)),
        TuiAutoScrollDragUpdate::Armed
    );
    assert_eq!(
        state
            .take_due_step(now + Duration::from_millis(10))
            .unwrap()
            .rows,
        -2
    );

    assert_eq!(
        state.track_drag(TuiPoint::new(1, 3), area, now + Duration::from_millis(20)),
        TuiAutoScrollDragUpdate::InBounds
    );
    assert!(!state.is_active());
}
