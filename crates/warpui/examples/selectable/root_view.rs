//! A UI sample demonstrating how the SelectableArea element can be used.

use warpui::color::ColorU;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, Flex, ParentElement, Rect, SelectableArea,
    SelectionHandle, Stack, Text,
};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

pub struct RootView {
    sub_view: ViewHandle<SelectableExampleView>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let sub_view = ctx.add_view(|ctx| {
            let font_family = warpui::fonts::Cache::handle(ctx).update(ctx, |cache, _| {
                cache.load_system_font("Menlo").expect("Should load Menlo")
            });
            let view = SelectableExampleView {
                font_family,
                selectable_area_state_handle_1: Default::default(),
                selectable_area_state_handle_2: Default::default(),
            };
            ctx.focus_self();
            view
        });
        Self { sub_view }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.sub_view).finish()
    }
}

pub struct SelectableExampleView {
    font_family: FamilyId,
    selectable_area_state_handle_1: SelectionHandle,
    selectable_area_state_handle_2: SelectionHandle,
}

impl Entity for SelectableExampleView {
    type Event = ();
}

impl View for SelectableExampleView {
    fn ui_name() -> &'static str {
        "SelectableExampleView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(
                Flex::column()
                    .with_child(
                        SelectableArea::new(
                            self.selectable_area_state_handle_1.clone(),
                            |selection_args, _, _| {
                                println!("SELECTED TEXT - {:?}", selection_args.selection);
                            },
                            Container::new(
                                ConstrainedBox::new(
                                    Flex::row()
                                        .with_child(
                                            Container::new(
                                                Flex::column()
                                                    .with_children([
                                                        Container::new(
                                                            Text::new(
                                                                "HELLO WORLD 1",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                        ConstrainedBox::new(
                                                            Text::new(
                                                                "HELLO WORLD 2",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_width(400.)
                                                        .with_height(400.)
                                                        .finish(),
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 3",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                    ])
                                                    .finish(),
                                            )
                                            .with_horizontal_margin(10.)
                                            .with_uniform_padding(10.)
                                            .with_border(
                                                Border::all(1.0).with_border_fill(ColorU::new(
                                                    255, 194, 255, 255,
                                                )),
                                            )
                                            .finish(),
                                        )
                                        .with_child(
                                            Container::new(
                                                Flex::column()
                                                    .with_children([
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 4",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                        ConstrainedBox::new(
                                                            Text::new(
                                                                "HELLO WORLD 5",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .finish(),
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 6",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                    ])
                                                    .finish(),
                                            )
                                            .with_horizontal_margin(10.)
                                            .with_uniform_padding(10.)
                                            .with_border(
                                                Border::all(1.0).with_border_fill(ColorU::new(
                                                    255, 194, 255, 255,
                                                )),
                                            )
                                            .finish(),
                                        )
                                        .finish(),
                                )
                                .finish(),
                            )
                            .with_uniform_padding(100.)
                            .finish(),
                        )
                        .finish(),
                    )
                    .with_child(
                        SelectableArea::new(
                            self.selectable_area_state_handle_2.clone(),
                            |selection_args, _, _| {
                                println!("SELECTED TEXT - {:?}", selection_args.selection);
                            },
                            Container::new(
                                ConstrainedBox::new(
                                    Flex::row()
                                        .with_child(
                                            Container::new(
                                                Flex::column()
                                                    .with_children([
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 11",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                        ConstrainedBox::new(
                                                            Text::new(
                                                                "HELLO WORLD 22",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .finish(),
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 33",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                    ])
                                                    .finish(),
                                            )
                                            .with_horizontal_margin(10.)
                                            .with_uniform_padding(10.)
                                            .with_border(
                                                Border::all(1.0).with_border_fill(ColorU::new(
                                                    255, 194, 255, 255,
                                                )),
                                            )
                                            .finish(),
                                        )
                                        .with_child(
                                            Container::new(
                                                Flex::column()
                                                    .with_children([
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 44",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                        ConstrainedBox::new(
                                                            Text::new(
                                                                "HELLO 👀👀👀 WORLD 55",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_width(400.)
                                                        .with_height(400.)
                                                        .finish(),
                                                        Container::new(
                                                            Text::new_inline(
                                                                "HELLO WORLD 66 👀",
                                                                self.font_family,
                                                                16.,
                                                            )
                                                            .finish(),
                                                        )
                                                        .with_vertical_margin(10.)
                                                        .finish(),
                                                    ])
                                                    .finish(),
                                            )
                                            .with_horizontal_margin(10.)
                                            .with_uniform_padding(10.)
                                            .with_border(
                                                Border::all(1.0).with_border_fill(ColorU::new(
                                                    255, 194, 255, 255,
                                                )),
                                            )
                                            .finish(),
                                        )
                                        .finish(),
                                )
                                .finish(),
                            )
                            .with_uniform_margin(100.)
                            .finish(),
                        )
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
