use super::{TuiContentPoint, TuiSelectionHandle, TuiSelectionSpan};
use crate::text::SelectionType;

/// Verifies multiple row resizes are applied in original content order.
#[test]
fn batch_resize_rebases_selection_by_the_cumulative_delta() {
    let handle = TuiSelectionHandle::default();
    handle.start(
        TuiSelectionSpan {
            start: TuiContentPoint { row: 10, col: 0 },
            end: TuiContentPoint { row: 10, col: 1 },
        },
        Some(TuiSelectionSpan {
            start: TuiContentPoint { row: 11, col: 0 },
            end: TuiContentPoint { row: 12, col: 0 },
        }),
        SelectionType::Simple,
        10,
    );
    handle.finish();

    assert!(handle.rebase_for_row_resizes(vec![(5..6, 0), (1..3, 4)]));

    let range = handle.range().unwrap();
    assert_eq!(range.start.row, 11);
    assert_eq!(range.end.row, 13);
}
