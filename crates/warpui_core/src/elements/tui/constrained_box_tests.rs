use super::TuiConstrainedBox;
use crate::elements::tui::{TuiConstraint, TuiElement, TuiInputLine, TuiRect, TuiSize, TuiText};

#[test]
fn caps_height_to_max_rows() {
    // A three-hard-line text wants three rows; the cap limits it to two.
    let three_lines = TuiText::new("a\nb\nc").truncate();
    assert_eq!(three_lines.desired_height(10), 3);

    let capped = TuiConstrainedBox::new(TuiText::new("a\nb\nc").truncate()).with_max_rows(2);
    assert_eq!(capped.desired_height(10), 2);
}

#[test]
fn caps_width_to_max_cols() {
    let mut capped = TuiConstrainedBox::new(TuiText::new("hello world")).with_max_cols(3);
    let size = capped.layout(TuiConstraint::loose(TuiSize::new(20, 5)));
    assert_eq!(size.width, 3);
}

#[test]
fn uncapped_axes_pass_through() {
    // Only the height is capped, so the width follows the child's natural size.
    let mut capped = TuiConstrainedBox::new(TuiText::new("hello").truncate()).with_max_rows(1);
    let size = capped.layout(TuiConstraint::loose(TuiSize::new(20, 5)));
    assert_eq!(size, TuiSize::new(5, 1));
}

#[test]
fn forwards_cursor_position_when_uncapped() {
    let capped = TuiConstrainedBox::new(TuiInputLine::new("ab", 2));
    assert_eq!(
        capped.cursor_position(TuiRect::new(0, 0, 10, 1)),
        Some((2, 0)),
    );
}
