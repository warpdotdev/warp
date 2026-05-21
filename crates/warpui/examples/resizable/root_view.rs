use warpui::color::ColorU;
use warpui::elements::{
    resizable_state_handle, Container, CrossAxisAlignment, DragBarSide, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Rect, Resizable, ResizableStateHandle, Shrinkable, Stack, Text,
};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

pub struct RootView {
    font_family: FamilyId,
    left_panel_state: ResizableStateHandle,
    top_panel_state: ResizableStateHandle,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = warpui::fonts::Cache::handle(ctx)
            .update(ctx, |cache, _| cache.load_system_font("Arial").unwrap());

        // Initialize resizable state handles
        let left_panel_state = resizable_state_handle(250.0);
        let top_panel_state = resizable_state_handle(150.0);

        RootView {
            font_family,
            left_panel_state,
            top_panel_state,
        }
    }

    fn make_panel_content(&self, text: String, color: ColorU) -> Box<dyn Element> {
        Container::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Text::new_inline(text, self.font_family, 16.)
                        .with_color(ColorU::white())
                        .finish(),
                )
                .finish(),
        )
        .with_background_color(color)
        .with_uniform_padding(10.)
        .finish()
    }

    fn make_info_text(&self, text: String) -> Box<dyn Element> {
        Text::new_inline(text, self.font_family, 14.)
            .with_color(ColorU::white())
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
        // Create main column for vertical layout
        let mut main_column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Top panel with bottom-side dragbar for vertical resizing
        let top_panel = Container::new(
            Resizable::new(
                self.top_panel_state.clone(),
                self.make_panel_content(
                    "Top Panel\n(Drag bottom edge)".to_string(),
                    ColorU::new(200, 100, 200, 255),
                ),
            )
            .with_dragbar_side(DragBarSide::Bottom)
            .with_dragbar_color(warpui::elements::Fill::Solid(ColorU::new(0, 255, 255, 200)))
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .on_start_resizing(|_, _| {
                eprintln!("Top panel: Started resizing");
            })
            .on_end_resizing(|_, _| {
                eprintln!("Top panel: Finished resizing");
            })
            .with_bounds_callback(Box::new(|window_size| {
                let min_height = 100.0;
                let max_height = window_size.y() * 0.5;
                (min_height, max_height.max(min_height))
            }))
            .finish(),
        )
        .finish();

        main_column.add_child(top_panel);

        // Create the row with CrossAxisAlignment::Stretch
        let mut main_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Left panel with right-side dragbar - added directly to the row
        let left_panel = Container::new(
            Resizable::new(
                self.left_panel_state.clone(),
                self.make_panel_content(
                    "Left Panel\n(Drag right edge)".to_string(),
                    ColorU::new(100, 100, 200, 255),
                ),
            )
            .with_dragbar_side(DragBarSide::Right)
            .with_dragbar_color(warpui::elements::Fill::Solid(ColorU::new(255, 255, 0, 200)))
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .on_start_resizing(|_, _| {
                eprintln!("Left panel: Started resizing");
            })
            .on_end_resizing(|_, _| {
                eprintln!("Left panel: Finished resizing");
            })
            .with_bounds_callback(Box::new(|window_size| {
                let min_width = 150.0;
                let max_width = window_size.x() * 0.6;
                (min_width, max_width.max(min_width))
            }))
            .finish(),
        )
        .finish();

        main_row.add_child(left_panel);

        // Right content area - wrapped in Shrinkable to fill remaining space
        let right_content = Container::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(20.)
                .with_child(
                    Text::new_inline("Resizable Example", self.font_family, 24.)
                        .with_color(ColorU::white())
                        .finish(),
                )
                .with_child(self.make_info_text("Yellow bar: Horizontal resize (left)".to_string()))
                .with_child(self.make_info_text("Cyan bar: Vertical resize (top)".to_string()))
                .with_child(self.make_info_text("Check terminal for events".to_string()))
                .finish(),
        )
        .with_background_color(ColorU::new(50, 50, 50, 255))
        .with_uniform_padding(20.)
        .finish();

        main_row.add_child(Shrinkable::new(1.0, right_content).finish());

        main_column.add_child(Shrinkable::new(1.0, main_row.finish()).finish());

        // Main layout
        Stack::new()
            .with_child(
                Rect::new()
                    .with_background_color(ColorU::new(30, 30, 30, 255))
                    .finish(),
            )
            .with_child(Shrinkable::new(1.0, main_column.finish()).finish())
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
