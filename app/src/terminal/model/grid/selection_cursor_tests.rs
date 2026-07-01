use super::*;

#[test]
fn test_cursor() {
    let grid = GridHandler::new_for_test(5, 5);

    let mut cursor = SelectionCursor::new(&grid, Point::new(0, 0));
    assert_eq!(cursor.position(), Some(Point::new(0, 0)));

    // Test moving the cursor up above the top of the grid and then back down.
    cursor.move_up();
    assert_eq!(cursor.position(), None);
    cursor.move_down();
    assert_eq!(cursor.position(), Some(Point::new(0, 0)));

    // Test moving the cursor backward from the first cell, then forward again.
    cursor.move_backward();
    assert_eq!(cursor.position(), None);
    cursor.move_forward();
    assert_eq!(cursor.position(), Some(Point::new(0, 0)));

    cursor.move_forward();
    assert_eq!(cursor.position(), Some(Point::new(0, 1)));

    cursor.move_forward();
    assert_eq!(cursor.position(), Some(Point::new(0, 2)));

    cursor.move_forward();
    assert_eq!(cursor.position(), Some(Point::new(0, 3)));

    cursor.move_forward();
    assert_eq!(cursor.position(), Some(Point::new(0, 4)));

    // Test line-wrapping both forward and backward across a line boundary.
    cursor.move_forward();
    assert_eq!(cursor.position(), Some(Point::new(1, 0)));
    cursor.move_backward();
    assert_eq!(cursor.position(), Some(Point::new(0, 4)));

    cursor = SelectionCursor::new(&grid, Point::new(4, 4));
    assert_eq!(cursor.position(), Some(Point::new(4, 4)));

    // Test moving the cursor down from the bottom of the grid, then back up.
    cursor.move_down();
    assert_eq!(cursor.position(), None);
    cursor.move_up();
    assert_eq!(cursor.position(), Some(Point::new(4, 4)));

    // Test moving the cursor forward from the end of the grid, then backward again.
    cursor.move_forward();
    assert_eq!(cursor.position(), None);
    cursor.move_backward();
    assert_eq!(cursor.position(), Some(Point::new(4, 4)));
}

#[test]
fn test_cursor_over_two_wide_char() {
    // Locks in the existing (pre-variable-width-cell) convention for a
    // fixed CJK width=2 span: moving forward from before the pair lands on
    // its LAST cell (the spacer); moving backward from after the pair
    // lands on its FIRST cell (the base). This must not regress when the
    // logic generalizes to spans > 2.
    let mut grid = GridHandler::new_for_test(2, 10);
    grid.grid_storage_mut()[0][2].set_span(2);
    grid.grid_storage_mut()[0][3]
        .flags_mut()
        .insert(crate::terminal::model::cell::Flags::WIDE_CHAR_SPACER);

    let mut cursor = SelectionCursor::new(&grid, Point::new(0, 1));
    cursor.move_forward();
    assert_eq!(
        cursor.position(),
        Some(Point::new(0, 3)),
        "forward across a 2-wide char should land on its trailing spacer"
    );

    let mut cursor = SelectionCursor::new(&grid, Point::new(0, 4));
    cursor.move_backward();
    assert_eq!(
        cursor.position(),
        Some(Point::new(0, 2)),
        "backward across a 2-wide char should land on its base cell"
    );
}

#[test]
fn test_cursor_over_variable_width_span() {
    // Generalization check: a 5-cell span (e.g. a measured Indic cluster)
    // must behave the same way a 2-wide char does, just with more cells to
    // skip -- forward lands on the LAST cell, backward lands on the FIRST.
    let mut grid = GridHandler::new_for_test(2, 10);
    grid.grid_storage_mut()[0][2].set_span(5);
    for col in 3..7 {
        grid.grid_storage_mut()[0][col]
            .flags_mut()
            .insert(crate::terminal::model::cell::Flags::WIDE_CHAR_SPACER);
    }

    let mut cursor = SelectionCursor::new(&grid, Point::new(0, 1));
    cursor.move_forward();
    assert_eq!(
        cursor.position(),
        Some(Point::new(0, 6)),
        "forward across a 5-wide span should land on its last cell (col 6)"
    );

    let mut cursor = SelectionCursor::new(&grid, Point::new(0, 7));
    cursor.move_backward();
    assert_eq!(
        cursor.position(),
        Some(Point::new(0, 2)),
        "backward across a 5-wide span should land on its base cell (col 2)"
    );
}

#[test]
fn test_cursor_starting_mid_span_does_not_overshoot() {
    // Regression test for a real bug found while generalizing this logic:
    // if the cursor starts DIRECTLY on a spacer cell (e.g. constructed from
    // a raw click/serialized position, which SelectionCursor::new does not
    // validate against), moving backward must land on the span's base --
    // not overshoot past it into the preceding, unrelated cell.
    let mut grid = GridHandler::new_for_test(2, 10);
    grid.grid_storage_mut()[0][2].set_span(4);
    for col in 3..6 {
        grid.grid_storage_mut()[0][col]
            .flags_mut()
            .insert(crate::terminal::model::cell::Flags::WIDE_CHAR_SPACER);
    }

    // Start directly on the first spacer cell (col 3) -- one cell into the span.
    let mut cursor = SelectionCursor::new(&grid, Point::new(0, 3));
    cursor.move_backward();
    assert_eq!(
        cursor.position(),
        Some(Point::new(0, 2)),
        "must land on the base cell, not overshoot into col 1"
    );
}
