use super::*;
use warpui::{
    elements::{ChildView, Empty, SavePosition},
    platform::WindowStyle,
    App, ViewHandle,
};

struct TipContainer {
    tips_view: ViewHandle<TipsView>,
}

impl TipContainer {
    const POSITION_ID: &'static str = "position_id";
    fn new(tips_completed: ModelHandle<TipsCompleted>, ctx: &mut ViewContext<Self>) -> Self {
        let tips_view = ctx.add_typed_action_view(move |ctx| {
            TipsView::new(tips_completed, Self::POSITION_ID.to_owned(), ctx)
        });
        Self { tips_view }
    }
}

impl Entity for TipContainer {
    type Event = ();
}

impl View for TipContainer {
    fn ui_name() -> &'static str {
        "TipContainer"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Stack::new()
            // Add a child that represents the anchor for the welcome tips.
            .with_child(SavePosition::new(Empty::new().finish(), Self::POSITION_ID).finish())
            // Add the welcome tips view.
            .with_child(ChildView::new(&self.tips_view).finish())
            .finish()
    }
}

impl TypedActionView for TipContainer {
    type Action = ();
}

#[test]
fn test_render_tip_view() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let tips_completed = app.add_model(|_| TipsCompleted::default());
        let (_window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TipContainer::new(tips_completed, ctx)
        });

        app.update(|_| {
            // This will force a redraw of the window, which lays out the
            // window, including the tips view.
        });
    });
}
