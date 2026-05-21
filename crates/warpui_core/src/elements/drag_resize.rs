use std::sync::{Arc, Mutex};

use pathfinder_geometry::vector::Vector2F;

use super::{Point, ZIndex};
use crate::event::DispatchedEvent;
use crate::platform::Cursor;
use crate::{
    AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// Shared handle for drag-to-resize state, following the same `Arc<Mutex<_>>`
/// pattern as `ResizableStateHandle`. The view creates the handle once at
/// construction and passes it into `DragResizeElement` each render.
pub type DragResizeHandle = Arc<Mutex<DragResizeState>>;

pub fn drag_resize_handle() -> DragResizeHandle {
    Arc::new(Mutex::new(DragResizeState::default()))
}

/// Tracks whether a drag-to-resize operation is in progress.
#[derive(Default)]
pub struct DragResizeState {
    is_dragging: bool,
    last_y: f32,
}

impl DragResizeState {
    fn begin(&mut self, y: f32) {
        self.is_dragging = true;
        self.last_y = y;
    }

    fn end(&mut self) {
        self.is_dragging = false;
    }

    fn is_dragging(&self) -> bool {
        self.is_dragging
    }

    /// Compute the vertical delta since the last event and update `last_y`.
    fn consume_delta(&mut self, y: f32) -> f32 {
        let delta = y - self.last_y;
        self.last_y = y;
        delta
    }
}

/// Callback invoked during a resize drag. Receives the vertical delta (pixels).
type ResizeUpdateFn = Box<dyn Fn(f32, &mut EventContext, &AppContext)>;

/// Callback invoked when a resize drag finishes.
pub type ResizeEndFn = Box<dyn Fn(&mut EventContext, &AppContext)>;

/// An element that enables drag-to-resize on its entire surface area.
///
/// The element dispatches events to its child first. If the child does not
/// handle a `LeftMouseDown`, the element begins a resize operation. Subsequent
/// `LeftMouseDragged` / `LeftMouseUp` events are captured via `raw_event()` so
/// they work regardless of cursor position (same pattern used by `Resizable`).
pub struct DragResizeElement {
    child: Box<dyn Element>,
    handle: DragResizeHandle,
    on_resize_update: ResizeUpdateFn,
    on_resize_end: Option<ResizeEndFn>,
    origin: Option<Point>,
    child_max_z_index: Option<ZIndex>,
}

impl DragResizeElement {
    pub fn new(
        handle: DragResizeHandle,
        child: Box<dyn Element>,
        on_resize_update: impl Fn(f32, &mut EventContext, &AppContext) + 'static,
        on_resize_end: Option<ResizeEndFn>,
    ) -> Self {
        Self {
            child,
            handle,
            on_resize_update: Box::new(on_resize_update),
            on_resize_end,
            origin: None,
            child_max_z_index: None,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, DragResizeState> {
        // This is the same (slightly scary) pattern as `Resizable::state()`.
        // Poisoning should only occur after a prior panic (already in a bad state).
        self.handle.lock().expect("DragResizeState lock poisoned")
    }
}

impl Element for DragResizeElement {
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
        self.child.paint(origin, ctx, app);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // Always let the child see the event first.
        let child_handled = self.child.dispatch_event(event, ctx, app);

        // Use raw_event() for drag/up so they are position-independent.
        match event.raw_event() {
            Event::LeftMouseDown { position, .. } => {
                if child_handled {
                    return true;
                }
                // Check if the click is within our bounds.
                if let (Some(origin), Some(size)) = (self.origin, self.size()) {
                    if let Some(rect) = ctx.visible_rect(origin, size) {
                        if rect.contains_point(*position) {
                            self.state().begin(position.y());
                            return true;
                        }
                    }
                }
                false
            }
            Event::LeftMouseDragged { position, .. } => {
                if self.state().is_dragging() {
                    if let Some(z_index) = self.child_max_z_index {
                        ctx.set_cursor(Cursor::ResizeUpDown, z_index);
                    }
                    let delta = self.state().consume_delta(position.y());
                    (self.on_resize_update)(delta, ctx, app);
                    return true;
                }
                child_handled
            }
            Event::LeftMouseUp { .. } => {
                if self.state().is_dragging() {
                    self.state().end();
                    if let Some(on_end) = &self.on_resize_end {
                        (on_end)(ctx, app);
                    }
                    ctx.reset_cursor();
                    return true;
                }
                child_handled
            }
            Event::MouseMoved { .. } => {
                if self.state().is_dragging() {
                    if let Some(z_index) = self.child_max_z_index {
                        ctx.set_cursor(Cursor::ResizeUpDown, z_index);
                    }
                    return true;
                }
                child_handled
            }
            _ => child_handled,
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}
