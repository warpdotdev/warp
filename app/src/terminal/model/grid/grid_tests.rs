use std::num::NonZeroUsize;

use crate::terminal::model::ansi::{self, Handler as _};
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::model::index::Point;
use crate::terminal::model::secrets::ObfuscateSecrets;
use crate::terminal::model::{
    grid::Dimensions,
    index::{VisiblePoint, VisibleRow},
};
use crate::terminal::SizeInfo;

use super::GridStorage;

macro_rules! assert_cell_char_eq {
    ($grid:ident[$row:literal][$col:literal], $expected:literal) => {
        let row = $grid.row($row).expect("row should exist");
        assert_eq!(row[$col].c, $expected);
    };
}

#[test]
fn test_grid_scroll_down() {
    // this simulates the alt-screen with 5 rows
    let mut grid = GridStorage::new(5, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';
    grid.scroll_down(&(VisibleRow(0)..VisibleRow(5)), 1);
    assert_eq!(grid[0][0].c, '\0');
    assert_eq!(grid[1][0].c, 'a');
    assert_eq!(grid[2][0].c, 'b');
    assert_eq!(grid[3][0].c, 'c');
    assert_eq!(grid[4][0].c, 'd');
}

#[test]
fn test_grid_scroll_down_with_fixed_line() {
    // this simulates the alt-screen with a fixed row at the top and bottom
    let mut grid = GridStorage::new(5, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';
    grid.scroll_down(&(VisibleRow(1)..VisibleRow(4)), 1);
    assert_eq!(grid[0][0].c, 'a');
    assert_eq!(grid[1][0].c, '\0');
    assert_eq!(grid[2][0].c, 'b');
    assert_eq!(grid[3][0].c, 'c');
    assert_eq!(grid[4][0].c, 'e');
}

#[test]
fn test_grid_scroll_up() {
    // this simulates the alt-screen with 5 rows
    let mut grid = GridStorage::new(5, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';
    grid.scroll_up(&(VisibleRow(0)..VisibleRow(5)), 1);
    assert_eq!(grid[0][0].c, 'b');
    assert_eq!(grid[1][0].c, 'c');
    assert_eq!(grid[2][0].c, 'd');
    assert_eq!(grid[3][0].c, 'e');
    assert_eq!(grid[4][0].c, '\0');
    assert_eq!(grid.num_lines_truncated, 1);

    // this simulates the alt-screen with a fixed row at the top and bottom
    let mut grid = GridStorage::new(5, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';
    grid.scroll_up(&(VisibleRow(1)..VisibleRow(4)), 1);
    assert_eq!(grid[0][0].c, 'a');
    assert_eq!(grid[1][0].c, 'c');
    assert_eq!(grid[2][0].c, 'd');
    assert_eq!(grid[3][0].c, '\0');
    assert_eq!(grid[4][0].c, 'e');
    assert_eq!(grid.num_lines_truncated, 1);
}

#[test]
fn test_grid_scroll_up_by_multiple_lines() {
    let mut grid = GridStorage::new(5, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';

    grid.scroll_up(&(VisibleRow(0)..VisibleRow(5)), 2);

    assert_eq!(grid[0][0].c, 'c');
    assert_eq!(grid[1][0].c, 'd');
    assert_eq!(grid[2][0].c, 'e');
    assert_eq!(grid[3][0].c, '\0');
    assert_eq!(grid[4][0].c, '\0');

    // Every line scrolled immediately becomes truncated since there's no scrollback buffer.
    assert_eq!(grid.num_lines_truncated, 2);
}

#[test]
fn test_grid_scroll_up_only_some_lines_truncated() {
    let mut grid = GridStorage::new(5, 1, 1, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';

    grid.scroll_up(&(VisibleRow(0)..VisibleRow(5)), 2);

    assert_eq!(grid[0][0].c, 'b');
    assert_eq!(grid[1][0].c, 'c');
    assert_eq!(grid[2][0].c, 'd');
    assert_eq!(grid[3][0].c, 'e');
    assert_eq!(grid[4][0].c, '\0');

    // Only one line ('b') should be truncated even though we scrolled up by 2
    // since the scrollback buffer has space for one row.
    assert_eq!(grid.history_size(), 1);
    assert_eq!(grid.num_lines_truncated, 1);
}

#[test]
fn test_grid_scroll_up_with_fixed_lines() {
    // this simulates the alt-screen with a fixed row at the top and bottom
    let mut grid = GridStorage::new(5, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';
    grid.scroll_up(&(VisibleRow(1)..VisibleRow(4)), 1);
    assert_eq!(grid[0][0].c, 'a');
    assert_eq!(grid[1][0].c, 'c');
    assert_eq!(grid[2][0].c, 'd');
    assert_eq!(grid[3][0].c, '\0');
    assert_eq!(grid[4][0].c, 'e');
}

#[test]
fn test_grid_git_diff_or_log() {
    // Imagine this is a git log.

    // First, simulate the shell populating the visible screen.
    let mut grid = GridStorage::new(5, 1, 2, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid[3][0].c = 'd';
    grid[4][0].c = 'e';
    assert_eq!(grid.total_rows(), 5);

    // Second, simulate the user scrolling for more rows twice.
    grid.scroll_up(&(VisibleRow(0)..VisibleRow(5)), 2); // use the scrollback
    assert_eq!(grid.total_rows(), 7);

    // Now, the shell populating these two new rows.
    grid[5][0].c = 'f';
    grid[6][0].c = 'g';

    assert_eq!(grid[VisibleRow(0)][0].c, 'c'); // grid index 2
    assert_eq!(grid[VisibleRow(1)][0].c, 'd');
    assert_eq!(grid[VisibleRow(2)][0].c, 'e');
    assert_eq!(grid[VisibleRow(3)][0].c, 'f');
    assert_eq!(grid[VisibleRow(4)][0].c, 'g'); // grid index 6

    // Third, simulate the user scrolling back towards the top
    grid.scroll_down(&(VisibleRow(0)..VisibleRow(5)), 1);
    assert_eq!(grid[VisibleRow(0)][0].c, '\0'); // the shell is responsible for populating this
    assert_eq!(grid[VisibleRow(1)][0].c, 'c');
    assert_eq!(grid[VisibleRow(2)][0].c, 'd');
    assert_eq!(grid[VisibleRow(3)][0].c, 'e');
    assert_eq!(grid[VisibleRow(4)][0].c, 'f');

    // The off-screen region should still be intact
    assert_eq!(grid[0][0].c, 'a');
    assert_eq!(grid[1][0].c, 'b');
}

#[test]
fn test_truncate_cursor_bottom_scrollback() {
    // Create a filled grid with 2 lines of scrollback buffer
    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 1, 2);
    {
        let grid = grid.grid_storage_mut();
        grid[0][0].c = 'a';
        grid[1][0].c = 'b';
        grid[2][0].c = 'c';
        grid[3][0].c = 'd';
        grid[4][0].c = 'e';
    }

    // Push two rows into scrollback
    grid.scroll_up(2);
    assert_eq!(grid.history_size(), 2);

    // Verify the grid
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Set the cursor to the bottom of the grid
    grid.set_cursor_point(4, 0);

    // Truncate everything after the cursor
    grid.truncate_to_cursor_rows();

    // Verify that the grid is unchanged
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Verify that the cursor is unchanged
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(4));
}

#[test]
fn test_truncate_cursor_bottom_no_scrollback() {
    // Create a filled grid with no scrollback buffer
    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 1, 0);
    {
        let grid = grid.grid_storage_mut();
        grid[0][0].c = 'a';
        grid[1][0].c = 'b';
        grid[2][0].c = 'c';
        grid[3][0].c = 'd';
        grid[4][0].c = 'e';
    }

    // Push two rows off the screen
    grid.scroll_up(2);
    assert_eq!(grid.history_size(), 0);

    // Verify the grid
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Set the cursor to the bottom of the grid
    grid.set_cursor_point(4, 0);

    // Truncate everything after the cursor
    grid.truncate_to_cursor_rows();

    // Verify that the grid is unchanged
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Verify that the cursor is unchanged
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(4));
}

#[test]
fn test_truncate_cursor_middle_scrollback() {
    // Create a filled grid with 2 lines of scrollback buffer
    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 1, 2);
    {
        let grid = grid.grid_storage_mut();
        grid[0][0].c = 'a';
        grid[1][0].c = 'b';
        grid[2][0].c = 'c';
        grid[3][0].c = 'd';
        grid[4][0].c = 'e';
    }

    // Push two rows into scrollback
    grid.scroll_up(2);

    // Verify the grid
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Set the cursor to the middle of the grid
    grid.set_cursor_point(2, 0);

    // Truncate everything after the cursor
    grid.truncate_to_cursor_rows();

    // Verify that the grid has pulled content from scrollback
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'a');
        assert_eq!(grid[VisibleRow(1)][0].c, 'b');
        assert_eq!(grid[VisibleRow(2)][0].c, 'c');
        assert_eq!(grid[VisibleRow(3)][0].c, 'd');
        assert_eq!(grid[VisibleRow(4)][0].c, 'e');
    }

    // Verify that the cursor is now at the bottom
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(4));
}

#[test]
fn test_truncate_cursor_middle_partial_scrollback() {
    // Create a filled grid with one line of scrollback buffer
    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 1, 1);
    {
        let grid = grid.grid_storage_mut();
        grid[0][0].c = 'a';
        grid[1][0].c = 'b';
        grid[2][0].c = 'c';
        grid[3][0].c = 'd';
        grid[4][0].c = 'e';
    }

    // Push two rows off the screen; one will stay in scrollback because that is the limit
    grid.scroll_up(2);
    assert_eq!(grid.history_size(), 1);

    // Verify the grid
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Set the cursor to the middle of the grid
    grid.set_cursor_point(2, 0);

    // Truncate everything after the cursor
    grid.truncate_to_cursor_rows();

    // Expected behavior is that two rows below the cursor get truncated, and
    // one row gets pulled into the grid from scrollback. This leaves a total
    // of 4 visible rows, and none in scrollback.
    assert_eq!(grid.visible_rows(), 4);
    assert_eq!(grid.history_size(), 0);

    // Verify that the grid has pulled content from scrollback
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'b');
        assert_eq!(grid[VisibleRow(1)][0].c, 'c');
        assert_eq!(grid[VisibleRow(2)][0].c, 'd');
        assert_eq!(grid[VisibleRow(3)][0].c, 'e');
    }

    // Verify that the cursor has shifted
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(3));
}

#[test]
fn test_truncate_cursor_middle_no_scrollback() {
    // Create a filled grid with no scrollback buffer
    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 1, 0);
    {
        let grid = grid.grid_storage_mut();
        grid[0][0].c = 'a';
        grid[1][0].c = 'b';
        grid[2][0].c = 'c';
        grid[3][0].c = 'd';
        grid[4][0].c = 'e';
    }

    // Push two rows off the screen
    grid.scroll_up(2);

    // Verify the grid
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
        assert_eq!(grid[VisibleRow(3)][0].c, '\0');
        assert_eq!(grid[VisibleRow(4)][0].c, '\0');
    }

    // Set the cursor to the middle of the grid
    grid.set_cursor_point(2, 0);

    // Truncate everything after the cursor
    grid.truncate_to_cursor_rows();
    assert_eq!(grid.visible_rows(), 3);
    assert_eq!(grid.history_size(), 0);

    // Verify that the grid is truncated
    {
        let grid = grid.grid_storage();
        assert_eq!(grid[VisibleRow(0)][0].c, 'c');
        assert_eq!(grid[VisibleRow(1)][0].c, 'd');
        assert_eq!(grid[VisibleRow(2)][0].c, 'e');
    }

    // Verify that the cursor is unchanged
    assert_eq!(grid.grid_storage().cursor.point.row, VisibleRow(2));
}

#[test]
fn test_end_prompt_point() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(5, 6, 1);
    assert_eq!(grid.prompt_end_point(), None);

    grid.grid_storage_mut()[2][1].mark_end_of_prompt(false);

    assert_eq!(grid.prompt_end_point(), Some(Point::new(2, 1)));
}

#[test]
fn test_grid_truncate_to_cursor_cols() {
    let mut grid = GridStorage::new(3, 6, 1, ObfuscateSecrets::No);
    // Before:
    //   012345
    // 0 ab
    // 1   c
    // 2
    grid[0][0].c = 'a';
    grid[0][1].c = 'b';
    grid[0][2].c = ' ';
    grid[1][2].c = 'c';
    grid[1][3].c = ' ';

    // Simulate that the cursor just finished printing the characters above.
    grid.cursor.point = VisiblePoint {
        row: VisibleRow(1),
        // Cursor goes one character beyond the last printed character
        // i.e. the next cell we can print.
        col: 4,
    };

    assert_eq!(grid.columns, 6);

    // We expect this truncate the grid to 4 columns.
    grid.truncate_to_cursor_cols();

    // After:
    //   0123
    // 0 ab
    // 1   c
    // 2

    // Verify we've truncated the columns and cell content still exists.
    assert_eq!(grid.columns, 4);
    assert_eq!(grid[1][2].c, 'c');
    assert_eq!(grid[1][3].c, ' ');
}

/// Test to ensure we DO NOT truncate "true content" in the case where the cursor is at the end of the line
/// and indicates the input needs to be wrapped for the next character.
#[test]
fn test_grid_truncate_to_cursor_cols_full_wrapped_line() {
    let mut grid = GridStorage::new(3, 6, 1, ObfuscateSecrets::No);
    // Before:
    //   012345
    // 0 ab
    // 1 abcdef <- cursor at (1, 5) with input_needs_wrap = true
    // 2
    grid[0][0].c = 'a';
    grid[0][1].c = 'b';
    grid[0][2].c = ' ';

    grid[1][0].c = 'a';
    grid[1][1].c = 'b';
    grid[1][2].c = 'c';
    grid[1][3].c = 'd';
    grid[1][4].c = 'e';
    grid[1][5].c = 'f';

    // Cursor at end of line, with input_needs_wrap = true, indicating the next
    // character should be wrapped!
    grid.cursor.point = VisiblePoint {
        row: VisibleRow(1),
        col: 5,
    };
    grid.cursor.input_needs_wrap = true;

    assert_eq!(grid.columns, 6);

    // We expect this DOES NOT truncate the grid any further, due to the cursor state above.
    grid.truncate_to_cursor_cols();

    // Verify we haven't truncated columns and cell content still exists.
    assert_eq!(grid.columns, 6);
    assert_eq!(grid[0][1].c, 'b');
    assert_eq!(grid[1][5].c, 'f');
}

#[test]
fn test_split_grid_cursor_last_position() {
    // 1. Setup: Create and initialize the grid.
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);

    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['d', 'd', 'd', 'd', 'd', '\0'],
    ]);

    assert_eq!(grid.dirty_cells_range(), None);

    // We update the cursor, to create a dirty_cells_range
    grid.update_cursor(|cursor| {
        cursor.point.row = VisibleRow(3);
        cursor.point.col = 5;
    });

    // The cursor marks the cell _after_ the last modified cell, so the range
    // should extend to the cell before the cursor.
    assert_eq!(
        grid.dirty_cells_range(),
        Some(Point::new(0, 0)..=Point::new(3, 4))
    );

    // Grid splitting only ever occurs from the main thread, so pretend to
    // finish byte processing before we start the split.  This should clear
    // out the dirty cells range.
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert!(grid.dirty_cells_range().is_none());

    // 2. Preconditions: Check initial state.
    assert_eq!(grid.columns(), 6);
    assert_eq!(grid.grid_storage()[0][0].c, 'a'); // Should be reversed due to bottom_row = 0

    // 3. Action: Split the grid.
    match grid.split(NonZeroUsize::new(1).expect("should not be zero")) {
        (grid, Some(bottom_grid)) => {
            // 4. Postconditions: Verify split results.
            // Check row and visible counts
            assert_eq!(grid.total_rows(), 1);
            assert_eq!(grid.visible_rows(), 1);
            assert_eq!(grid.grid_storage().raw.len(), 1);

            assert_eq!(bottom_grid.total_rows(), 3);
            assert_eq!(bottom_grid.visible_rows(), 3);
            assert_eq!(bottom_grid.grid_storage().raw.len(), 3);

            // Verify character content for grid1 and grid2
            assert_eq!(grid.grid_storage()[0][0].c, 'a');
            assert_eq!(grid.grid_storage()[0][1].c, 'a');

            assert_eq!(bottom_grid.grid_storage()[0][0].c, 'b');
            assert_eq!(bottom_grid.grid_storage()[0][1].c, 'b');
            assert_eq!(bottom_grid.grid_storage()[1][0].c, 'c');
            assert_eq!(bottom_grid.grid_storage()[2][0].c, 'd');

            // Check cursor position
            assert_eq!(
                grid.grid_storage().cursor.point,
                VisiblePoint {
                    row: VisibleRow(0),
                    col: 5
                }
            );
            assert_eq!(
                bottom_grid.grid_storage().cursor.point,
                VisiblePoint {
                    row: VisibleRow(2),
                    col: 5
                }
            );

            // We shouldn't ever split a grid while we're in the middle of
            // processing data, so the dirty_cells_range should always be
            // empty (i.e.: None).
            assert!(grid.dirty_cells_range().is_none());
            assert!(bottom_grid.dirty_cells_range().is_none());
        }
        _ => {
            panic!("Received None from split (split row exceeded Grid!)");
        }
    }
}

#[test]
fn test_split_grid_cursor_in_grid1() {
    // 1. Setup: Create and initialize the grid.
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);
    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['d', 'd', 'd', 'd', 'd', '\0'],
    ]);

    // Place the cursor somewhere in the first half of the grid.
    grid.set_cursor_point(1, 1); // Cursor at 'b'
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // 2. Preconditions: Check initial state.
    assert_eq!(
        grid.grid_storage().cursor.point,
        VisiblePoint {
            row: VisibleRow(1),
            col: 1
        }
    );

    // 3. Action: Split the grid at the third row (index 2).
    match grid.split(NonZeroUsize::new(2).expect("should not be zero")) {
        (grid, Some(bottom_grid)) => {
            let grid = grid.grid_storage();
            let bottom_grid = bottom_grid.grid_storage();

            // 4. Postconditions: Verify split results.

            // Check that cursor's position in grid1 is as expected.
            assert_eq!(
                grid.cursor.point,
                VisiblePoint {
                    row: VisibleRow(1),
                    col: 1
                }
            );

            // Ensure grid2's cursor is at the default (0, 0) position.
            assert_eq!(
                bottom_grid.cursor.point,
                VisiblePoint {
                    row: VisibleRow(0),
                    col: 0
                }
            );

            // Additionally, verify the contents to ensure the split was successful.
            assert_eq!(grid[0][0].c, 'a');
            assert_eq!(grid[1][0].c, 'b');

            assert_eq!(bottom_grid[0][0].c, 'c');
            assert_eq!(bottom_grid[1][0].c, 'd');
        }
        _ => {
            panic!("Received None from split (split row exceeded Grid!)");
        }
    }
}

#[test]
fn test_split_already_split_grid() {
    // 1. Setup: Create and initialize the grid.
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);
    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['d', 'd', 'd', 'd', 'd', '\0'],
    ]);

    match grid.split(NonZeroUsize::new(2).expect("should not be zero")) {
        (grid, Some(bottom_grid)) => {
            // Preconditions: Check initial state of split grid.
            assert_eq!(grid.columns(), 6);
            assert_eq!(grid.total_rows(), 2);

            assert_eq!(bottom_grid.columns(), 6);
            assert_eq!(bottom_grid.total_rows(), 2);

            // Action: Split the grid again.
            match bottom_grid.split(NonZeroUsize::new(1).expect("should not be zero")) {
                (bottom_grid, Some(bottom_grid_2)) => {
                    let bottom_grid = bottom_grid.grid_storage();
                    let bottom_grid_2 = bottom_grid_2.grid_storage();

                    // Postconditions: Verify split results for second split.
                    assert_eq!(bottom_grid.total_rows(), 1);
                    assert_eq!(bottom_grid.raw.len(), 1);

                    assert_eq!(bottom_grid_2.total_rows(), 1);
                    assert_eq!(bottom_grid_2.raw.len(), 1);

                    // Verify character content for grid2a and grid2b
                    assert_eq!(bottom_grid[0][0].c, 'c');

                    assert_eq!(bottom_grid_2[0][0].c, 'd');
                }
                _ => {
                    panic!("Received None from split (split row exceeded Grid!)");
                }
            }
        }
        _ => {
            panic!("Received None from split (split row exceeded Grid!)");
        }
    }
}

#[test]
fn test_split_grid_cursor_at_split_point() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);
    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['d', 'd', 'd', 'd', 'd', '\0'],
    ]);

    grid.set_cursor_point(2, 3); // Cursor at 'c'
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    match grid.split(NonZeroUsize::new(2).expect("should not be zero")) {
        (grid, Some(bottom_grid)) => {
            let grid = grid.grid_storage();
            let bottom_grid = bottom_grid.grid_storage();

            // Check cursor position in grid2 since the cursor is at the split point.
            assert_eq!(
                bottom_grid.cursor.point,
                VisiblePoint {
                    row: VisibleRow(0),
                    col: 3
                }
            );

            // Check auxiliary data
            assert_eq!(grid.total_rows(), 2);
            assert_eq!(grid.raw.len(), 2);

            assert_eq!(bottom_grid.total_rows(), 2);
            assert_eq!(bottom_grid.raw.len(), 2);

            // Verify character content for grid2a and grid2b
            assert_eq!(grid[0][0].c, 'a');
            assert_eq!(grid[1][0].c, 'b');

            assert_eq!(bottom_grid[0][0].c, 'c');
            assert_eq!(bottom_grid[1][0].c, 'd');
        }
        _ => {
            panic!("Received None from split (split row exceeded Grid!)");
        }
    }
}

#[test]
fn test_split_grid_exceed_rows() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(4, 6, 4);
    grid.grid_storage_mut().populate_from_array(&[
        &['a', 'a', '\0', '\0', '\0', '\0'],
        &['b', 'b', '\0', '\0', '\0', '\0'],
        &['c', 'c', 'c', 'c', '\0', '\0'],
        &['d', 'd', 'd', 'd', 'd', '\0'],
    ]);

    // Attempt to split beyond len which is illegal.
    let (_top_grid, bottom_grid) = grid.split(NonZeroUsize::new(5).expect("should not be zero"));
    assert!(bottom_grid.is_none());
}

/// Regression test for (CORE-1950), checks whether the grid splitting operation correctly
/// splits visible rows (appropriately handling scrollback history).
#[test]
fn test_split_grid_scrollback_visible_rows() {
    // Setup: Create and initialize the grid.
    let mut grid = GridHandler::new_for_test_with_scroll_limit(30, 6, 7);
    grid.grid_storage_mut().populate_from_array(&[
        // Comments describe rows _after_ resizes below.
        &['a', '0', '\0', '\0', '\0', '\0'], // history
        &['a', '1', '\0', '\0', '\0', '\0'], // history
        &['a', '2', '\0', '\0', '\0', '\0'], // history
        &['a', '3', '\0', '\0', '\0', '\0'], // visible row 0
        &['a', '4', '\0', '\0', '\0', '\0'], // visible row 1 <---- split above this row
        &['a', '5', '\0', '\0', '\0', '\0'], // visible row 2
        &['a', '6', '\0', '\0', '\0', '\0'], // visible row 3
    ]);
    grid.set_cursor_point(6, 2);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // Force rows to go into scrollback with consecutive resize operations.
    grid.resize(SizeInfo::new_without_font_metrics(18, 4));
    // We force 4 visible rows (the bottom rows from grid above).
    grid.resize(SizeInfo::new_without_font_metrics(4, 8));

    // Confirm the grid is in the expected state after resizing.
    assert_eq!(grid.visible_rows(), 4);
    assert_eq!(grid.total_rows(), 7);
    assert_eq!(grid.history_size(), 3);

    // Split the grid at the row containing "a4", so "a4\na5\na6" goes into the
    // bottom grid and the rest remains in the top grid.
    match grid.split(NonZeroUsize::new(4).expect("should not be zero")) {
        (grid, Some(bottom_grid)) => {
            // Check that visible rows were split correctly.
            assert_eq!(grid.total_rows(), 4);
            assert_eq!(grid.visible_rows(), 4);
            assert_eq!(grid.history_size(), 0);

            assert_eq!(bottom_grid.total_rows(), 3);
            assert_eq!(bottom_grid.visible_rows(), 3);
            assert_eq!(bottom_grid.history_size(), 0);

            // Verify character content for both grids.
            assert_cell_char_eq!(grid[0][1], '0');
            assert_cell_char_eq!(grid[1][1], '1');
            assert_cell_char_eq!(grid[2][1], '2');
            assert_cell_char_eq!(grid[3][1], '3');

            assert_cell_char_eq!(bottom_grid[0][1], '4');
            assert_cell_char_eq!(bottom_grid[1][1], '5');
            assert_cell_char_eq!(bottom_grid[2][1], '6');

            // Verify cursor positions.
            assert_eq!(
                grid.grid_storage().cursor.point,
                VisiblePoint {
                    row: VisibleRow(3),
                    col: 7
                }
            );
            assert_eq!(
                bottom_grid.grid_storage().cursor.point,
                VisiblePoint {
                    row: VisibleRow(2),
                    col: 2
                }
            );
        }
        _ => {
            panic!("Received None from split (split row exceeded Grid!)");
        }
    }
}

/// Regression test for checking whether row indices which are out of bounds of the visible rows
/// (but in bounds for total rows) are handled correctly i.e. splits are not allowed.
#[test]
fn test_split_grid_scrollback_visible_rows_out_of_bounds() {
    // Setup: Create and initialize the grid.
    let mut grid = GridHandler::new_for_test_with_scroll_limit(30, 6, 7);
    grid.grid_storage_mut().populate_from_array(&[
        // Comments describe rows _after_ resizes below.
        &['a', '0', '\0', '\0', '\0', '\0'], // history
        &['a', '1', '\0', '\0', '\0', '\0'], // history
        &['a', '2', '\0', '\0', '\0', '\0'], // history
        &['a', '3', '\0', '\0', '\0', '\0'], // visible row 0
        &['a', '4', '\0', '\0', '\0', '\0'], // visible row 1 <---- split above this row
        &['a', '5', '\0', '\0', '\0', '\0'], // visible row 2
        &['a', '6', '\0', '\0', '\0', '\0'], // visible row 3
    ]);
    grid.set_cursor_point(6, 2);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // Force rows to go into scrollback with consecutive resize operations.
    grid.resize(SizeInfo::new_without_font_metrics(18, 4));
    // We force 4 visible rows (the bottom rows from grid above).
    grid.resize(SizeInfo::new_without_font_metrics(4, 8));

    // Confirm the grid is in the expected state after resizing.
    assert_eq!(grid.visible_rows(), 4);
    assert_eq!(grid.total_rows(), 7);
    assert_eq!(grid.history_size(), 3);

    // Purposely split at a row which is out of bounds, to confirm behavior.
    let (_top_grid, bottom_grid) = grid.split(NonZeroUsize::new(7).expect("should not be zero"));
    assert!(bottom_grid.is_none());
}

#[test]
fn test_scrolling_up_updates_dirty_cells_range() {
    let mut grid = GridHandler::new_for_test_with_scroll_limit(3, 4, 2);
    grid.input_at_cursor("abcd");

    let pre_scroll_cursor_point = grid.cursor_point();

    assert!(
        grid.dirty_cells_range().is_none(),
        "input_at_cursor should clear dirty_cells_range"
    );
    assert_eq!(pre_scroll_cursor_point, Point { row: 1, col: 0 });

    grid.scroll_up(1);

    // Increasing the scroll limit by 1 will update the cursor position.
    assert_eq!(grid.cursor_point(), Point { row: 2, col: 0 });

    // The dirty cells range should have been updated to include everything
    // up to the new cursor point.
    let dirty_cells_range = grid
        .dirty_cells_range()
        .expect("dirty_cells_range should be non-empty");
    assert_eq!(dirty_cells_range.start(), &pre_scroll_cursor_point);
    assert_eq!(
        grid.cursor_point(),
        dirty_cells_range.end().wrapping_add(grid.columns(), 1)
    );
}

#[test]
pub fn test_cursor_cell() {
    let mut grid = GridStorage::new(3, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    grid.cursor.point = VisiblePoint {
        row: VisibleRow(2),
        col: 0,
    };

    let cell = grid.cursor_cell();
    assert_eq!(cell.c, 'c');
}

#[test]
pub fn test_cursor_cell_out_of_bounds() {
    let mut grid = GridStorage::new(3, 1, 0, ObfuscateSecrets::No);
    grid[0][0].c = 'a';
    grid[1][0].c = 'b';
    grid[2][0].c = 'c';
    let total_rows = grid.total_rows();
    let total_cols = grid.columns();

    // Row is out of bounds.
    grid.cursor.point = VisiblePoint {
        row: VisibleRow(total_rows + 1),
        col: 0,
    };

    let cell = grid.cursor_cell();
    assert_eq!(cell.c, 'c');

    // Column is out of bounds.
    grid.cursor.point = VisiblePoint {
        row: VisibleRow(0),
        col: total_cols + 1,
    };

    let cell = grid.cursor_cell();
    assert_eq!(cell.c, 'c');
}
