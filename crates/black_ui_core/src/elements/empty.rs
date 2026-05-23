use pathfinder_geometry::vector::Vector2F;

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint,
};
use crate::event::DispatchedEvent;

#[derive(Default)]
pub struct Empty {
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl Empty {
    pub fn new() -> Self {
        Self {
            size: None,
            origin: None,
        }
    }
}

impl Element for Empty {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        // Set the size of the element to be the max constraint. If the max constraint is unbounded
        // use the min constraint to avoid rendering an unbounded-sized element.
        let max_constraint = constraint.max;

        let x = if max_constraint.x().is_infinite() {
            constraint.min.x()
        } else {
            max_constraint.x()
        };

        let y = if max_constraint.y().is_infinite() {
            constraint.min.y()
        } else {
            max_constraint.y()
        };

        let size = Vector2F::new(x, y);

        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
