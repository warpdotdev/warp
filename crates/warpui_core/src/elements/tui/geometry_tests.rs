use super::*;

#[test]
fn constraint_clamps_each_axis_independently() {
    let constraint = TuiConstraint::new(TuiSize::new(2, 1), TuiSize::new(10, 4));

    assert_eq!(constraint.clamp(TuiSize::new(7, 3)), TuiSize::new(7, 3));
    assert_eq!(constraint.clamp(TuiSize::new(99, 99)), TuiSize::new(10, 4));
    assert_eq!(constraint.clamp(TuiSize::new(0, 0)), TuiSize::new(2, 1));
    assert_eq!(constraint.constrain_width(99), 10);
    assert_eq!(constraint.constrain_height(0), 1);
}

#[test]
fn tight_and_loose_constraints() {
    let tight = TuiConstraint::tight(TuiSize::new(5, 5));
    assert_eq!(tight.clamp(TuiSize::new(1, 9)), TuiSize::new(5, 5));

    let loose = TuiConstraint::loose(TuiSize::new(8, 3));
    assert_eq!(loose.clamp(TuiSize::new(4, 9)), TuiSize::new(4, 3));
    assert_eq!(loose.clamp(TuiSize::ZERO), TuiSize::ZERO);
}

#[test]
fn inset_shrinks_on_all_sides_and_saturates() {
    assert_eq!(TuiRect::new(2, 2, 10, 6).inset(1), TuiRect::new(3, 3, 8, 4));
    // Too small to inset: collapses to zero extent rather than wrapping.
    assert_eq!(TuiRect::new(0, 0, 1, 1).inset(2), TuiRect::new(2, 2, 0, 0));
}

#[test]
fn split_top_tiles_the_rect_exactly() {
    let rect = TuiRect::new(0, 0, 8, 5);
    let (top, rest) = rect.split_top(2);
    assert_eq!(top, TuiRect::new(0, 0, 8, 2));
    assert_eq!(rest, TuiRect::new(0, 2, 8, 3));

    // Over-tall split is clamped; the remainder is empty but well-formed.
    let (top, rest) = rect.split_top(99);
    assert_eq!(top, rect);
    assert_eq!(rest, TuiRect::new(0, 5, 8, 0));
    assert!(rest.is_empty());
}

#[test]
fn split_left_tiles_the_rect_exactly() {
    let rect = TuiRect::new(1, 1, 8, 5);
    let (left, rest) = rect.split_left(3);
    assert_eq!(left, TuiRect::new(1, 1, 3, 5));
    assert_eq!(rest, TuiRect::new(4, 1, 5, 5));
}

#[test]
fn right_and_bottom_saturate() {
    let rect = TuiRect::new(u16::MAX, u16::MAX, 4, 4);
    assert_eq!(rect.right(), u16::MAX);
    assert_eq!(rect.bottom(), u16::MAX);
}
