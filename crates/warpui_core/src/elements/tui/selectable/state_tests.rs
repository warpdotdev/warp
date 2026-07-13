use std::time::Duration;

use instant::Instant;

use super::{
    TuiGridPoint, TuiPaintContext, TuiPoint, TuiRect, TuiRowResize, TuiSelectionHandle,
    TuiSelectionSpan,
};
use crate::text::SelectionType;
use crate::EntityIdMap;

/// Repeated pointer updates on one edge preserve the cadence deadline instead
/// of making auto-scroll advance once per event.
#[test]
fn auto_scroll_deadline_survives_same_edge_updates() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        None,
        SelectionType::Simple,
        10,
    );
    let area = TuiRect::new(0, 2, 10, 4);
    let interval = Duration::from_millis(100);
    let now = Instant::now();

    assert!(handle.update_auto_scroll(TuiPoint::new(1, 7), area, now));
    let initial = handle.due_auto_scroll_target(now, interval).unwrap();
    assert_eq!(initial.started_at, now);
    let deadline = now + interval;

    assert!(!handle.update_auto_scroll(TuiPoint::new(3, 9), area, now + Duration::from_millis(10),));
    assert!(handle
        .due_auto_scroll_target(now + Duration::from_millis(99), interval)
        .is_none());
    let updated = handle.due_auto_scroll_target(deadline, interval).unwrap();
    assert_eq!(updated.position, TuiPoint::new(3, 9));
    assert_eq!(updated.started_at, now);
}

/// A frame that finishes after its cadence deadline leaves time for input
/// processing instead of requesting another immediate frame.
#[test]
fn overdue_auto_scroll_repaint_is_delayed_from_frame_completion() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        None,
        SelectionType::Simple,
        10,
    );
    let area = TuiRect::new(0, 0, 10, 4);
    let interval = Duration::from_millis(100);
    let armed_at = Instant::now();
    assert!(handle.update_auto_scroll(TuiPoint::new(1, 0), area, armed_at));
    assert!(handle.due_auto_scroll_target(armed_at, interval).is_some());

    let frame_completed_at = armed_at + Duration::from_millis(250);
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiPaintContext::new(&mut rendered_views);
    handle.request_auto_scroll_repaint(&mut ctx, interval, frame_completed_at);

    assert_eq!(
        ctx.requested_repaint_at(),
        Some(frame_completed_at + interval)
    );
}

/// Auto-scroll ramps with hold time and distance without exceeding half the viewport.
#[test]
fn auto_scroll_delta_accelerates_and_caps_to_half_the_viewport() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        None,
        SelectionType::Simple,
        10,
    );
    let area = TuiRect::new(0, 2, 10, 20);
    let now = Instant::now();

    assert!(handle.update_auto_scroll(TuiPoint::new(1, 22), area, now));
    let target = handle
        .due_auto_scroll_target(now, Duration::from_millis(100))
        .unwrap();

    assert_eq!(target.scroll_delta(now), Some(1));
    assert_eq!(
        target.scroll_delta(now + Duration::from_millis(500)),
        Some(2)
    );
    assert_eq!(
        target.scroll_delta(now + Duration::from_millis(1_500)),
        Some(4)
    );
    assert_eq!(
        target.scroll_delta(now + Duration::from_millis(3_000)),
        Some(8)
    );

    let far_below = TuiPoint::new(1, 100);
    assert!(!handle.update_auto_scroll(far_below, area, now));
    let target = handle
        .due_auto_scroll_target(now + Duration::from_millis(100), Duration::from_millis(100))
        .unwrap();
    assert_eq!(target.scroll_delta(now + Duration::from_secs(10)), Some(10));
}

/// Changing the parked edge starts a new cadence immediately, while finishing
/// the gesture clears it.
#[test]
fn auto_scroll_edge_change_resets_deadline_and_finish_disarms() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        None,
        SelectionType::Simple,
        10,
    );
    let area = TuiRect::new(0, 2, 10, 4);
    let interval = Duration::from_millis(100);
    let now = Instant::now();

    assert!(handle.update_auto_scroll(TuiPoint::new(1, 7), area, now));
    assert!(handle.due_auto_scroll_target(now, interval).is_some());

    let changed_at = now + Duration::from_millis(10);
    assert!(handle.update_auto_scroll(TuiPoint::new(1, 0), area, changed_at));
    let changed = handle.due_auto_scroll_target(changed_at, interval).unwrap();
    assert_eq!(changed.started_at, changed_at);
    assert_eq!(changed.scroll_delta(changed_at), Some(-2));

    handle.finish();
    assert!(handle
        .due_auto_scroll_target(changed_at + interval, interval)
        .is_none());
}

/// Extent updates distinguish movement from a repeated endpoint.
#[test]
fn update_extent_reports_whether_endpoint_changed() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        None,
        SelectionType::Simple,
        10,
    );
    let extent_span = TuiSelectionSpan {
        start: TuiGridPoint { row: 1, col: 0 },
        end: TuiGridPoint { row: 2, col: 0 },
    };

    assert!(handle.update_extent(extent_span));
    assert!(!handle.update_extent(extent_span));
}

/// Verifies multiple row resizes are applied in original content order.
#[test]
fn batch_resize_rebases_selection_by_the_cumulative_delta() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 10, col: 0 },
            end: TuiGridPoint { row: 10, col: 1 },
        },
        Some(TuiSelectionSpan {
            start: TuiGridPoint { row: 11, col: 0 },
            end: TuiGridPoint { row: 12, col: 0 },
        }),
        SelectionType::Simple,
        10,
    );
    handle.finish();

    assert!(handle.rebase_for_row_resizes(vec![
        TuiRowResize {
            old_rows: 5..6,
            new_height: 0,
        },
        TuiRowResize {
            old_rows: 1..3,
            new_height: 4,
        },
    ]));

    let range = handle.range().unwrap();
    assert_eq!(range.start.row, 11);
    assert_eq!(range.end.row, 13);
}

#[test]
fn batch_resize_without_selection_is_a_noop() {
    let handle = TuiSelectionHandle::default();

    assert!(!handle.rebase_for_row_resizes(vec![TuiRowResize {
        old_rows: 1..2,
        new_height: 3,
    }]));
}

#[test]
fn resize_below_selection_is_a_noop() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 2, col: 0 },
            end: TuiGridPoint { row: 2, col: 1 },
        },
        Some(TuiSelectionSpan {
            start: TuiGridPoint { row: 3, col: 0 },
            end: TuiGridPoint { row: 4, col: 0 },
        }),
        SelectionType::Simple,
        10,
    );
    handle.finish();

    assert!(!handle.rebase_for_row_resize(TuiRowResize {
        old_rows: 10..11,
        new_height: 2,
    }));
    assert_eq!(
        handle.range(),
        Some(TuiSelectionSpan {
            start: TuiGridPoint { row: 2, col: 0 },
            end: TuiGridPoint { row: 4, col: 0 },
        })
    );
}
