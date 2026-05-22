//! Module containing the definition of [`DebugElement`].

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint,
};
use crate::event::DispatchedEvent;
use crate::scene::{Border, Dash};

/// A debug element that draws a dashed around its child. Intended for quick visual debugging.
pub struct DebugElement {
    child: Box<dyn Element>,
    color: ColorU,
    border_width: f32,
    dash: Dash,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

/// Options for configuration of the [`DebugElement`].
#[derive(Default)]
pub struct DebugOptions {
    /// The color to use for the border. Defaults to red.
    pub color_override: Option<ColorU>,
    /// The width of the border. Defaults to 2.0.
    pub border_width_override: Option<f32>,
    /// The dash pattern to use for the border. Defaults to a 4.0 dash and 2.0 gap.
    pub dash_override: Option<Dash>,
}

impl DebugElement {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self::new_with_options(child, DebugOptions::default())
    }

    pub fn new_with_options(child: Box<dyn Element>, options: DebugOptions) -> Self {
        Self {
            child,
            color: options
                .color_override
                .unwrap_or(ColorU::new(255, 0, 0, 255)),
            border_width: options.border_width_override.unwrap_or(2.0),
            dash: options.dash_override.unwrap_or(Dash {
                dash_length: 4.0,
                gap_length: 2.0,
                force_consistent_gap_length: true,
            }),
            size: None,
            origin: None,
        }
    }
}

impl Element for DebugElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let child_size = self.child.layout(constraint, ctx, app);
        self.size = Some(child_size);
        child_size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        self.child.paint(origin, ctx, app);

        let size = self.size.unwrap_or(Vector2F::zero());

        // Draw a dashed border around the child without including the size of the
        // border in the overall size of the element.
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(origin, size))
            .with_border(
                Border::all(self.border_width)
                    .with_border_color(self.color)
                    .with_dashed_border(self.dash),
            );
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

pub trait Debug {
    fn debug(self) -> Box<dyn Element>;

    fn debug_with_options(self, options: DebugOptions) -> Box<dyn Element>;
}

impl Debug for Box<dyn Element> {
    fn debug(self) -> Box<dyn Element> {
        DebugElement::new(self).finish()
    }

    fn debug_with_options(self, options: DebugOptions) -> Box<dyn Element> {
        Box::new(DebugElement::new_with_options(self, options))
    }
}
