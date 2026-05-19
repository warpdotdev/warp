use pathfinder_geometry::{rect::RectF, vector::Vector2F};

use super::Point;
use crate::{
    event::DispatchedEvent, AfterLayoutContext, AppContext, ClipBounds, Element, Event,
    EventContext, LayoutContext, PaintContext, SizeConstraint,
};

type DismissCallback = Box<dyn FnMut(&mut EventContext, &AppContext)>;

/// Element that is used to dismiss its child element.
/// Clicking on this element is equivalent to clicking outside the child element.
pub struct Dismiss {
    child: Box<dyn Element>,
    dismiss_handler: Option<DismissCallback>,
    origin: Option<Point>,
    /// Whether or not the element should make the rest of the window unresponsive. All mouse events
    /// are handled by the [`Dismiss`] rather than being propagated further down in the element
    /// hierarchy.
    prevent_interaction_with_other_elements: bool,
}

impl Dismiss {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self {
            child,
            dismiss_handler: None,
            origin: None,
            prevent_interaction_with_other_elements: false,
        }
    }

    /// Attach a handler for when the dismiss is clicked
    pub fn on_dismiss<F>(mut self, handler: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext),
    {
        self.dismiss_handler = Some(Box::new(handler));
        self
    }

    /// Prevents interactions with any other elements outside of the [`Dismiss`]. All events are
    /// handled by this element and are _not_ propagated further down the element hierarchy.
    pub fn prevent_interaction_with_other_elements(mut self) -> Self {
        self.prevent_interaction_with_other_elements = true;
        self
    }
}

impl Element for Dismiss {
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
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        if !self.prevent_interaction_with_other_elements {
            // Create a new layer for the contents so that we can distinguish click events that happen
            // outside of the child element we want to dismiss
            ctx.scene.start_layer(ClipBounds::ActiveLayer);
            self.child.paint(origin, ctx, app);
            ctx.scene.stop_layer();
        } else {
            // Create an invisible rect underneath the child that spans the window and prevents all
            // underlayed elements from responding to events, until a click occurs.
            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(Vector2F::zero(), ctx.window_size));
            self.child.paint(origin, ctx, app);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.child.dispatch_event(event, ctx, app) {
            return true;
        }

        let z_index = self.z_index().unwrap();

        let event_at_z_index = event.at_z_index(z_index, ctx);
        match (self.dismiss_handler.as_mut(), event_at_z_index) {
            // If the event is available at the root z-index, that means it isn't covered by the
            // child element, which means the user is clicking outside of the child element
            (Some(handler), Some(Event::LeftMouseDown { .. })) => {
                handler(ctx, app);
                return self.prevent_interaction_with_other_elements;
            }
            (None, Some(Event::LeftMouseDown { .. })) => {
                log::warn!("Dismiss underlay was clicked but no handler was set!");
                return self.prevent_interaction_with_other_elements;
            }
            _ => {}
        };

        if self.prevent_interaction_with_other_elements
            && event_at_z_index.is_some()
            && matches!(
                event.raw_event(),
                Event::ScrollWheel { .. }
                    | Event::LeftMouseUp { .. }
                    | Event::LeftMouseDragged { .. }
                    | Event::MiddleMouseDown { .. }
                    | Event::RightMouseDown { .. }
                    | Event::BackMouseDown { .. }
                    | Event::ForwardMouseDown { .. }
                    | Event::MouseMoved { .. }
            )
        {
            return true;
        }

        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.child.debug_text_content()
    }
}
