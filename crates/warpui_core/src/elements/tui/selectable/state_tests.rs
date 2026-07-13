use super::{TuiGridPoint, TuiRowResize, TuiSelectionHandle, TuiSelectionSpan};
use crate::text::SelectionType;

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
