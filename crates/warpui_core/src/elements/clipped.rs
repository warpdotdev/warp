use pathfinder_geometry::vector::Vector2F;

use super::Point;
use crate::{
    event::DispatchedEvent, AfterLayoutContext, AppContext, ClipBounds, Element, EventContext,
    LayoutContext, PaintContext, SizeConstraint,
};
use std::any::Any;

/// Element that clips a child to its bounds
pub struct Clipped {
    origin: Option<Point>,
    size: Option<Vector2F>,
    child: Box<dyn Element>,
}

impl Clipped {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self {
            origin: None,
            size: None,
            child,
        }
    }

    pub fn sized(child: Box<dyn Element>, size: Vector2F) -> Self {
        Self {
            origin: None,
            size: Some(size),
            child,
        }
    }
}

impl Element for Clipped {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let origin_point = Point::from_vec2f(origin, ctx.scene.z_index());
        self.origin = Some(origin_point);

        // Get current clip bounds (if any) to ensure that the next layer respects them.
        let current_bounds = ctx.scene.visible_rect(
            origin_point,
            self.size()
                .expect("Clipped element should have a size at time of paint"),
        );

        // Clipping works by creating a separate layer for an element with clip bounds.
        // If current_bounds is None, this means that we shouldn't paint anything.
        if let Some(bounds) = current_bounds {
            ctx.scene.start_layer(ClipBounds::BoundedBy(bounds));
            self.child.paint(origin, ctx, app);
            ctx.scene.stop_layer();
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // Only dispatch the event to the child if it has been painted.
        self.child.origin().is_some() && self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.size.or_else(|| self.child.size())
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        self.child.parent_data()
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}

#[cfg(test)]
#[path = "clipped_tests.rs"]
mod tests;
