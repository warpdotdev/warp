use super::*;

fn cell(c: char) -> Cell {
    let mut cell = Cell::default();
    cell.c = c;
    cell
}

#[test]
fn test_cursor() {
    macro_rules! assert_cursor_contents_eq {
        ($c:expr_2021, $cursor:ident) => {
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
