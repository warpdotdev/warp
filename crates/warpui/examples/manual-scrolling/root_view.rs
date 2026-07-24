use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui::color::ColorU;
use warpui::elements::new_scrollable::{
    AxisConfiguration, ClippedAxisConfiguration, DualAxisConfig, NewScrollableElement,
    ScrollableAppearance, ScrollableAxis,
};
use warpui::elements::{
    Axis, ChildView, ClippedScrollStateHandle, ConstrainedBox, NewScrollable, ParentElement, Point,
    Rect, ScrollData, ScrollStateHandle, ScrollbarWidth, Stack,
};
use warpui::keymap::FixedBinding;
use warpui::units::Pixels;
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

pub fn init(ctx: &mut AppContext) {
    use warpui::keymap::macros::*;

    // Add bindings to trigger actions in the subview.
    ctx.register_fixed_bindings([
        FixedBinding::new("up", SubViewAction::ScrollVertical(50.), id!("SubView")),
        FixedBinding::new("down", SubViewAction::ScrollVertical(-50.), id!("SubView")),
    ]);
}

pub struct RootView {
    // RootView "owns" a viewhandle to the subview.
    sub_view: ViewHandle<SubView>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Adding typed action view allows the view to receive keydown events.
        let sub_view = ctx.add_typed_action_view(|ctx| {
            let view = SubView::default();
            // Need the view to be focused for keydown actions to be dispatched to it.
            ctx.focus_self();
            view
        });
        Self { sub_view }
    }
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

    // Renders the child view of sub_view.
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.sub_view).finish()
    }
}

#[derive(Debug, Clone)]
pub enum SubViewAction {
    ScrollVertical(f32),
}

#[derive(Default)]
pub struct SubView {
    pub scroll_state_horizontal: ClippedScrollStateHandle,
    pub scroll_state_vertical: ScrollStateHandle,
    pub scroll_top: f32,
}

impl SubView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        SubView::default()
    }
}

impl Entity for SubView {
    type Event = ();
}
impl View for SubView {
    fn ui_name() -> &'static str {
        "SubView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let axis_config = DualAxisConfig::Manual {
            horizontal: AxisConfiguration::Clipped(ClippedAxisConfiguration {
                handle: self.scroll_state_horizontal.clone(),
                max_size: None,
                stretch_child: false,
            }),
            vertical: AxisConfiguration::Manual(self.scroll_state_vertical.clone()),
            child: ScrollableElement::new(self.scroll_top).finish_scrollable(),
        };
        let horizontally_scrollable = NewScrollable::horizontal_and_vertical(
            axis_config,
            ColorU::new(255, 255, 255, 150).into(),
            ColorU::white().into(),
            ColorU::new(100, 100, 100, 255).into(),
        )
        .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false));

        let constrained = ConstrainedBox::new(horizontally_scrollable.finish())
            .with_height(250.)
            .with_width(250.);

        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(constrained.finish())
            .finish()
    }
}
impl TypedActionView for SubView {
    type Action = SubViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SubViewAction::ScrollVertical(scroll_top) => {
                // 250. viewport + 7. scrollbar width.
                self.scroll_top = (self.scroll_top - *scroll_top).clamp(0., 257.);
            }
        };
        ctx.notify();
    }
}

struct ScrollableElement {
    size: Option<Vector2F>,
    origin: Option<Point>,
    scroll_top: f32,
}

impl ScrollableElement {
    fn new(scroll_top: f32) -> Self {
        Self {
            scroll_top,
            size: None,
            origin: None,
        }
    }
}

impl Element for ScrollableElement {
    fn layout(
        &mut self,
        constraint: warpui::SizeConstraint,
        _: &mut warpui::LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        let size = vec2f(
            constraint.max_along(Axis::Horizontal).min(500.),
            constraint.max_along(Axis::Vertical).min(500.),
        );
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut warpui::AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut warpui::PaintContext, _app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        let adjusted_origin = origin - vec2f(0., self.scroll_top);
        for i in 0..10 {
            for j in 0..10 {
                let color = (i + j) % 3;
                let color = if color == 0 {
                    ColorU::new(255, 0, 0, 255)
                } else if color == 1 {
                    ColorU::new(0, 255, 0, 255)
                } else {
                    ColorU::new(0, 0, 255, 255)
                };

                let cell_origin = adjusted_origin + vec2f(i as f32 * 50., j as f32 * 50.);
                ctx.scene
                    .draw_rect_with_hit_recording(RectF::new(cell_origin, vec2f(50., 50.)))
                    .with_background(color);
            }
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _: &warpui::event::DispatchedEvent,
        _: &mut warpui::EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}

impl NewScrollableElement for ScrollableElement {
    fn axis(&self) -> ScrollableAxis {
        ScrollableAxis::Vertical
    }

    fn scroll_data(&self, axis: Axis, _app: &AppContext) -> Option<ScrollData> {
        match axis {
            Axis::Horizontal => None,
            Axis::Vertical => Some(ScrollData {
                scroll_start: Pixels::new(self.scroll_top),
                visible_px: Pixels::new(self.size.unwrap().y()),
                total_size: Pixels::new(500.),
            }),
        }
    }

    fn scroll(&mut self, delta: warpui::units::Pixels, axis: Axis, ctx: &mut warpui::EventContext) {
        match axis {
            Axis::Horizontal => (),
            Axis::Vertical => {
                ctx.dispatch_typed_action(SubViewAction::ScrollVertical(delta.as_f32()))
            }
        }
    }

    fn axis_should_handle_scroll_wheel(&self, _axis: Axis) -> bool {
        true
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
