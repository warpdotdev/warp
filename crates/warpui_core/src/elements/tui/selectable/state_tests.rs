use std::collections::BTreeMap;

use super::{TuiGridPoint, TuiRowResize, TuiSelectionHandle, TuiSelectionSpan};
use crate::text::SelectionType;

/// Regression for drag-past-edge clearing: while a gesture is still active
/// (`is_selecting`), a cosmetic symbol change for a still-visible selected cell
/// (as produced by the auto-scroll re-render, or streaming content behind the
/// drag) must NOT clear the selection.
#[test]
fn snapshot_symbol_change_preserves_active_selection() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        Some(TuiSelectionSpan {
            start: TuiGridPoint { row: 1, col: 0 },
            end: TuiGridPoint { row: 2, col: 0 },
        }),
        SelectionType::Simple,
        10,
    );
    // The gesture is still in progress (no `finish()`): the user is dragging.
    assert!(handle.is_selecting());

    let mut first = BTreeMap::new();
    first.insert(TuiGridPoint { row: 0, col: 0 }, "a".to_owned());
    assert!(handle.validate_and_snapshot(first));
    assert!(handle.range().is_some());

    // Next frame (post auto-scroll): the same visible selected cell now renders
    // a different glyph. An active gesture must survive this.
    let mut changed = BTreeMap::new();
    changed.insert(TuiGridPoint { row: 0, col: 0 }, "b".to_owned());
    assert!(handle.validate_and_snapshot(changed));
    assert!(handle.range().is_some());
    assert!(handle.is_selecting());
}

/// A settled (finished) selection is still invalidated when a selected cell's
/// glyph changes, so a stale highlight never points at wrong content.
#[test]
fn snapshot_symbol_change_clears_settled_selection() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint { row: 0, col: 1 },
        },
        Some(TuiSelectionSpan {
            start: TuiGridPoint { row: 1, col: 0 },
            end: TuiGridPoint { row: 2, col: 0 },
        }),
        SelectionType::Simple,
        10,
    );
    handle.finish();

    let mut first = BTreeMap::new();
    first.insert(TuiGridPoint { row: 0, col: 0 }, "a".to_owned());
    assert!(handle.validate_and_snapshot(first));

    let mut changed = BTreeMap::new();
    changed.insert(TuiGridPoint { row: 0, col: 0 }, "b".to_owned());
    assert!(!handle.validate_and_snapshot(changed));
    assert!(handle.range().is_none());
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
