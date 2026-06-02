use warpui::color::ColorU;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Percentage, Rect, Shrinkable, Text,
};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

pub struct RootView {
    font_family: FamilyId,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = warpui::fonts::Cache::handle(ctx)
            .update(ctx, |cache, _| cache.load_system_font("Arial").unwrap());
        RootView { font_family }
    }

    fn make_label(&self, text: &str) -> Box<dyn Element> {
        Text::new_inline(text.to_string(), self.font_family, 16.).finish()
    }

    fn make_width(&self) -> Box<dyn Element> {
        let bar = |pct: f32, color: ColorU| {
            Shrinkable::new(
                1.,
                Container::new(
                    ConstrainedBox::new(
                        Align::new(
                            Percentage::width(
                                pct,
                                Rect::new().with_background_color(color).finish(),
                            )
                            .finish(),
                        )
                        .left()
                        .finish(),
                    )
                    .with_height(12.)
                    .finish(),
                )
                .with_border(Border::all(1.).with_border_color(ColorU::white()))
                .finish(),
            )
            .finish()
        };

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(self.make_label("Width 25/50/75%:"))
            .with_child(bar(0.25, ColorU::new(200, 80, 80, 255)))
            .with_child(bar(0.50, ColorU::new(80, 200, 80, 255)))
            .with_child(bar(0.75, ColorU::new(80, 120, 220, 255)))
            .finish()
    }

    fn make_height(&self) -> Box<dyn Element> {
        let bar = |pct: f32, color: ColorU| {
            Shrinkable::new(
                1.,
                Container::new(
                    Align::new(
                        Percentage::height(pct, Rect::new().with_background_color(color).finish())
                            .finish(),
                    )
                    .top_left()
                    .finish(),
                )
                .with_border(Border::all(1.).with_border_color(ColorU::white()))
                .finish(),
            )
            .finish()
        };
        Shrinkable::new(
            1.,
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::End)
                .with_spacing(12.)
                .with_child(self.make_label("Height 30/60/90%:"))
                .with_child(bar(0.30, ColorU::new(200, 80, 80, 255)))
                .with_child(bar(0.60, ColorU::new(80, 200, 80, 255)))
                .with_child(bar(0.90, ColorU::new(80, 120, 220, 255)))
                .finish(),
        )
        .finish()
    }

    fn make_both(&self) -> Box<dyn Element> {
        let cell = |w: f32, h: f32, color: ColorU| {
            let child =
                Percentage::both(w, h, Rect::new().with_background_color(color).finish()).finish();
            Shrinkable::new(
                1.,
                Container::new(Align::new(child).top_left().finish())
                    .with_border(Border::all(1.).with_border_color(ColorU::white()))
                    .finish(),
            )
            .finish()
        };

        Shrinkable::new(
            1.,
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(12.)
                .with_child(self.make_label("Both 60% x 50%, 80% x 30%:"))
                .with_child(cell(0.6, 0.5, ColorU::new(200, 80, 80, 255)))
                .with_child(cell(0.8, 0.3, ColorU::new(80, 200, 80, 255)))
                .finish(),
        )
        .finish()
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
        Container::new(
            Flex::column()
                .with_spacing(16.)
                .with_child(self.make_width())
                .with_child(self.make_height())
                .with_child(self.make_both())
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_background_color(ColorU::black())
        .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
