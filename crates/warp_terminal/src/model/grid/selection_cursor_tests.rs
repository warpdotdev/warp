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
