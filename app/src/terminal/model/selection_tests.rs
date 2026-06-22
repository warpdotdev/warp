use super::*;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::grid::grid_handler::PerformResetGridChecks;
use crate::terminal::model::secrets::ObfuscateSecrets;
use crate::terminal::SizeInfo;

fn grid_handler(rows: usize, cols: usize) -> GridHandler {
    GridHandler::new(
        SizeInfo::new_without_font_metrics(rows, cols),
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        false,
        ObfuscateSecrets::No,
        PerformResetGridChecks::No,
    )
}

/// Test case of single cell selection.
///
/// 1. [  ]
/// 2. [B ]
/// 3. [BE]
#[test]
fn single_cell_left_to_right() {
    let location = Point { row: 0, col: 0 };
    let mut selection = Selection::new(SelectionType::Simple, location, Side::Left);
    selection.update(location, Side::Right);
    let semantic_selection = SemanticSelection::mock(false, "");

    assert_eq!(
        selection
            .to_range(&grid_handler(1, 2), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: location,
            end: location,
            is_reversed: false,
        }
    );
}

/// Test case of single cell selection.
///
/// 1. [  ]
/// 2. [ B]
/// 3. [EB]
#[test]
fn single_cell_right_to_left() {
    let location = Point { row: 0, col: 0 };
    let mut selection = Selection::new(SelectionType::Simple, location, Side::Right);
    selection.update(location, Side::Left);
    let semantic_selection = SemanticSelection::mock(false, "");

    assert_eq!(
        selection
            .to_range(&grid_handler(1, 2), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: location,
            end: location,
            is_reversed: true,
        }
    );
}

/// Test selection across adjacent lines.
///
/// 1.  [  ][  ][  ][  ][  ]
///     [  ][  ][  ][  ][  ]
/// 2.  [  ][ B][  ][  ][  ]
///     [  ][  ][  ][  ][  ]
/// 3.  [  ][ B][XX][XX][XX]
///     [XX][XE][  ][  ][  ]
#[test]
fn across_adjacent_lines() {
    let mut selection = Selection::new(SelectionType::Simple, Point::new(0, 1), Side::Right);
    selection.update(Point::new(1, 1), Side::Right);
    let semantic_selection = SemanticSelection::mock(false, "");

    assert_eq!(
        selection
            .to_range(&grid_handler(2, 5), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point::new(0, 2),
            end: Point::new(1, 1),
            is_reversed: false,
        }
    );
}

/// Test selection across adjacent lines.
///
/// 1.  [  ][  ][  ][  ][  ]
///     [  ][  ][  ][  ][  ]
/// 2.  [  ][  ][  ][  ][  ]
///     [  ][ B][  ][  ][  ]
/// 3.  [  ][ E][XX][XX][XX]
///     [XX][XB][  ][  ][  ]
/// 4.  [ E][XX][XX][XX][XX]
///     [XX][XB][  ][  ][  ]
#[test]
fn selection_bigger_then_smaller() {
    let mut selection = Selection::new(SelectionType::Simple, Point::new(1, 1), Side::Right);
    selection.update(Point::new(0, 1), Side::Right);
    let semantic_selection = SemanticSelection::mock(false, "");

    assert_eq!(
        selection
            .to_range(&grid_handler(2, 5), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point::new(0, 2),
            end: Point::new(1, 1),
            is_reversed: true,
        }
    );
}

#[test]
fn simple_is_empty() {
    let mut selection = Selection::new(SelectionType::Simple, Point::new(0, 0), Side::Right);
    assert!(selection.is_empty());
    selection.update(Point::new(0, 1), Side::Left);
    assert!(selection.is_empty());
    selection.update(Point::new(1, 0), Side::Right);
    assert!(!selection.is_empty());
}

#[test]
fn simple_max_min_column() {
    // If the selection starts on the Right side of a cell, it should skip that cell
    let mut selection = Selection::new(SelectionType::Simple, Point::new(0, 0), Side::Right);
    selection.update(Point::new(0, 2), Side::Right);
    let semantic_selection = SemanticSelection::mock(false, "");
    assert_eq!(
        selection
            .to_range(&grid_handler(3, 3), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 0, col: 1 },
            end: Point { row: 0, col: 2 },
            is_reversed: false,
        }
    );

    // If the selection starts on the Right side of the last column, it should span that column
    let mut selection = Selection::new(SelectionType::Simple, Point::new(0, 4), Side::Right);
    selection.update(Point::new(1, 2), Side::Right);
    assert_eq!(
        selection
            .to_range(&grid_handler(5, 5), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 1, col: 0 },
            end: Point { row: 1, col: 2 },
            is_reversed: false,
        }
    );

    // If the selection ends on the Left side of a cell, it should remove that cell
    let mut selection = Selection::new(SelectionType::Simple, Point::new(0, 0), Side::Left);
    selection.update(Point::new(2, 2), Side::Left);
    assert_eq!(
        selection
            .to_range(&grid_handler(5, 3), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 0, col: 0 },
            end: Point { row: 2, col: 1 },
            is_reversed: false,
        }
    );

    // If the selection ends on the Left side of the first column, it should span that column
    let mut selection = Selection::new(SelectionType::Simple, Point::new(0, 0), Side::Left);
    selection.update(Point::new(2, 0), Side::Left);
    assert_eq!(
        selection
            .to_range(&grid_handler(5, 3), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 0, col: 0 },
            end: Point { row: 1, col: 2 },
            is_reversed: false,
        }
    );
}

#[test]
fn test_selection_rotate_up() {
    let range = VisibleRow(0)..VisibleRow(30);
    let semantic_selection = SemanticSelection::mock(false, "");
    let mut selection = Selection::new(SelectionType::Simple, Point::new(10, 3), Side::Left);
    selection.update(Point::new(20, 56), Side::Right);

    selection = selection
        .rotate(&range, ScrollDelta::Up { lines: 1 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 9, col: 3 },
            end: Point { row: 19, col: 56 },
            is_reversed: false,
        }
    );

    selection = selection
        .rotate(&range, ScrollDelta::Up { lines: 5 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 4, col: 3 },
            end: Point { row: 14, col: 56 },
            is_reversed: false,
        }
    );

    // This causes the selection range to go past the top.
    selection = selection
        .rotate(&range, ScrollDelta::Up { lines: 5 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 0, col: 0 },
            end: Point { row: 9, col: 56 },
            is_reversed: false,
        }
    );

    selection = selection
        .rotate(&range, ScrollDelta::Down { lines: 1 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 1, col: 0 },
            end: Point { row: 10, col: 56 },
            is_reversed: false,
        }
    );

    selection = selection
        .rotate(&range, ScrollDelta::Up { lines: 6 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 0, col: 0 },
            end: Point { row: 4, col: 56 },
            is_reversed: false,
        }
    );

    // The end of the range passes the top, so the selection should go away.
    assert_eq!(
        selection.rotate(&range, ScrollDelta::Up { lines: 6 }, 60),
        None
    );
}

#[test]
fn test_selection_rotate_down() {
    let range = VisibleRow(0)..VisibleRow(30);
    let semantic_selection = SemanticSelection::mock(false, "");
    let mut selection = Selection::new(SelectionType::Simple, Point::new(10, 3), Side::Left);
    selection.update(Point::new(20, 56), Side::Right);

    selection = selection
        .rotate(&range, ScrollDelta::Down { lines: 1 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 11, col: 3 },
            end: Point { row: 21, col: 56 },
            is_reversed: false,
        }
    );

    selection = selection
        .rotate(&range, ScrollDelta::Down { lines: 5 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 16, col: 3 },
            end: Point { row: 26, col: 56 },
            is_reversed: false,
        }
    );

    // This scrolls the end of the selection range passed the bottom.
    selection = selection
        .rotate(&range, ScrollDelta::Down { lines: 5 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 21, col: 3 },
            end: Point { row: 29, col: 59 },
            is_reversed: false,
        }
    );

    selection = selection
        .rotate(&range, ScrollDelta::Up { lines: 1 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 20, col: 3 },
            end: Point { row: 28, col: 59 },
            is_reversed: false,
        }
    );

    selection = selection
        .rotate(&range, ScrollDelta::Down { lines: 6 }, 60)
        .expect("should still have a selection");
    assert_eq!(
        selection
            .to_range(&grid_handler(30, 60), &semantic_selection)
            .unwrap(),
        SelectionRange {
            start: Point { row: 26, col: 3 },
            end: Point { row: 29, col: 59 },
            is_reversed: false,
        }
    );

    assert_eq!(
        selection.rotate(&range, ScrollDelta::Down { lines: 6 }, 60),
        None
    );
}
