use pathfinder_geometry::vector::Vector2F;

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint,
};

/// An element that constrains its child to a percentage of the available size, either width or height.
pub struct Percentage {
    width_percentage: Option<f32>,
    height_percentage: Option<f32>,
    child: Box<dyn Element>,
}

impl Percentage {
    /// Constrain width of child to a percentage of the available max width.
    pub fn width(percentage: f32, child: Box<dyn Element>) -> Self {
        Self {
            width_percentage: Some(percentage),
            height_percentage: None,
            child,
        }
    }

    /// Constrain height of child to a percentage of the available max height.
    pub fn height(percentage: f32, child: Box<dyn Element>) -> Self {
        Self {
            width_percentage: None,
            height_percentage: Some(percentage),
            child,
        }
    }

    /// Constrain both width and height of child to a percentage of the available max width and height.
    pub fn both(width_percentage: f32, height_percentage: f32, child: Box<dyn Element>) -> Self {
        Self {
            width_percentage: Some(width_percentage),
            height_percentage: Some(height_percentage),
            child,
        }
    }
}

impl Element for Percentage {
    fn layout(
        &mut self,
        mut constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        if let Some(width_percentage) = self.width_percentage {
            let width_percentage = width_percentage.clamp(0.0, 1.0);
            constraint.max.set_x(constraint.max.x() * width_percentage);
        }
        if let Some(height_percentage) = self.height_percentage {
            let height_percentage = height_percentage.clamp(0.0, 1.0);
            constraint.max.set_y(constraint.max.y() * height_percentage);
        }
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &crate::event::DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}
