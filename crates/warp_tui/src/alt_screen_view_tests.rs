use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::TerminalModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{TuiConstraint, TuiElement, TuiLayoutContext, TuiSize};
use warpui_core::App;

use super::AltScreenElement;

#[test]
fn layout_measures_and_after_layout_commits_the_resize() {
    App::test((), |app| async move {
        app.read(|app| {
            let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
            let (resize_tx, resize_rx) = async_channel::unbounded();
            let mut element = AltScreenElement::new(model, resize_tx);
            let expected_size = TuiSize::new(42, 8);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = element.layout(TuiConstraint::loose(expected_size), &mut layout_ctx, app);
            assert_eq!(size, expected_size);
            // `layout` only measures — it must not fire the PTY resize.
            assert!(
                resize_rx.try_recv().is_err(),
                "layout should not commit a resize"
            );

            // `after_layout` commits the settled size exactly once.
            element.after_layout(&mut layout_ctx, app);
            assert_eq!(resize_rx.try_recv().unwrap(), expected_size);
            assert!(
                resize_rx.try_recv().is_err(),
                "after_layout should commit the resize exactly once"
            );
        });
    });
}
