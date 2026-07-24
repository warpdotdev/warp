//! Tests for the Grid.

use grid_handler::GridHandler;
use warp_terminal::model::grid::cell;

use super::*;
use crate::features::FeatureFlag;
use crate::terminal::SizeInfo;
use crate::terminal::model::ansi::Handler;
use crate::terminal::model::cell::{Cell, Flags};
use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::index::{Point, VisiblePoint, VisibleRow};
use crate::terminal::model::secrets::ObfuscateSecrets;

// Scroll up moves lines upward.
#[test]
fn scroll_up() {
    let index_to_char = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

    let mut grid = GridStorage::new(10, 1, 0, ObfuscateSecrets::No);
    for i in 0..10 {
        grid[i][0] = cell(index_to_char[i]);
    }

    grid.scroll_up(&(VisibleRow(0)..VisibleRow(10)), 2);

    assert_eq!(grid[0][0], cell('2'));
    assert_eq!(grid[0].occ, 1);
    assert_eq!(grid[1][0], cell('3'));
    assert_eq!(grid[1].occ, 1);
    assert_eq!(grid[2][0], cell('4'));
    assert_eq!(grid[2].occ, 1);
    assert_eq!(grid[3][0], cell('5'));
    assert_eq!(grid[3].occ, 1);
    assert_eq!(grid[4][0], cell('6'));
    assert_eq!(grid[4].occ, 1);
    assert_eq!(grid[5][0], cell('7'));
    assert_eq!(grid[5].occ, 1);
    assert_eq!(grid[6][0], cell('8'));
    assert_eq!(grid[6].occ, 1);
    assert_eq!(grid[7][0], cell('9'));
    assert_eq!(grid[7].occ, 1);
    assert_eq!(grid[8][0], Cell::default()); // was 0.
    assert_eq!(grid[8].occ, 0);
    assert_eq!(grid[9][0], Cell::default()); // was 1.
    assert_eq!(grid[9].occ, 0);
}

#[test]
fn scroll_up_with_scrollback() {
    let index_to_char = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

    let mut grid = GridStorage::new(10, 1, 10, ObfuscateSecrets::No);
    for i in 0..10 {
        grid[i][0] = cell(index_to_char[i]);
    }

    grid.scroll_up(&(VisibleRow(0)..VisibleRow(10)), 2);

    let history_size = grid.history_size();

    assert_eq!(grid[history_size][0], cell('2'));
    assert_eq!(grid[history_size].occ, 1);
    assert_eq!(grid[history_size + 1][0], cell('3'));
    assert_eq!(grid[history_size + 1].occ, 1);
    assert_eq!(grid[history_size + 2][0], cell('4'));
    assert_eq!(grid[history_size + 2].occ, 1);
    assert_eq!(grid[history_size + 3][0], cell('5'));
    assert_eq!(grid[history_size + 3].occ, 1);
    assert_eq!(grid[history_size + 4][0], cell('6'));
    assert_eq!(grid[history_size + 4].occ, 1);
    assert_eq!(grid[history_size + 5][0], cell('7'));
    assert_eq!(grid[history_size + 5].occ, 1);
    assert_eq!(grid[history_size + 6][0], cell('8'));
    assert_eq!(grid[history_size + 6].occ, 1);
    assert_eq!(grid[history_size + 7][0], cell('9'));
    assert_eq!(grid[history_size + 7].occ, 1);
    assert_eq!(grid[history_size + 8][0], Cell::default()); // was 0.
    assert_eq!(grid[history_size + 8].occ, 0);
    assert_eq!(grid[history_size + 9][0], Cell::default()); // was 1.
    assert_eq!(grid[history_size + 9].occ, 0);
}

// Scroll down moves lines downward.
#[test]
fn scroll_down() {
    let index_to_char = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];
    let mut grid = GridStorage::new(10, 1, 0, ObfuscateSecrets::No);
    for i in 0..10 {
        grid[i][0] = index_to_char[i].into();
    }

    grid.scroll_down(&(VisibleRow(0)..VisibleRow(10)), 2);

    assert_eq!(grid[0][0], Cell::default()); // was 8.
    assert_eq!(grid[0].occ, 0);
    assert_eq!(grid[1][0], Cell::default()); // was 9.
    assert_eq!(grid[1].occ, 0);
    assert_eq!(grid[2][0], cell('0'));
    assert_eq!(grid[2].occ, 1);
    assert_eq!(grid[3][0], cell('1'));
    assert_eq!(grid[3].occ, 1);
    assert_eq!(grid[4][0], cell('2'));
    assert_eq!(grid[4].occ, 1);
    assert_eq!(grid[5][0], cell('3'));
    assert_eq!(grid[5].occ, 1);
    assert_eq!(grid[6][0], cell('4'));
    assert_eq!(grid[6].occ, 1);
    assert_eq!(grid[7][0], cell('5'));
    assert_eq!(grid[7].occ, 1);
    assert_eq!(grid[8][0], cell('6'));
    assert_eq!(grid[8].occ, 1);
    assert_eq!(grid[9][0], cell('7'));
    assert_eq!(grid[9].occ, 1);
}

#[test]
fn shrink_reflow() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, 2);
    grid.input_at_cursor("12345");

    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert_eq!(grid.total_rows(), 3);

    let row = grid.row(0).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], wrap_cell('2'));

    let row = grid.row(1).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('3'));
    assert_eq!(row[1], wrap_cell('4'));

    let row = grid.row(2).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('5'));
    assert_eq!(row[1], Cell::default());
}

/// Tests shrinking grid with reflow, including a start of command marker on an empty cell.
#[test]
fn shrink_reflow_with_start_of_command_marker() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, 2);
    grid.input_at_cursor("1234");
    grid.grid_storage_mut()
        .cursor_cell()
        .mark_end_of_prompt(false);

    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert_eq!(grid.total_rows(), 3);

    let row = grid.row(0).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], wrap_cell('2'));

    let row = grid.row(1).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('3'));
    assert_eq!(row[1], wrap_cell('4'));

    let row = grid.row(2).unwrap();
    assert_eq!(row.len(), 2);
    // We expect the start of command marker to be preserved in the reflow, even with an empty cell!
    assert_eq!(row[0], cell_with_end_of_prompt_marker('\0', false));
    assert_eq!(row[1], Cell::default());
}

#[test]
fn shrink_reflow_twice() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, 2);
    grid.input_at_cursor("12345");

    grid.resize(SizeInfo::new_without_font_metrics(1, 4));
    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert_eq!(grid.total_rows(), 3);

    let row = grid.row(0).expect("row should exist");
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], wrap_cell('2'));

    let row = grid.row(1).expect("row should exist");
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('3'));
    assert_eq!(row[1], wrap_cell('4'));

    let row = grid.row(2).expect("row should exist");
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('5'));
    assert_eq!(row[1], Cell::default());
}

/// Regression test for bug where cursor position was not correctly reflowed after a resize.
#[test]
fn shrink_grow_reflow_cursor_position_saturation() {
    let _flag = FeatureFlag::ResizeFix.override_enabled(true);

    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 8, 2);
    grid.input_at_cursor("12345");
    grid.set_cursor_point(0, 5);

    // Assert the absolute point matches the visible cursor point at the start.
    assert_eq!(grid.cursor_point(), Point::new(0, 5));

    // Causes a shrink_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(3, 3));

    // Absolute position is (1, 2). Visible point is (0, 2) since 1 row is scrollback.
    assert_eq!(grid.cursor_point(), Point::new(1, 2));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(0));
    assert_eq!(grid.grid_storage().cursor.point.col, 2);

    // Causes a grow_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(3, 8));

    // At this point, we expect the absolute and relative points to be matching + restored to (0, 5) i.e. the original cursor position.
    // We should NOT saturate the subtraction of the cursor position at (0, 0).
    assert_eq!(grid.cursor_point(), Point::new(0, 5));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(0));
    assert_eq!(grid.grid_storage().cursor.point.col, 5);
}

/// Regression test for crash with cursor reflow fix (overflow on subtraction resulting from
/// 1 row being added to the cursor position but not removed, meaning we had an invalid position).
#[test]
fn shrink_grow_reflow_cursor_position_multiple_grows_same_line_cursor_adjust() {
    let _flag = FeatureFlag::ResizeFix.override_enabled(true);

    let mut grid = GridHandler::new_for_test_with_scroll_limit(10, 80, 10);
    // First row.
    grid.input_at_cursor("12345");
    // Second row.
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("12345");

    // Set the cursor to an empty cell somewhere in the middle of the final
    // row.
    grid.set_cursor_point(9, 30);

    assert_eq!(grid.cursor_point(), Point::new(9, 30));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(9));
    assert_eq!(grid.grid_storage().cursor.point.col, 30);

    // Causes a shrink_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(10, 8));
    // Cursor is at column index 30 (31 cells), wrapped down to a width of 8
    // columns.
    // 31 / 8 = 3 full rows + a partial row w/ the cursor over the 7th cell (31 % 8 == 7).
    // Row 9 becomes row 12 (added two full rows and 1 partial row).
    assert_eq!(grid.cursor_point(), Point::new(12, 6));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(9));
    assert_eq!(grid.grid_storage().cursor.point.col, 6);

    // Causes a grow_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(10, 10));
    assert_eq!(grid.cursor_point(), Point::new(11, 9));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(9));
    assert_eq!(grid.grid_storage().cursor.point.col, 9);
    assert!(grid.grid_storage().cursor.input_needs_wrap);

    // Causes a grow_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(10, 12));
    assert_eq!(grid.cursor_point(), Point::new(11, 6));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(9));
    assert_eq!(grid.grid_storage().cursor.point.col, 6);

    // Causes a grow_cols call. Ensure that this does NOT panic due to subtraction overflow.
    grid.resize(SizeInfo::new_without_font_metrics(10, 14));
    assert_eq!(grid.cursor_point(), Point::new(11, 2));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(9));
    assert_eq!(grid.grid_storage().cursor.point.col, 2);

    // Resize the grid back to its initial dimensions, which should restore the
    // cursor to its initial position.
    grid.resize(SizeInfo::new_without_font_metrics(10, 80));
    assert_eq!(grid.cursor_point(), Point::new(9, 30));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(9));
    assert_eq!(grid.grid_storage().cursor.point.col, 30);
}

/// Confirm the cursor position is reflowed correctly after a resize for a non-zero row restoration.
#[test]
fn shrink_grow_reflow_cursor_position_non_zero_row() {
    let _flag = FeatureFlag::ResizeFix.override_enabled(true);

    let mut grid = GridHandler::new_for_test_with_scroll_limit(8, 8, 6);
    // First row.
    grid.input_at_cursor("12345");
    // Second row.
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("12345");
    // Third row.
    grid.set_cursor_point(2, 0);
    grid.input_at_cursor("12345");

    grid.set_cursor_point(2, 5);

    // Assert the absolute point matches the visible cursor point at the start.
    assert_eq!(grid.cursor_point(), Point::new(2, 5));

    // Causes a shrink_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(8, 3));

    // Absolute position should be (5, 2) i.e. cursor reflowed to the 6th row. Visible point is (2, 2) since 3 rows are scrollback.
    assert_eq!(grid.cursor_point(), Point::new(5, 2));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(2));
    assert_eq!(grid.grid_storage().cursor.point.col, 2);

    // Causes a grow_cols call.
    grid.resize(SizeInfo::new_without_font_metrics(3, 8));

    // At this point, we expect the absolute and relative points to be matching + restored to (2, 5) i.e. the original cursor position.
    assert_eq!(grid.cursor_point(), Point::new(2, 5));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(2));
    assert_eq!(grid.grid_storage().cursor.point.col, 5);
}

#[test]
fn multiple_resizes_cursor_position_restoration() {
    let _flag = FeatureFlag::ResizeFix.override_enabled(true);

    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 10, 5);
    {
        let grid = grid.grid_storage_mut();
        // Populate the first row with incremental values.
        for col in 0..10 {
            // ASCII A-J as characters.
            grid[0][col] = cell((col + 65) as u8 as char);
        }
    }
    grid.set_cursor_point(0, 7);

    // First resize: Shrink columns.
    grid.resize(SizeInfo::new_without_font_metrics(4, 5));
    assert_eq!(grid.cursor_point(), Point::new(1, 2));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(0));
    assert_eq!(grid.grid_storage().cursor.point.col, 2);

    // Second resize: Shrink rows.
    grid.resize(SizeInfo::new_without_font_metrics(2, 5));
    assert_eq!(grid.cursor_point(), Point::new(1, 2));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(0));
    assert_eq!(grid.grid_storage().cursor.point.col, 2);

    // Third resize: Grow columns back.
    grid.resize(SizeInfo::new_without_font_metrics(2, 10));
    // Cursor should be restored back to the original position.
    assert_eq!(grid.cursor_point(), Point::new(0, 7));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(0));
    assert_eq!(grid.grid_storage().cursor.point.col, 7);

    // Fourth resize: Grow rows back.
    grid.resize(SizeInfo::new_without_font_metrics(4, 10));
    // Assert the cursor remains at the original position.
    assert_eq!(grid.cursor_point(), Point::new(0, 7));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(0));
    assert_eq!(grid.grid_storage().cursor.point.col, 7);
}

#[test]
fn non_sequential_resizes_cursor_restoration() {
    let _flag = FeatureFlag::ResizeFix.override_enabled(true);

    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 5, 10);
    {
        // Populate the grid with some data
        let grid = grid.grid_storage_mut();
        for row in 0..5 {
            for col in 0..5 {
                grid[row][col] = cell((65 + row * 5 + col) as u8 as char); // ASCII characters
            }
        }
    }
    grid.set_cursor_point(3, 4);

    // Non-sequential resizes
    grid.resize(SizeInfo::new_without_font_metrics(3, 3)); // Shrink
    grid.resize(SizeInfo::new_without_font_metrics(2, 4)); // Shrink width, grow height
    grid.resize(SizeInfo::new_without_font_metrics(4, 2)); // Grow width, shrink height
    grid.resize(SizeInfo::new_without_font_metrics(5, 5)); // Restore to original

    // Check cursor position is restored or correctly adjusted
    assert_eq!(grid.grid_storage().cursor_point(), Point::new(3, 4));
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(3));
    assert_eq!(grid.grid_storage().cursor.point.col, 4);
}

#[test]
fn shrink_reflow_empty_cell_inside_line() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(1, 5, 3);
    grid.input_at_cursor("1 34 ");

    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert_eq!(grid.total_rows(), 3);

    let row = grid.row(0).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], wrap_cell(' '));

    let row = grid.row(1).unwrap();
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('3'));
    assert_eq!(row[1], wrap_cell('4'));
}

#[test]
fn grow_reflow() {
    let mut grid = GridHandler::new_for_test(2, 2);
    grid.input_at_cursor("123 ");

    assert_eq!(grid.cursor_point(), Point::new(1, 1));

    grid.resize(SizeInfo::new_without_font_metrics(2, 3));

    assert_eq!(grid.total_rows(), 2);
    assert_eq!(grid.cursor_point(), Point::new(1, 0));

    let row = grid.row(0).expect("row should exist");
    assert_eq!(row.len(), 3);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], cell('2'));
    assert_eq!(row[2], wrap_cell('3'));

    // Make sure rest of grid is empty.
    let row = grid.row(1).expect("row should exist");
    assert_eq!(row.len(), 3);
    assert_eq!(row[0], cell(' '));
    assert_eq!(row[1], Cell::default());
    assert_eq!(row[2], Cell::default());
}

/// Tests growing the grid with reflow, including the start of command marker on an empty line.
#[test]
fn grow_reflow_with_start_of_command_marker() {
    let mut grid = GridHandler::new_for_test(3, 3);

    {
        let grid = grid.grid_storage_mut();
        grid[0][0] = cell('1');
        grid[0][1] = cell('2');
        grid[0][2] = wrap_cell('3');
        // Second line is "empty", but has a start of command marker which means it should NOT be considered a "clear row".
        // We have `let len = min(row.len(), num_wrapped);` which results in the len of 1 (from num_wrapped). This means, we reflow the 1st cell
        // of the 2nd row, regardless of whether it's empty or not. Hence, we need the start of the command marker to be in a cell _after_
        // this 1st cell, so we can test the logic that computes whether we should reflow the rest of the contents or not (the latter part
        // of `front_split_off`).
        // This confirms that we correctly consider the start of command marker as a non-empty cell, strictly for the purposes of resizing.
        // The code corresponding to this is: `row_is_clear = row.is_clear() && row.has_no_start_of_command_marker()`.
        grid[1][0] = Cell::default();
        grid[1][1] = Cell::default();
        grid[1][2] = cell_with_end_of_prompt_marker(' ', false);
    }

    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    assert_eq!(grid.total_rows(), 2);

    let row = grid.row(0).expect("row should exist");
    assert_eq!(row.len(), 4);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], cell('2'));
    assert_eq!(row[2], cell('3'));
    assert_eq!(row[3], wrap_cell('\0'));

    let row = grid.row(1).expect("row should exist");
    assert_eq!(row.len(), 4);
    assert_eq!(row[0], Cell::default());
    // Start of command marker should be preserved!
    assert_eq!(row[1], cell_with_end_of_prompt_marker(' ', false));
    assert_eq!(row[2], Cell::default());
    assert_eq!(row[3], Cell::default());
}

#[test]
fn grow_reflow_multiline() {
    let mut grid = GridHandler::new_for_test(3, 2);
    grid.input_at_cursor("123456");

    grid.resize(SizeInfo::new_without_font_metrics(3, 6));

    assert_eq!(grid.total_rows(), 3);

    let row = grid.row(0).expect("row should exist");
    assert_eq!(row.len(), 6);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], cell('2'));
    assert_eq!(row[2], cell('3'));
    assert_eq!(row[3], cell('4'));
    assert_eq!(row[4], cell('5'));
    assert_eq!(row[5], cell('6'));

    // Make sure rest of grid is empty.
    for r in 1..3 {
        let row = grid.row(r).expect("row should exist");
        assert_eq!(row.len(), 6);
        for c in 0..6 {
            assert_eq!(row[c], Cell::default());
        }
    }
}

#[test]
fn grow_reflow_disabled() {
    let mut grid = GridHandler::new_for_alt_screen_test(2, 2);
    grid.input_at_cursor("123 ");

    grid.resize(SizeInfo::new_without_font_metrics(2, 3));

    assert_eq!(grid.total_rows(), 2);

    let row = grid.row(0).expect("row should exist");
    assert_eq!(row.len(), 3);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], wrap_cell('2'));
    assert_eq!(row[2], Cell::default());

    let row = grid.row(1).expect("row should exist");
    assert_eq!(row.len(), 3);
    assert_eq!(row[0], cell('3'));
    assert_eq!(row[1], cell(' '));
    assert_eq!(row[2], Cell::default());
}

#[test]
fn shrink_reflow_disabled() {
    let mut grid = GridHandler::new_for_alt_screen_test(1, 5);
    grid.input_at_cursor("12345");

    grid.resize(SizeInfo::new_without_font_metrics(1, 2));

    assert_eq!(grid.total_rows(), 1);

    let row = grid.row(0).expect("row should exist");
    assert_eq!(row.len(), 2);
    assert_eq!(row[0], cell('1'));
    assert_eq!(row[1], cell('2'));
}

#[test]
fn grow_can_move_max_cursor_column_left() {
    // Starting Grid
    // [1234] (wrap)
    // [5678]
    let mut grid = GridHandler::new_for_test(2, 4);
    grid.input_at_cursor("12345678");
    grid.grid_storage_mut().update_max_cursor();

    // Grow to 6 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(2, 6));

    // After resizing, the grid should look like:
    // [123456] (wrap)
    // [78    ]

    // The `max_cursor` should still be on the Cell containing '8'
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 1,
        }
    );
}

#[test]
fn grow_can_leave_max_cursor_column_unchanged() {
    // Starting Grid
    // [1234] (wrap)
    // [5678] (wrap)
    // [90  ]
    let mut grid = GridHandler::new_for_test(3, 4);
    grid.input_at_cursor("1234567890");

    // Grow to 8 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 8));

    // After resizing, the grid should look like:
    // [12345678]
    // [90      ]
    // [        ]

    // The `max_cursor` should still be on the cell after the '0'
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 2,
        }
    );
}

#[test]
fn grow_can_move_max_cursor_column_right() {
    // Starting Grid
    // [12] (wrap)
    // [34] (wrap)
    // [56]
    let mut grid = GridHandler::new_for_test(3, 2);
    grid.input_at_cursor("123456");

    // Grow to 6 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 6));

    // After resizing, the grid should look like:
    // [123456]
    // [      ]
    // [      ]

    // The `max_cursor` should still be on the cell containing '6'
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(0),
            col: 5,
        }
    );
}

#[test]
fn shrink_can_move_max_cursor_column_left() {
    // Starting Grid:
    // [     ]
    // [     ]
    // [12345]
    let mut grid = GridHandler::new_for_test(3, 5);
    grid.set_cursor_point(2, 0);
    grid.input_at_cursor("12345");

    // Shrink to 2 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 2));
    // grid.grid_storage_mut().resize(true, 3, 2);

    // After resizing, the grid should look like:
    // [12] (wrap)
    // [34] (wrap)
    // [5 ]

    // The `max_cursor` should still be on the cell containing '5'
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 0,
        }
    );
}

#[test]
fn shrink_can_leave_max_cursor_column_unchanged() {
    // Starting Grid:
    // [        ]
    // [12345678] (wrap)
    // [90      ]
    let mut grid = GridHandler::new_for_test(3, 8);
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("1234567890");

    // Shrink to 4 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 4));

    // After resizing, the grid should look like:
    // [1234] (wrap)
    // [5678] (wrap)
    // [90  ]

    // The `max_cursor` should still be after the cell containing '0'
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 2,
        }
    );
}

#[test]
fn shrink_can_move_max_cursor_column_right() {
    // Starting Grid:
    // [123456] (wrap)
    // [7     ]
    let mut grid = GridHandler::new_for_test(2, 6);
    grid.input_at_cursor("1234567");

    // Shrink to 4 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    // After resizing, the grid should look like:
    // [1234] (wrap)
    // [5678]

    // The `max_cursor` should still be after the cell containing '7'
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 3,
        }
    );
}

#[test]
fn shrink_keeps_whitespace_before_max_cursor() {
    // Starting Grid:
    // [      ]
    // [123456] (wrap)
    // [78   |] (max cursor at end)
    let mut grid = GridHandler::new_for_test(3, 6);
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("12345678    ");
    grid.grid_storage_mut().update_max_cursor();

    // Shrink to 4 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 4));

    // After resizing, the grid should look like the following. Note that the max_cursor encodes
    // explicit whitespace, so that needs to be included as well.
    // [1234] (wrap)
    // [5678] (wrap)
    // [   |] (max cursor still at end)

    // The `max_cursor` should still be at the end of the last row
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 3,
        }
    );
}

#[test]
fn shrink_keeps_multiple_lines_of_whitespace_before_max_cursor() {
    // Starting Grid:
    // [        ]
    // [12345678] (wrap)
    // [90     |] (max cursor at end)
    let mut grid = GridHandler::new_for_test(3, 8);
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("1234567890      ");
    grid.grid_storage_mut().update_max_cursor();

    // Shrink to 3 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 3));

    // After resizing, the grid should look like the following. Note that the max_cursor encodes
    // explicit whitespace, so that needs to be included as well.
    // [0  ]
    // [   ]
    // [|  ] (max cursor at beginning now)

    // The `max_cursor` should be at the beginning of the last row
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 0,
        }
    );
}

#[test]
fn shrink_keeps_less_than_one_line_of_whitespace_before_max_cursor() {
    // Starting Grid:
    // [      ]
    // [123456] (wrap)
    // [78   |] (max cursor at end)
    let mut grid = GridHandler::new_for_test(3, 6);
    grid.set_cursor_point(1, 0);
    grid.input_at_cursor("12345678    ");
    grid.grid_storage_mut().update_max_cursor();

    // Shrink to 5 cells wide
    grid.resize(SizeInfo::new_without_font_metrics(3, 5));

    // After resizing, the grid should look like the following. Note that the max_cursor encodes
    // explicit whitespace, so that needs to be included as well.
    // [12345] (wrap)
    // [678  ]
    // [ |   ] (max cursor moved to middle)

    // The `max_cursor` should still be at the end of the last row
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 1,
        }
    );
}

#[test]
fn shrink_lines_moves_into_scrollback() {
    // Starting Grid:
    // [abcd]
    // [1234]
    // [efgh]
    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 4, 2);
    grid.input_at_cursor("abcd1234efgh");

    // There shouldn't be anything in scrollback yet.
    assert_eq!(grid.history_size(), 0);

    // The cursor should be on the last cell of the grid.
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('a'));

    // Reduce the number of visible rows by 1.
    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    // We should have moved one row into scrollback.
    assert_eq!(grid.history_size(), 1);

    // The cursor should be in the same absolute position, but a different
    // "visible position" (as one line is no longer visible and is in
    // scrollback).
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('1'));

    let first_row = grid.row(0).expect("row 0 should exist");
    assert_eq!(first_row[0], cell('a'));
}

#[test]
fn shrink_lines_with_cursor_not_at_bottom() {
    // Starting Grid:
    // [1234]
    // [abcd]
    // [5678]
    // [efgh]
    let mut grid = GridHandler::new_for_test(4, 4);
    grid.input_at_cursor("1234abcd5678efgh");
    grid.set_cursor_point(2, 0);

    let row = &grid.grid_storage()[VisibleRow(0)];
    assert_eq!(row[0], cell('1'));

    let cursor_point = grid.grid_storage().cursor.point;
    assert_eq!(
        grid.grid_storage()[cursor_point.row][cursor_point.col],
        cell('5')
    );

    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    // While shrinking by two rows, we first truncate rows below the cursor,
    // then start pushing content into scrollback.  This means that we drop
    // the "efgh" row (we've now shrunk by 1), and then we push the "1234"
    // row into scrollback (we've now shrunk by 2).
    let row = &grid.grid_storage()[VisibleRow(0)];
    assert_eq!(row[0], cell('a'));

    // This should not affect the position of the cursor, which is still over
    // the cell containing '5'.
    let cursor_point = grid.grid_storage().cursor.point;
    assert_eq!(
        grid.grid_storage()[cursor_point.row][cursor_point.col],
        cell('5')
    );
}

#[test]
fn grow_lines_pulls_all_needed_lines_from_scrollback() {
    // Starting Grid:
    // [abcd]
    // [1234]
    // [efgh]
    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 4, 2);
    grid.input_at_cursor("abcd1234efgh");

    // There shouldn't be anything in scrollback yet.
    assert_eq!(grid.history_size(), 0);

    // The cursor should be on the last cell of the grid.
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('a'));

    // Reduce the number of visible rows by 2.
    grid.resize(SizeInfo::new_without_font_metrics(1, 4));

    // We should have moved two rows into scrollback.
    assert_eq!(grid.history_size(), 2);

    // The cursor should be in the same absolute position, but a different
    // "visible position" (as one line is no longer visible and is in
    // scrollback).
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(0),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('e'));

    // Grow the number of lines by 1, pulling one line from scrollback.
    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    // We should now only have one row in scrollback.
    assert_eq!(grid.history_size(), 1);

    // The cursor should be in the same absolute position, but a different
    // "visible position".
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('1'));
}

#[test]
fn grow_lines_pulls_some_lines_from_scrollback() {
    // Starting Grid:
    // [abcd]
    // [1234]
    // [efgh]
    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 4, 2);
    grid.input_at_cursor("abcd1234efgh");

    // There shouldn't be anything in scrollback yet.
    assert_eq!(grid.history_size(), 0);

    // The cursor should be on the last cell of the grid.
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('a'));

    // Reduce the number of visible rows by 1.
    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    // We should have moved one row into scrollback.
    assert_eq!(grid.history_size(), 1);

    // The cursor should be in the same absolute position, but a different
    // "visible position" (as one line is no longer visible and is in
    // scrollback).
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('1'));

    // Grow the number of lines by 2, pulling one line from scrollback and
    // inserting a blank row at the bottom.
    grid.resize(SizeInfo::new_without_font_metrics(4, 4));

    // We should now have zero rows in scrollback.
    assert_eq!(grid.history_size(), 0);

    // The cursor should be back at the original position and "visible
    // position".
    assert_eq!(grid.cursor_point(), Point::new(2, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('a'));

    let last_row = grid
        .row(grid.total_rows() - 1)
        .expect("last row should exist");
    assert_eq!(last_row[0], Cell::default());
}

#[test]
fn grow_lines_with_no_scrollback() {
    // Starting Grid:
    // [abcd]
    // [1234]
    let mut grid = GridHandler::new_for_test_with_scroll_limit(2, 4, 2);
    grid.input_at_cursor("abcd1234");

    // There shouldn't be anything in scrollback.
    assert_eq!(grid.history_size(), 0);

    // The cursor should be on the last cell of the grid.
    assert_eq!(grid.cursor_point(), Point::new(1, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('a'));

    // Grow the number of visible rows by 1.
    grid.resize(SizeInfo::new_without_font_metrics(3, 4));

    // There still shouldn't be anything in scrollback.
    assert_eq!(grid.history_size(), 0);

    // The cursor should be in the same absolute and "visible" location as
    // before..
    assert_eq!(grid.cursor_point(), Point::new(1, 3));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 3,
        }
    );

    // Assert the row contents match our expectations.
    let first_visible_row = grid
        .row(grid.history_size())
        .expect("visible row 0 should exist");
    assert_eq!(first_visible_row[0], cell('a'));

    let new_final_row = grid
        .row(grid.total_rows() - 1)
        .expect("last row should exist");
    assert_eq!(new_final_row[0], Cell::default());
}

#[test]
fn shrink_lines_with_max_and_saved_cursors_after_cursor() {
    // Starting Grid (pre resize):
    // [abcd]
    // [1234]
    // [xyzw] <- cursor is under the x
    // [_ M ] <- empty row where _ is the saved cursor & M is the max cursor
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 4, 2);
    grid.input_at_cursor("abcd1234xyzw");

    assert_eq!(grid.cursor_point(), Point::new(3, 0));

    // Save the cursor position.
    grid.save_cursor_position();

    // Move the cursor over a couple cells and mark it as the max cursor point.
    grid.set_cursor_point(3, 2);
    grid.grid_storage_mut().update_max_cursor();

    // Move the cursor to a location before both the saved and max cursors.
    grid.set_cursor_point(2, 0);
    assert_eq!(grid.cursor_point(), Point::new(2, 0));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(2),
            col: 0,
        }
    );
    assert_eq!(
        grid.grid_storage().saved_cursor.point,
        VisiblePoint {
            row: VisibleRow(3),
            col: 0,
        }
    );
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(3),
            col: 2,
        }
    );

    // Shrink the number of visible rows by 2.
    grid.resize(SizeInfo::new_without_font_metrics(2, 4));

    // Resizing the grid from 4 rows down to 2 should have dropped the row
    // after the cursor, and pushed the top row into scrollback.
    assert_eq!(grid.total_rows(), 3);
    assert_eq!(grid.visible_rows(), 2);
    assert_eq!(grid.cursor_point(), Point::new(2, 0));
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 0,
        }
    );

    // The max cursor and saved cursors should have been moved up by two rows,
    // given that the row containing them was dropped.
    assert_eq!(
        grid.grid_storage().saved_cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 0,
        }
    );
    assert_eq!(
        grid.grid_storage().max_cursor_point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 2,
        }
    );
}

/// Test for the scenario where an empty row has only a LEADING_WIDE_CHAR_SPACER flag.
///
/// This happens when the cursor is moved to the final cell, and then a wide character
/// (like an emoji) is written, which wraps to the next line due to being two cells wide.
///
/// Some previous logic would cause a panic during resize due to an early return in
/// append_to_index dropping a row that should have been added to the index; this regression
/// test covers that scenario.
#[test]
fn test_empty_row_with_leading_wide_char_spacer_resize_panic() {
    // Create a small grid to force wrapping behavior.
    let mut grid = GridHandler::new_for_test(3, 3);

    // Move cursor to the final cell of the second row.
    grid.set_cursor_point(1, 2);

    // Make sure the character we're using is wide.
    let wide_char = '😂';
    let char_width =
        unicode_width::UnicodeWidthChar::width(wide_char).expect("char should have a known width");
    assert!(char_width > 1);

    // Input a wide character (emoji) that will wrap to the next line.
    // This should create a LEADING_WIDE_CHAR_SPACER in the final cell of the second row
    // and put the actual emoji on the third row.
    //
    // Why do this on the second row, not the first?  We want the row with the
    // LEADING_WIDE_CHAR_SPACER flag to be empty, and when we resize the grid, we
    // will mark cells with cursors in a way that makes them non-empty, and
    // `saved_cursor` defaults to (0,0).
    grid.input(wide_char);

    let row = grid.row(1).expect("should be able to get the second row");
    let last_cell = row.get(2).expect("should have three cells");

    // Validate the contents of cell (1, 2).
    assert_eq!(last_cell.c, cell::DEFAULT_CHAR);
    assert!(last_cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER));

    // Validate the contents of cell (2, 0).
    let row = grid.row(2).expect("should be able to get the third row");
    let first_cell = row.get(0).expect("should have three cells");
    assert_eq!(first_cell.c, wide_char);

    // Move the cursor to the end of the grid.
    grid.set_cursor_point(2, 2);

    // Resize the grid by adding a column.  Before the bug fix, this would have caused
    // a panic where we failed to compute the content offset for the cursor, as flat
    // storage would have one too few rows, and the cursor position would be out of bounds.
    grid.resize(SizeInfo::new_without_font_metrics(2, 4));
}

#[test]
fn test_shrink_cols_reflow_preserves_split_wide_char_as_wrapped_content() {
    let mut grid = GridStorage::new(2, 6, 0, ObfuscateSecrets::No);
    let mut wide_char = cell('Ｗ');
    wide_char.flags.insert(Flags::WIDE_CHAR);
    let mut spacer = Cell::default();
    spacer.flags.insert(Flags::WIDE_CHAR_SPACER);

    grid.set_stored_rows(
        vec![
            row::Row::new(6),
            row::Row::from_vec(
                vec![
                    cell('a'),
                    cell('b'),
                    cell('c'),
                    cell('d'),
                    wide_char,
                    spacer,
                ],
                6,
            ),
        ],
        2,
        6,
    );

    grid.resize(true, 2, 5, false);

    let retained_boundary = &grid[VisibleRow(0)][4];
    assert!(
        retained_boundary
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER)
    );
    assert!(!retained_boundary.flags.contains(Flags::WIDE_CHAR));

    let wrapped_wide_char = &grid[VisibleRow(1)][0];
    assert_eq!(wrapped_wide_char.c, 'Ｗ');
    assert!(wrapped_wide_char.flags.contains(Flags::WIDE_CHAR));
    assert!(
        grid[VisibleRow(1)][1]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER)
    );
}

fn cell(c: char) -> Cell {
    let mut cell = Cell::default();
    cell.c = c;
    cell
}

fn cell_with_end_of_prompt_marker(c: char, has_extra_trailing_newline: bool) -> Cell {
    let mut cell = Cell::default();
    cell.c = c;
    cell.mark_end_of_prompt(has_extra_trailing_newline);
    cell
}

fn wrap_cell(c: char) -> Cell {
    let mut cell = cell(c);
    cell.flags.insert(Flags::WRAPLINE);
    cell
}
