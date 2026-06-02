use pathfinder_color::ColorU;
use warpui::elements::Rect;
use warpui::{AppContext, Element, Entity, TypedActionView, View};

pub struct BlurredView {}

impl Entity for BlurredView {
    type Event = ();
}
impl View for BlurredView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    /// Renders a transparent red rectangle. The blur effect is applied on the window (see
    /// `open_new()`.
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Rect::new()
            .with_background_color(ColorU::new(255, 0, 0, 50))
            .finish()
    }
}

impl TypedActionView for BlurredView {
    type Action = ();
}
