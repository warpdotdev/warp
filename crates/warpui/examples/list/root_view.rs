use std::sync::{Arc, Mutex};

use warpui::color::ColorU;
use warpui::elements::{
    Border, ConstrainedBox, Container, Fill, Flex, List, ListState, MainAxisSize, ParentElement,
    Rect, ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Stack, Text,
};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

pub struct RootView {
    font_family: FamilyId,
    list_state: ListState<()>,
    scroll_state: ScrollStateHandle,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = warpui::fonts::Cache::handle(ctx)
            .update(ctx, |cache, _| cache.load_system_font("Arial").unwrap());

        let list_state = ListState::new(move |i, _scroll_offset, _app| {
            println!("  📦 Creating element for item {i}"); // This should only appear for visible items!
            Self::make_list_item(i, font_family).finish()
        });
        let scroll_state = Arc::new(Mutex::new(Default::default()));

        // Add many items to demonstrate viewporting - only visible ones should be rendered
        println!("Creating List with 1000 items...");
        for _ in 0..=1000 {
            list_state.add_item();
        }
        println!(
            "✅ All 1000 items added to list state. Now only visible ones should be rendered."
        );

        RootView {
            font_family,
            list_state,
            scroll_state,
        }
    }

    fn make_list_item(index: usize, font_family: FamilyId) -> Container {
        // Alternate colors to make it easy to see which items are rendered
        let bg_color = if index.is_multiple_of(2) {
            ColorU::new(240, 240, 240, 255) // Light gray
        } else {
            ColorU::new(255, 255, 255, 255) // White
        };

        let border_color = if index.is_multiple_of(10) {
            ColorU::new(255, 0, 0, 255) // Red border for every 10th item
        } else {
            ColorU::new(200, 200, 200, 255) // Light gray border
        };

        let height = 50. * (index + 1) as f32;

        Container::new(
            ConstrainedBox::new(
                Flex::row()
                    .with_child(
                        Text::new_inline(format!("Item #{index}"), font_family, 16.)
                            .with_color(ColorU::black())
                            .finish(),
                    )
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(
                                Text::new_inline(
                                    if index.is_multiple_of(10) {
                                        " (MILESTONE)".to_string()
                                    } else {
                                        format!(" - Height: {height}px")
                                    },
                                    font_family,
                                    14.,
                                )
                                .with_color(ColorU::black())
                                .finish(),
                            )
                            .with_width(200.)
                            .finish(),
                        )
                        .finish(),
                    )
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            )
            .with_width(600.)
            .with_height(height)
            .finish(),
        )
        .with_background_color(bg_color)
        .with_border(Border::all(1.).with_border_color(border_color))
    }

    fn make_instructions(&self) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_child(
                        Text::new_inline(
                            "List Demo - 1000 Items".to_string(),
                            self.font_family,
                            20.,
                        )
                        .with_color(ColorU::black())
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            "This demonstrates viewporting: only visible items are rendered!"
                                .to_string(),
                            self.font_family,
                            14.,
                        )
                        .with_color(ColorU::black())
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            "🔍 Check the console output to see which items are being rendered."
                                .to_string(),
                            self.font_family,
                            14.,
                        )
                        .with_color(ColorU::black())
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            "🟥 Red borders mark milestone items (every 10th).".to_string(),
                            self.font_family,
                            14.,
                        )
                        .with_color(ColorU::black())
                        .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            "📏 Viewport height: 400px.".to_string(),
                            self.font_family,
                            12.,
                        )
                        .with_color(ColorU::black())
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(140.)
            .finish(),
        )
        .with_background_color(ColorU::new(230, 230, 255, 255))
        .with_border(Border::all(2.).with_border_color(ColorU::new(100, 100, 200, 255)))
        .finish()
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "ListRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::white()).finish())
            .with_child(
                Flex::column()
                    .with_child(self.make_instructions())
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(
                                Scrollable::vertical(
                                    self.scroll_state.clone(),
                                    List::new(self.list_state.clone()).finish_scrollable(),
                                    ScrollbarWidth::Auto,
                                    Fill::Solid(ColorU::new(150, 150, 150, 255)), // Non-active thumb
                                    Fill::Solid(ColorU::new(100, 100, 100, 255)), // Active thumb
                                    Fill::Solid(ColorU::new(240, 240, 240, 255)), // Track background
                                )
                                .finish(),
                            )
                            .with_height(400.) // Constrain the viewport height
                            .finish(),
                        )
                        .with_background_color(ColorU::new(250, 250, 250, 255))
                        .with_border(Border::all(2.).with_border_color(ColorU::black()))
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
