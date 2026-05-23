use super::*;

#[test]
fn location_ordering() {
    assert!(Point::new(0, 0) == Point::new(0, 0));
    assert!(Point::new(1, 0) > Point::new(0, 0));
    assert!(Point::new(0, 1) > Point::new(0, 0));
    assert!(Point::new(1, 1) > Point::new(0, 0));
    assert!(Point::new(1, 1) > Point::new(0, 1));
    assert!(Point::new(1, 1) > Point::new(1, 0));
}

#[test]
fn wrapping_sub() {
    let num_cols = 42;
    let point = Point::new(0, 13);

    let result = point.wrapping_sub(num_cols, 1);

    assert_eq!(result, Point::new(0, point.col - 1));
}

#[test]
fn wrapping_sub_wrap() {
    let num_cols = 42;
    let point = Point::new(1, 0);

    let result = point.wrapping_sub(num_cols, 1);

    assert_eq!(result, Point::new(0, num_cols - 1));
}

#[test]
fn wrapping_sub_clamp() {
    let num_cols = 42;
    let point = Point::new(0, 0);

    let result = point.wrapping_sub(num_cols, 1);

    assert_eq!(result, point);
}

#[test]
fn wrapping_add() {
    let num_cols = 42;
    let point = Point::new(0, 13);

    let result = point.wrapping_add(num_cols, 1);

    assert_eq!(result, Point::new(0, point.col + 1));
}

#[test]
fn wrapping_add_wrap() {
    let num_cols = 42;
    let point = Point::new(0, num_cols - 1);

    let result = point.wrapping_add(num_cols, 1);

    assert_eq!(result, Point::new(1, 0));
}

#[test]
fn add_absolute() {
    let point = Point::new(0, 13);

    let result = point.add_absolute(&(1, 42), Boundary::Clamp, 1);

    assert_eq!(result, Point::new(0, point.col + 1));
}

#[test]
fn add_absolute_wrapline() {
    let point = Point::new(1, 41);

    let result = point.add_absolute(&(2, 42), Boundary::Clamp, 1);

    assert_eq!(result, Point::new(0, 0));
}

#[test]
fn add_absolute_multiline_wrapline() {
    let point = Point::new(2, 9);

    let result = point.add_absolute(&(3, 10), Boundary::Clamp, 11);

    assert_eq!(result, Point::new(0, 0));
}

#[test]
fn add_absolute_clamp() {
    let point = Point::new(0, 41);

    let result = point.add_absolute(&(1, 42), Boundary::Clamp, 1);

    assert_eq!(result, point);
}

#[test]
fn add_absolute_wrap() {
    let point = Point::new(0, 41);

    let result = point.add_absolute(&(3, 42), Boundary::Wrap, 1);

    assert_eq!(result, Point::new(2, 0));
}

#[test]
fn add_absolute_multiline_wrap() {
    let point = Point::new(0, 9);

    let result = point.add_absolute(&(3, 10), Boundary::Wrap, 11);

    assert_eq!(result, Point::new(1, 0));
}

#[test]
fn sub_absolute() {
    let point = Point::new(0, 13);

    let result = point.sub_absolute(&(1, 42), Boundary::Clamp, 1);

    assert_eq!(result, Point::new(0, point.col - 1));
}

#[test]
fn sub_absolute_wrapline() {
    let point = Point::new(0, 0);

    let result = point.sub_absolute(&(2, 42), Boundary::Clamp, 1);

    assert_eq!(result, Point::new(1, 41));
}

#[test]
fn sub_absolute_multiline_wrapline() {
    let point = Point::new(0, 0);

    let result = point.sub_absolute(&(3, 10), Boundary::Clamp, 11);

    assert_eq!(result, Point::new(2, 9));
}

#[test]
fn sub_absolute_wrap() {
    let point = Point::new(2, 0);

    let result = point.sub_absolute(&(3, 42), Boundary::Wrap, 1);

    assert_eq!(result, Point::new(0, 41));
}

#[test]
fn sub_absolute_multiline_wrap() {
    let point = Point::new(2, 0);

    let result = point.sub_absolute(&(3, 10), Boundary::Wrap, 11);

    assert_eq!(result, Point::new(1, 9));
}

#[test]
fn test_point_difference() {
    let a = Point::new(3, 10);
    assert_eq!(a.distance(30, &a), 0);

    let b = Point::new(3, 6);
    assert_eq!(a.distance(30, &b), 4);
    assert_eq!(b.distance(30, &a), 4);

    let c = Point::new(4, 2);
    assert_eq!(a.distance(30, &c), 22);
    assert_eq!(c.distance(30, &a), 22);
}
