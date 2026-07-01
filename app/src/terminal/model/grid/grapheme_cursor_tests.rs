use super::*;

fn cell(c: char) -> Cell {
    let mut cell = Cell::default();
    cell.c = c;
    cell
}

#[test]
fn test_cursor() {
    macro_rules! assert_cursor_contents_eq {
        ($c:expr, $cursor:ident) => {
            let item = $cursor
                .current_item()
                .expect("cursor location should be valid");
            assert_eq!(&cell($c), item.cell());
        };
    }

    let mut grid = GridHandler::new_for_test(5, 5);
    for i in 0u8..5u8 {
        for j in 0u8..5u8 {
            grid.grid_storage_mut()[usize::from(i)][usize::from(j)] = cell((i * 5 + j) as char);
        }
    }

    let mut cursor = grid.grapheme_cursor_from(Point { row: 0, col: 0 }, Wrap::All);

    cursor.move_backward();
    assert!(cursor.current_item().is_none());
    cursor.move_forward();
    assert_cursor_contents_eq!(0u8 as char, cursor);

    cursor.move_forward();
    assert_cursor_contents_eq!(1u8 as char, cursor);
    assert_eq!(Some(1), cursor.current_item().map(|item| item.point().col));
    assert_eq!(Some(0), cursor.current_item().map(|item| item.point().row));

    cursor.move_forward();
    assert_cursor_contents_eq!(2u8 as char, cursor);

    cursor.move_forward();
    assert_cursor_contents_eq!(3u8 as char, cursor);

    cursor.move_forward();
    assert_cursor_contents_eq!(4u8 as char, cursor);

    // Test line-wrapping.
    cursor.move_forward();
    assert_cursor_contents_eq!(5u8 as char, cursor);
    assert_eq!(Some(0), cursor.current_item().map(|item| item.point().col));
    assert_eq!(Some(1), cursor.current_item().map(|item| item.point().row));

    cursor.move_backward();
    assert_cursor_contents_eq!(4u8 as char, cursor);
    assert_eq!(Some(4), cursor.current_item().map(|item| item.point().col));
    assert_eq!(Some(0), cursor.current_item().map(|item| item.point().row));

    // Make sure iter.cell() returns the current iterator position.
    assert_cursor_contents_eq!(4u8 as char, cursor);

    // Test that iter ends at end of grid.
    let mut final_cursor = grid.grapheme_cursor_from(Point { row: 4, col: 4 }, Wrap::All);
    final_cursor.move_forward();
    assert!(final_cursor.current_item().is_none());

    final_cursor.move_backward();
    assert_cursor_contents_eq!(24u8 as char, final_cursor);
    final_cursor.move_backward();
    assert_cursor_contents_eq!(23u8 as char, final_cursor);
}

/// Regression test locking in that `move_forward`/`move_backward` already
/// generalize to ANY cluster span (not just the CJK width=2 case) for
/// free: both recurse internally while landing on a spacer cell, so a
/// single call walks all the way to the cluster's base/next real cell
/// regardless of how many spacer cells are in between. This must keep
/// working for the variable-width-cell rewrite (spans up to 8, not just 2).
#[test]
fn test_cursor_skips_over_variable_width_span() {
    let mut grid = GridHandler::new_for_test(2, 10);

    // Row: [base(span=4)][spacer][spacer][spacer]['x']['y']...
    grid.grid_storage_mut()[0][0] = cell('X');
    grid.grid_storage_mut()[0][0].set_span(4);
    for col in 1..4 {
        grid.grid_storage_mut()[0][col]
            .flags_mut()
            .insert(cell::Flags::WIDE_CHAR_SPACER);
    }
    grid.grid_storage_mut()[0][4] = cell('y');

    // Starting fresh at col 0 and moving forward should land on 'y' at col 4
    // in a single move_forward() call, skipping all 3 spacer cells.
    let mut cursor = grid.grapheme_cursor_from(Point { row: 0, col: 0 }, Wrap::All);
    cursor.move_forward();
    assert_eq!(Some(4), cursor.current_item().map(|item| item.point().col));
    assert_eq!(
        Some('y'),
        cursor.current_item().map(|item| item.cell().c)
    );

    // Starting on a spacer cell (col 2, the middle of the span) should snap
    // back to the base cell at col 0 -- constructor-time snapping must also
    // handle a span wider than 2.
    let snapped = grid.grapheme_cursor_from(Point { row: 0, col: 2 }, Wrap::All);
    assert_eq!(Some(0), snapped.current_item().map(|item| item.point().col));
    assert_eq!(Some('X'), snapped.current_item().map(|item| item.cell().c));

    // Moving backward from 'y' should land back on the base cell at col 0
    // in a single move_backward() call, skipping all 3 spacer cells.
    let mut back_cursor = grid.grapheme_cursor_from(Point { row: 0, col: 4 }, Wrap::All);
    back_cursor.move_backward();
    assert_eq!(Some(0), back_cursor.current_item().map(|item| item.point().col));
    assert_eq!(Some('X'), back_cursor.current_item().map(|item| item.cell().c));
}
