use crate::elements::Point;
use crate::event::DispatchedEvent;
use crate::geometry::vector::Vector2F;
use crate::{
    AfterLayoutContext, AppContext, ClipBounds, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// Internal elements used to support the `add_overlay_child` and `add_positioned_overlay_child`
/// APIs within the `Stack`. It is a thin wrapper around the child, creating a new Overlay layer
/// and painting the child within that layer, so that it is drawn above the normal UI elements.
pub(super) struct Overlay {
    child: Box<dyn Element>,
}

impl Overlay {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self { child }
    }
}

impl Element for Overlay {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        ctx.scene.start_overlay_layer(ClipBounds::None);
        self.child.paint(origin, ctx, app);
        ctx.scene.stop_layer();
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
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}
