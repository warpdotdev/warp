use std::collections::HashMap;

use super::TuiChildView;
use crate::elements::tui::{
    TuiBuffer, TuiElement, TuiPresentationContext, TuiRect, TuiSize, TuiText,
};
use crate::EntityId;

#[test]
fn embeds_and_renders_the_stub_at_the_given_area() {
    let view = TuiChildView::from_rendered(EntityId::from_usize(1), Box::new(TuiText::new("Z")));

    let mut buffer = TuiBuffer::new(TuiSize::new(3, 1));
    // Render offset one column in: the embedded glyph must land at x = 1.
    view.render(TuiRect::new(1, 0, 2, 1), &mut buffer);
    assert_eq!(buffer.to_lines(), vec![" Z "]);
}

#[test]
fn delegates_desired_height_to_the_embedded_element() {
    let view = TuiChildView::from_rendered(
        EntityId::from_usize(1),
        Box::new(TuiText::new("AB\nCD").truncate()),
    );
    assert_eq!(view.desired_height(2), 2);
}

#[test]
fn present_records_the_child_as_a_child_of_the_current_view() {
    let root = EntityId::from_usize(7);
    let child = EntityId::from_usize(8);
    let mut parent_by_child = HashMap::new();

    {
        let mut ctx = TuiPresentationContext::new(root, &mut parent_by_child);
        let mut view = TuiChildView::from_rendered(child, Box::new(()));
        view.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&child), Some(&root));
}

#[test]
fn present_nests_grandchildren_under_their_immediate_parent() {
    let root = EntityId::from_usize(1);
    let child = EntityId::from_usize(2);
    let grandchild = EntityId::from_usize(3);
    let mut parent_by_child = HashMap::new();

    {
        let mut ctx = TuiPresentationContext::new(root, &mut parent_by_child);
        let nested = TuiChildView::from_rendered(grandchild, Box::new(()));
        let mut view = TuiChildView::from_rendered(child, Box::new(nested));
        view.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&child), Some(&root));
    assert_eq!(parent_by_child.get(&grandchild), Some(&child));
}
