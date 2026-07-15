use warpui::EntityIdMap;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiScreenPosition, TuiSize,
};
use warpui_core::{App, AppContext};

use super::TuiTerminalSizeElement;

/// A leaf that fills its constraint and retains the laid-out size.
struct FillElement {
    size: Option<TuiSize>,
}

impl TuiElement for FillElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        self.size = Some(constraint.max);
        constraint.max
    }

    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }
}

#[test]
fn layout_measures_and_after_layout_publishes_the_size() {
    App::test((), |app| async move {
        app.read(|app| {
            let (resize_tx, resize_rx) = async_channel::unbounded();
            let mut element =
                TuiTerminalSizeElement::new(resize_tx, FillElement { size: None }.finish());
            let expected_size = TuiSize::new(42, 8);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            let size = element.layout(TuiConstraint::loose(expected_size), &mut layout_ctx, app);
            assert_eq!(size, expected_size);
            // `layout` only measures — it must not publish a resize.
            assert!(
                resize_rx.try_recv().is_err(),
                "layout should not publish a resize"
            );

            // `after_layout` publishes the settled size exactly once.
            element.after_layout(&mut layout_ctx, app);
            assert_eq!(resize_rx.try_recv().unwrap(), expected_size);
            assert!(
                resize_rx.try_recv().is_err(),
                "after_layout should publish the resize exactly once"
            );
        });
    });
}

#[test]
fn dropped_receiver_does_not_panic() {
    App::test((), |app| async move {
        app.read(|app| {
            let (resize_tx, resize_rx) = async_channel::unbounded::<TuiSize>();
            drop(resize_rx);
            let mut element =
                TuiTerminalSizeElement::new(resize_tx, FillElement { size: None }.finish());
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };

            element.layout(
                TuiConstraint::loose(TuiSize::new(10, 4)),
                &mut layout_ctx,
                app,
            );
            // The consumer being gone (e.g. a torn-down session) is not an error.
            element.after_layout(&mut layout_ctx, app);
        });
    });
}
