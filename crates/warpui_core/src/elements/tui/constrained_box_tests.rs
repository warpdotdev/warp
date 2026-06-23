use std::collections::HashMap;

use super::TuiConstrainedBox;
use crate::elements::tui::{
    TuiConstraint, TuiElement, TuiInputLine, TuiLayoutContext, TuiRect, TuiSize, TuiText,
};

#[test]
fn caps_height_to_max_rows() {
    // A three-hard-line text wants three rows; the cap limits it to two.
    // Verify via layout: uncapped gives 3 rows, capped gives 2.
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    let mut uncapped = TuiText::new("a\nb\nc").truncate();
    assert_eq!(uncapped.layout(TuiConstraint::loose(TuiSize::new(10, 10)), &mut ctx).height, 3);

    let mut capped = TuiConstrainedBox::new(TuiText::new("a\nb\nc").truncate()).with_max_rows(2);
    assert_eq!(capped.layout(TuiConstraint::loose(TuiSize::new(10, 10)), &mut ctx).height, 2);
}

#[test]
fn caps_width_to_max_cols() {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    let mut capped = TuiConstrainedBox::new(TuiText::new("hello world")).with_max_cols(3);
    let size = capped.layout(TuiConstraint::loose(TuiSize::new(20, 5)), &mut ctx);
    assert_eq!(size.width, 3);
}

#[test]
fn uncapped_axes_pass_through() {
    // Only the height is capped, so the width follows the child's natural size.
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    let mut capped = TuiConstrainedBox::new(TuiText::new("hello").truncate()).with_max_rows(1);
    let size = capped.layout(TuiConstraint::loose(TuiSize::new(20, 5)), &mut ctx);
    assert_eq!(size, TuiSize::new(5, 1));
}

#[test]
fn forwards_cursor_position_when_uncapped() {
    let mut rendered_views = HashMap::new();
    let mut ctx = TuiLayoutContext { rendered_views: &mut rendered_views };
    let mut capped = TuiConstrainedBox::new(TuiInputLine::new("ab", 2));
    capped.layout(TuiConstraint::tight(TuiSize::new(10, 1)), &mut ctx);
    assert_eq!(
        capped.cursor_position(TuiRect::new(0, 0, 10, 1), &mut ctx),
        Some((2, 0)),
    );
}
