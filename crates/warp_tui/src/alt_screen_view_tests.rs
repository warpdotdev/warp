use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::TerminalModel;
use warpui::EntityIdMap;
use warpui_core::elements::tui::{TuiConstraint, TuiElement, TuiLayoutContext, TuiSize};
use warpui_core::App;

use super::AltScreenElement;

#[test]
fn layout_fills_the_allocated_pane() {
    App::test((), |app| async move {
        app.read(|app| {
            let model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
            let mut element = AltScreenElement::new(model);
            let expected_size = TuiSize::new(42, 8);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = element.layout(TuiConstraint::loose(expected_size), &mut layout_ctx, app);
            assert_eq!(size, expected_size);
            // The laid-out size is retained for the terminal-content wrapper
            // and mouse hit-testing to read back.
            assert_eq!(element.size(), Some(expected_size));
        });
    });
}
