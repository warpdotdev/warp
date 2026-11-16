use warpui::color::ColorU;
use warpui::elements::{
    Border, ConstrainedBox, Container, Expanded, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Rect, Shrinkable, Stack, Text,
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

    fn make_label(&self, label: String) -> Box<dyn Element> {
        Flex::row()
            .with_child(Text::new_inline(label, self.font_family, 16.).finish())
            .finish()
    }

    fn make_expanded_row(&self) -> Flex {
        Flex::row()
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Text::new_inline("Fixed 100", self.font_family, 16.).finish(),
                    )
                    .with_width(100.)
                    .with_height(50.)
                    .finish(),
                )
                .with_background_color(ColorU::new(255, 0, 0, 255))
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Max Width 500, Min 200", self.font_family, 16.)
                                .finish(),
                        )
                        .with_max_width(500.)
                        .with_min_width(200.)
                        .with_height(50.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(0, 150, 0, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Text::new_inline("Fixed 100", self.font_family, 16.).finish(),
                    )
                    .with_width(100.)
                    .with_height(50.)
                    .finish(),
                )
                .with_background_color(ColorU::new(0, 0, 255, 255))
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
    }

    fn make_shrinkable_row(&self) -> Flex {
        Flex::row()
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Text::new_inline("Fixed 100", self.font_family, 16.).finish(),
                    )
                    .with_width(100.)
                    .with_height(50.)
                    .finish(),
                )
                .with_background_color(ColorU::new(255, 0, 0, 255))
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Max Width 500, Min 200", self.font_family, 16.)
                                .finish(),
                        )
                        .with_max_width(500.)
                        .with_min_width(200.)
                        .with_height(50.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(0, 150, 0, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Text::new_inline("Fixed 100", self.font_family, 16.).finish(),
                    )
                    .with_width(100.)
                    .with_height(50.)
                    .finish(),
                )
                .with_background_color(ColorU::new(0, 0, 255, 255))
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
    }

    fn make_multiple_expanded_row(&self) -> Flex {
        Flex::row()
            .with_child(
                Expanded::new(
                    3.0,
                    Container::new(Text::new_inline("Flex: 3.0", self.font_family, 16.).finish())
                        .with_background_color(ColorU::new(255, 0, 0, 255))
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.0,
                    Container::new(Text::new_inline("Flex: 1.0", self.font_family, 16.).finish())
                        .with_background_color(ColorU::new(0, 150, 0, 255))
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    2.0,
                    Container::new(Text::new_inline("Flex: 2.0", self.font_family, 16.).finish())
                        .with_background_color(ColorU::new(0, 0, 255, 255))
                        .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
    }

    fn make_multiple_expanded_with_constraints_row(&self) -> Flex {
        Flex::row()
            .with_child(
                Expanded::new(
                    1.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Min 100, Max 200", self.font_family, 16.).finish(),
                        )
                        .with_max_width(200.)
                        .with_min_width(100.)
                        .with_height(50.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(255, 0, 0, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Min 100, Max 200", self.font_family, 16.).finish(),
                        )
                        .with_max_width(200.)
                        .with_min_width(100.)
                        .with_height(50.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(0, 150, 0, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Min 100, Max 200", self.font_family, 16.).finish(),
                        )
                        .with_max_width(200.)
                        .with_min_width(100.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(0, 0, 255, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Min)
    }

    fn make_multiple_expanded_with_constraints_varying_flex_row(&self) -> Flex {
        Flex::row()
            .with_child(
                Expanded::new(
                    3.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Min 100, Max 200, Flex: 3.0", self.font_family, 16.)
                                .finish(),
                        )
                        .with_max_width(200.)
                        .with_min_width(100.)
                        .with_height(50.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(255, 0, 0, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Min 100, Max 200, Flex: 1.0", self.font_family, 16.)
                                .finish(),
                        )
                        .with_max_width(200.)
                        .with_min_width(100.)
                        .with_height(50.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(0, 150, 0, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    2.0,
                    Container::new(
                        ConstrainedBox::new(
                            Text::new_inline("Min 100, Max 200, Flex: 2.0", self.font_family, 16.)
                                .finish(),
                        )
                        .with_max_width(200.)
                        .with_min_width(100.)
                        .finish(),
                    )
                    .with_background_color(ColorU::new(0, 0, 255, 255))
                    .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Min)
    }
}

impl Entity for RootView {
    type Event = ();
}
impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    // Let's render a simple black rect background.
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let row_expanded = Container::new(
            self.make_expanded_row()
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();
        let row_shrinkable = Container::new(
            self.make_shrinkable_row()
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_margin_bottom(32.)
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let multiple_expanded_row = Container::new(
            self.make_multiple_expanded_row()
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let multiple_expanded_with_constraints_row = Container::new(
            self.make_multiple_expanded_with_constraints_row()
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let multiple_expanded_with_constraints_varying_flex_row = Container::new(
            self.make_multiple_expanded_with_constraints_varying_flex_row()
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(
                Container::new(
                    Flex::column()
                        .with_child(self.make_label("Expanded - FlexFit::Tight".to_owned()))
                        .with_child(row_expanded)
                        .with_child(
                            self.make_label(
                                "Shrinkable (Old Expanded) - FlexFit::Loose ".to_owned(),
                            ),
                        )
                        .with_child(row_shrinkable)
                        .with_child(self.make_label("Multiple Expanded with varying flex amounts".to_owned()))
                        .with_child(multiple_expanded_row)
                        .with_child(self.make_label("Multiple Expanded with constraints in Flex with MainAxisSize::Min".to_owned()))
                        .with_child(multiple_expanded_with_constraints_row)
                        .with_child(self.make_label("Multiple Expanded with constraints and varying flex amounts in Flex with MainAxisSize::Min".to_owned()))
                        .with_child(self.make_label("Note that this results in children beginning to shrink at different times and rates".to_owned()))
                        .with_child(multiple_expanded_with_constraints_varying_flex_row)
                        .finish(),
                )
                .with_margin_top(32.)
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
