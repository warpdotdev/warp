use super::TuiChildView;
use crate::elements::tui::{TuiBufferExt, TuiElement, TuiPresentationContext, TuiRect, TuiText};
use crate::presenter::tui::TuiPresenter;
use crate::{App, EntityId, EntityIdMap};

#[test]
fn embeds_and_renders_the_stub_at_the_given_area() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let view_id = EntityId::from_usize(1);
            let mut presenter = TuiPresenter::new();
            presenter
                .rendered_views
                .insert(view_id, Box::new(TuiText::new("Z")));
            let view = TuiChildView::for_view_id(view_id);
            let frame = presenter.present_element(view.finish(), TuiRect::new(1, 0, 2, 1), app_ctx);
            assert_eq!(frame.buffer.to_lines(), vec![" Z "]);
        });
    });
}

#[test]
fn present_records_the_child_as_a_child_of_the_current_view() {
    let root = EntityId::from_usize(7);
    let child = EntityId::from_usize(8);
    let mut rendered_views = EntityIdMap::default();
    let mut parent_by_child = EntityIdMap::default();

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
    let mut rendered_views = EntityIdMap::default();
    let mut parent_by_child = EntityIdMap::default();

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
