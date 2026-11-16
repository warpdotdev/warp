use std::time::Duration;

use pathfinder_geometry::vector::Vector2F;

use super::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext, Point,
    SizeConstraint,
};
use crate::event::DispatchedEvent;

/// A wrapper element that triggers periodic repaints at a fixed interval.
///
/// Wrap any child element in `LiveElement` to ensure the view repaints on a
/// timer, which is useful for content that changes over time (e.g. an elapsed
/// duration counter). The repaint cycle is self-sustaining: each `paint` call
/// schedules the next repaint.
pub struct LiveElement {
    child: Box<dyn Element>,
    repaint_interval: Duration,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl LiveElement {
    pub fn new(child: Box<dyn Element>, repaint_interval: Duration) -> Self {
        Self {
            child,
            repaint_interval,
            size: None,
            origin: None,
        }
    }
}

impl Element for LiveElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = self.child.layout(constraint, ctx, app);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        ctx.repaint_after(self.repaint_interval);
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn as_selectable_element(&self) -> Option<&dyn super::SelectableElement> {
        self.child.as_selectable_element()
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}
