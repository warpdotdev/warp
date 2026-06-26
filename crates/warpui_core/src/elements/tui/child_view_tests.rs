use std::collections::HashMap;

use super::TuiChildView;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiElement, TuiLayoutContext, TuiPresentationContext, TuiRect, TuiText,
};
use crate::EntityId;

#[test]
fn embeds_and_renders_the_stub_at_the_given_area() {
    let mut rendered_views = HashMap::new();
    let view = TuiChildView::from_rendered(
        EntityId::from_usize(1),
        Box::new(TuiText::new("Z")),
        &mut rendered_views,
    );

    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 3, 1));
    let mut ctx = TuiLayoutContext {
        rendered_views: &mut rendered_views,
    };
    // Render offset one column in: the embedded glyph must land at x = 1.
    view.render(TuiRect::new(1, 0, 2, 1), &mut buffer, &mut ctx);
    assert_eq!(buffer.to_lines(), vec![" Z "]);
}

#[test]
fn present_records_the_child_as_a_child_of_the_current_view() {
    let root = EntityId::from_usize(7);
    let child = EntityId::from_usize(8);
    let mut rendered_views = HashMap::new();
    let mut parent_by_child = HashMap::new();

    {
        let mut ctx = TuiPresentationContext::new(root, &mut rendered_views, &mut parent_by_child);
        let mut view = TuiChildView::from_rendered(child, Box::new(()), ctx.rendered_views);
        view.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&child), Some(&root));
}

#[test]
fn present_nests_grandchildren_under_their_immediate_parent() {
    let root = EntityId::from_usize(1);
    let child = EntityId::from_usize(2);
    let grandchild = EntityId::from_usize(3);
    let mut rendered_views = HashMap::new();
    let mut parent_by_child = HashMap::new();

    {
        let mut ctx = TuiPresentationContext::new(root, &mut rendered_views, &mut parent_by_child);
        // grandchild must be in rendered_views so the nested TuiChildView
        // node can find it during the present pass.
        TuiChildView::from_rendered(grandchild, Box::new(()), ctx.rendered_views);
        // The child's element is a TuiChildView that embeds the grandchild.
        let nested_child_view = Box::new(TuiChildView::for_view_id(grandchild));
        let mut view = TuiChildView::from_rendered(child, nested_child_view, ctx.rendered_views);
        view.present(&mut ctx);
    }

    assert_eq!(parent_by_child.get(&child), Some(&root));
    assert_eq!(parent_by_child.get(&grandchild), Some(&child));
}
