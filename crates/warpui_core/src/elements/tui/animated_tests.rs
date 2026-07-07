use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use super::TuiAnimated;
use crate::elements::tui::test_support::with_paint_context;
use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiRect,
    TuiSize, TuiText,
};
use crate::{App, EntityIdMap};

#[test]
fn rebuilds_its_frame_on_every_layout_pass_and_requests_repaints() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let builds = Rc::new(Cell::new(0));
            let builds_in_closure = builds.clone();
            let mut animated = TuiAnimated::new(Duration::from_millis(50), move || {
                builds_in_closure.set(builds_in_closure.get() + 1);
                TuiText::new(format!("pass {}", builds_in_closure.get())).finish()
            });

            // Each layout+render pass paints a frame built for that pass and
            // requests the next repaint.
            for expected in ["pass 1", "pass 2"] {
                let mut rendered_views = EntityIdMap::default();
                let mut layout_ctx = TuiLayoutContext {
                    rendered_views: &mut rendered_views,
                };
                animated.layout(
                    TuiConstraint::loose(TuiSize::new(10, 1)),
                    &mut layout_ctx,
                    app_ctx,
                );

                let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 10, 1));
                let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
                animated.render(TuiRect::new(0, 0, 10, 1), &mut buffer, &mut paint_ctx);
                assert_eq!(buffer.to_lines(), vec![format!("{expected:<10}")]);
                assert!(paint_ctx.requested_repaint_at().is_some());
            }
            assert_eq!(builds.get(), 2);
        });
    });
}

#[test]
fn paints_nothing_before_its_first_layout() {
    let animated = TuiAnimated::new(Duration::from_millis(50), || {
        TuiText::new("content").finish()
    });
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 7, 1));
    with_paint_context(|ctx| animated.render(TuiRect::new(0, 0, 7, 1), &mut buffer, ctx));
    assert_eq!(buffer.to_lines(), vec!["       "]);
}
