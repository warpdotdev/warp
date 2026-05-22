use warpui::color::ColorU;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, Flex, ParentElement,
    Rect, ScrollbarWidth, Stack,
};
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext};

#[derive(Default)]
pub struct RootView {
    pub clipped_scroll_state: ClippedScrollStateHandle,
}

impl RootView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        RootView::default()
    }
}

impl Entity for RootView {
    type Event = ();
}
impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let mut column = Flex::column();

        // Create 10 rows, where each row has 10 rectanges (each of size 50*50).
        // By the end, `column` will be 500 * 500.
        for i in 0..10 {
            let mut row = Flex::row();
            for j in 0..10 {
                let color = (i + j) % 3;
                let color = if color == 0 {
                    ColorU::new(255, 0, 0, 255)
                } else if color == 1 {
                    ColorU::new(0, 255, 0, 255)
                } else {
                    ColorU::new(0, 0, 255, 255)
                };

                row.add_child(
                    Container::new(
                        ConstrainedBox::new(Rect::new().finish())
                            .with_height(50.)
                            .with_width(50.)
                            .finish(),
                    )
                    .with_background_color(color)
                    .finish(),
                );
            }
            column.add_child(row.finish());
        }

        // Change this to [`ClippedScrollable::vertical`] to see what a vertically scrollable element looks like.
        let horizontally_scrollable = ClippedScrollable::horizontal(
            self.clipped_scroll_state.clone(),
            column.finish(),
            ScrollbarWidth::Auto,
            ColorU::white().into(),
            ColorU::white().into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        let constrained = ConstrainedBox::new(horizontally_scrollable.finish())
            .with_height(250.)
            .with_width(250.);

        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(constrained.finish())
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
