use std::any::Any;

use async_channel::Sender;
use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::{
    AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

use super::view::TerminalAction;

pub struct TerminalSizeElement {
    child: Box<dyn Element>,
    resize_tx: Sender<Vector2F>,
}

impl TerminalSizeElement {
    pub fn new(resize_tx: Sender<Vector2F>, child: Box<dyn Element>) -> Self {
        TerminalSizeElement { child, resize_tx }
    }
}

impl Element for TerminalSizeElement {
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

        let terminal_size = self.size().expect("Size should be present");
        // It's possible that the underlying shell session has been terminated
        // but we're showing a read-only terminal view, in which case, the
        // channel will be closed.  If we're unable to send a resize through
        // the channel, that's fine, just ignore the error.
        let _ = self.resize_tx.try_send(terminal_size);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn bounds(&self) -> Option<RectF> {
        self.child.bounds()
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        self.child.parent_data()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled_by_child = self.child.dispatch_event(event, ctx, app);
        let Some(z_index) = self.z_index() else {
            return false;
        };

        if !handled_by_child && let Some(event_at_z_index) = event.at_z_index(z_index, ctx) {
            match event_at_z_index {
                Event::DragFiles { location } => {
                    if self.mouse_position_is_in_bounds(*location) {
                        ctx.dispatch_typed_action(TerminalAction::StartFileDropTarget);
                    } else {
                        ctx.dispatch_typed_action(TerminalAction::StopFileDropTarget);
                    }
                    return true;
                }
                Event::DragFileExit => {
                    ctx.dispatch_typed_action(TerminalAction::StopFileDropTarget);
                    return true;
                }
                Event::DragAndDropFiles { paths, location } => {
                    if self.mouse_position_is_in_bounds(*location) && !paths.is_empty() {
                        let paths = paths.iter().map(ToOwned::to_owned).collect();
                        ctx.dispatch_typed_action(TerminalAction::DragAndDropFiles(paths));
                    }
                    return true;
                }
                _ => {}
            };
        }
        handled_by_child
    }
}

impl TerminalSizeElement {
    fn mouse_position_is_in_bounds(&self, position: Vector2F) -> bool {
        let Some(bounds) = self.bounds() else {
            return false;
        };

        bounds.contains_point(position)
    }
}
