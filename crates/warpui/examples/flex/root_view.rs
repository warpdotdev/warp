use warpui::color::ColorU;
use warpui::elements::{
    Border, ConstrainedBox, Container, Flex, MainAxisAlignment, MainAxisSize, ParentElement, Rect,
    Shrinkable, Stack, Text, Wrap,
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

    fn make_row(&self) -> Flex {
        Flex::row()
            .with_spacing(20.)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("1", self.font_family, 16.).finish())
                        .with_width(200.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(255, 0, 0, 255))
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("2", self.font_family, 16.).finish())
                        .with_width(200.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(0, 255, 0, 255))
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("3", self.font_family, 16.).finish())
                        .with_width(200.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(0, 0, 255, 255))
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
    }

    fn make_wrap_row(&self) -> Wrap {
        Wrap::row()
            .with_spacing(20.)
            .with_run_spacing(10.)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("1", self.font_family, 16.).finish())
                        .with_width(200.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(255, 0, 0, 255))
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("2", self.font_family, 16.).finish())
                        .with_width(200.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(0, 255, 0, 255))
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("3", self.font_family, 16.).finish())
                        .with_width(200.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(0, 0, 255, 255))
                .finish(),
            )
    }

    fn make_column(&self) -> Flex {
        Flex::column()
            .with_spacing(20.)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("1", self.font_family, 16.).finish())
                        .with_width(20.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(255, 0, 0, 255))
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("2", self.font_family, 16.).finish())
                        .with_width(20.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(0, 255, 0, 255))
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Text::new_inline("3", self.font_family, 16.).finish())
                        .with_width(20.)
                        .with_height(50.)
                        .finish(),
                )
                .with_background_color(ColorU::new(0, 0, 255, 255))
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
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
        let row_between = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_between_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_evenly = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_evenly_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_center = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_center_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_start = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_start_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_end = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let row_end_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_row()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_between = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_between_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_evenly = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_evenly_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_center = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_center_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_start = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_start_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_end = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let column_end_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_column()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let wrap_row = Container::new(
            Shrinkable::new(
                1.,
                self.make_wrap_row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        let wrap_row_reverse = Container::new(
            Shrinkable::new(
                1.,
                self.make_wrap_row()
                    .with_reverse_orientation()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .finish(),
        )
        .with_border(Border::all(2.).with_border_color(ColorU::white()))
        .finish();

        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(
                Flex::column()
                    .with_child(self.make_label("Space Between".to_owned()))
                    .with_child(row_between)
                    .with_child(row_between_reverse)
                    .with_child(self.make_label("Space Evenly".to_owned()))
                    .with_child(row_evenly)
                    .with_child(row_evenly_reverse)
                    .with_child(self.make_label("Center".to_owned()))
                    .with_child(row_center)
                    .with_child(row_center_reverse)
                    .with_child(self.make_label("Start".to_owned()))
                    .with_child(row_start)
                    .with_child(row_start_reverse)
                    .with_child(self.make_label("End".to_owned()))
                    .with_child(row_end)
                    .with_child(row_end_reverse)
                    .with_child(self.make_label("Wrap Row".to_owned()))
                    .with_child(wrap_row)
                    .with_child(wrap_row_reverse)
                    .with_child(
                        ConstrainedBox::new(
                            Flex::row()
                                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_child(self.make_label("Space Between ->".to_owned()))
                                .with_child(column_between)
                                .with_child(column_between_reverse)
                                .with_child(self.make_label("Space Evenly ->".to_owned()))
                                .with_child(column_evenly)
                                .with_child(column_evenly_reverse)
                                .with_child(self.make_label("Center ->".to_owned()))
                                .with_child(column_center)
                                .with_child(column_center_reverse)
                                .with_child(self.make_label("Start ->".to_owned()))
                                .with_child(column_start)
                                .with_child(column_start_reverse)
                                .with_child(self.make_label("End ->".to_owned()))
                                .with_child(column_end)
                                .with_child(column_end_reverse)
                                .finish(),
                        )
                        .with_max_height(300.)
                        .finish(),
                    )
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
