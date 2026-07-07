use std::any::Any;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warpui::elements::{
    AcceptedByDropTarget, Align, ConstrainedBox, Container, DragAxis, Draggable, DraggableState,
    DropTarget, DropTargetData, ParentElement, Rect, Stack,
};
use warpui::{AppContext, Element, Entity, TypedActionView, View};

#[derive(Default)]
pub struct RootView {
    basic_draggable_state: DraggableState,
    horizontal_draggable_state: DraggableState,
    vertical_draggable_state: DraggableState,
    clamped_draggable_state: DraggableState,
}

// Implement the entity trait.
impl Entity for RootView {
    type Event = ();
}

// Implement the view trait so RootView could be considered as a view.
impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    // Let's render a simple black rect background.
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(
                Align::new(
                    Draggable::new(
                        self.basic_draggable_state.clone(),
                        ConstrainedBox::new(
                            Rect::new()
                                .with_background_color(ColorU::new(255, 0, 0, 255))
                                .finish(),
                        )
                        .with_width(50.)
                        .with_height(50.)
                        .finish(),
                    )
                    .on_drag_start(|_, _, _| eprintln!("Regular Drag Start!"))
                    .on_drop(|_, _, _, drop_data| eprintln!("Regular Drop! Data: {drop_data:?}"))
                    .with_accepted_by_drop_target_fn(|_, _| AcceptedByDropTarget::Yes)
                    .with_drag_bounds_callback(|_, window_size| {
                        Some(RectF::new(Vector2F::zero(), window_size))
                    })
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Align::new(
                    Container::new({
                        let draggable = Draggable::new(
                            self.horizontal_draggable_state.clone(),
                            ConstrainedBox::new(
                                Rect::new()
                                    .with_background_color(ColorU::new(255, 0, 255, 255))
                                    .finish(),
                            )
                            .with_width(100.)
                            .with_height(50.)
                            .finish(),
                        )
                        .with_drag_axis(DragAxis::HorizontalOnly)
                        .on_drag_start(|_, _, _| eprintln!("Horizontal Drag Start!"))
                        .on_drop(|_, _, _, drop_data| {
                            eprintln!("Horizontal Drop! Drop data: {drop_data:?}")
                        })
                        .with_accepted_by_drop_target_fn(|_, _| AcceptedByDropTarget::Yes)
                        .finish();
                        DropTarget::new(draggable, DropTargetColor::Magenta).finish()
                    })
                    .with_margin_top(20.)
                    .finish(),
                )
                .top_center()
                .finish(),
            )
            .with_child(
                Align::new(
                    Container::new(
                        Draggable::new(
                            self.vertical_draggable_state.clone(),
                            ConstrainedBox::new(
                                Rect::new()
                                    .with_background_color(ColorU::new(0, 255, 255, 255))
                                    .finish(),
                            )
                            .with_width(50.)
                            .with_height(100.)
                            .finish(),
                        )
                        .with_drag_axis(DragAxis::VerticalOnly)
                        .on_drag_start(|_, _, _| eprintln!("Vertical Drag Start!"))
                        .on_drop(|_, _, _, _| eprintln!("Vertical Drop!"))
                        .with_accepted_by_drop_target_fn(|_, _| AcceptedByDropTarget::Yes)
                        .finish(),
                    )
                    .with_margin_right(20.)
                    .finish(),
                )
                .right()
                .finish(),
            )
            .with_child(
                Align::new(
                    Container::new({
                        let draggable = Draggable::new(
                            self.clamped_draggable_state.clone(),
                            ConstrainedBox::new(
                                Rect::new()
                                    .with_background_color(ColorU::new(255, 255, 0, 255))
                                    .finish(),
                            )
                            .with_width(50.)
                            .with_height(50.)
                            .finish(),
                        )
                        .with_drag_bounds(RectF::new(vec2f(20., 30.), vec2f(200., 400.)))
                        .on_drag_start(|_, _, _| eprintln!("Clamped Drag Start!"))
                        .on_drop(|_, _, _, drop_target_data| {
                            eprintln!("Clamped Drop! Data: {drop_target_data:?}")
                        })
                        .with_accepted_by_drop_target_fn(|_, _| AcceptedByDropTarget::Yes)
                        .finish();

                        DropTarget::new(draggable, DropTargetColor::Yellow).finish()
                    })
                    .with_margin_left(50.)
                    .with_margin_top(50.)
                    .finish(),
                )
                .top_left()
                .finish(),
            )
            .with_child(
                Align::new(
                    DropTarget::new(
                        ConstrainedBox::new(
                            Rect::new()
                                .with_background_color(ColorU::new(0, 0, 255, 255))
                                .finish(),
                        )
                        .with_width(100.)
                        .with_height(100.)
                        .finish(),
                        DropTargetColor::Blue,
                    )
                    .finish(),
                )
                .bottom_center()
                .finish(),
            )
            .finish()
    }
}

#[derive(Debug)]
enum DropTargetColor {
    Yellow,
    Blue,
    Magenta,
}

impl DropTargetData for DropTargetColor {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
