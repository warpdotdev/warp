use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_errors::report_error;

use super::{Fill, Point, ZIndex};
use crate::event::DispatchedEvent;
use crate::platform::Cursor;
use crate::{AfterLayoutContext, AppContext, Element, EventContext, PaintContext, SizeConstraint};

const DRAGBAR_WIDTH: f32 = 5.0;

/// A UI element with internal resizing ability.
///
/// This element takes a ResizableStateHandle to receive a starting size and
/// manage dimensions as the element is resized.
///
/// Supports both horizontal and vertical resizing via the ResizeDirection.
///
/// TODO:
///     - Take a configurable dragbar size instead of always using 5.0
pub struct Resizable {
    child: Box<dyn Element>,
    origin: Option<Vector2F>,
    dragbar: Dragbar,
    state_handle: ResizableStateHandle,
    size: Option<Vector2F>,
    bounds_callback: Option<BoundsCallback>,
    resize_handler: Option<Handler>,
    start_resize_handler: Option<Handler>,
    end_resize_handler: Option<Handler>,
    hovering_dragbar: bool,
    direction: ResizeDirection,
    origin_delta: Vector2F,
    dragbar_offset: f32,
}

type Handler = Box<dyn FnMut(&mut EventContext, &AppContext)>;
pub type BoundsCallback = Box<dyn FnMut(Vector2F) -> (f32, f32)>;

/// Similar to MouseStateHandle, the view that incorporates the owner can instantiate,
/// read, and set the state.
pub type ResizableStateHandle = Arc<Mutex<ResizableState>>;
pub fn resizable_state_handle(size: f32) -> ResizableStateHandle {
    Arc::new(Mutex::new(ResizableState::new(size)))
}

pub struct ResizableState {
    size: f32,
    bounds: Option<(f32, f32)>,
    mode: ResizableMode,
}

#[derive(Default)]
pub enum ResizableMode {
    Dragging {
        last_position: Vector2F,
    },
    #[default]
    Stationary,
}

impl ResizableState {
    pub fn new(size: f32) -> Self {
        Self {
            size,
            bounds: None,
            mode: Default::default(),
        }
    }
    pub fn size(&self) -> f32 {
        self.size
    }

    pub fn clamp_size(&mut self) {
        if let Some((min, max)) = self.bounds {
            self.size = self.size.clamp(min, max);
        }
    }

    fn check_for_resize(
        &mut self,
        position: Vector2F,
        origin: Option<Vector2F>,
        dragbar_side: DragBarSide,
    ) -> Option<Vector2F> {
        if let ResizableMode::Dragging { last_position } = self.mode {
            self.resize(last_position, position, origin, dragbar_side)
        } else {
            None
        }
    }

    fn is_resizing(&self) -> bool {
        matches!(self.mode, ResizableMode::Dragging { .. })
    }

    fn resize(
        &mut self,
        old_position: Vector2F,
        new_position: Vector2F,
        origin: Option<Vector2F>,
        dragbar_side: DragBarSide,
    ) -> Option<Vector2F> {
        let mut resized = false;

        if let Some(origin) = origin {
            let delta = match dragbar_side {
                DragBarSide::Right => new_position.x() - old_position.x(),
                DragBarSide::Left => old_position.x() - new_position.x(),
                DragBarSide::Bottom => new_position.y() - old_position.y(),
                DragBarSide::Top => old_position.y() - new_position.y(),
            };

            let old_size = self.size;
            if delta.abs() >= f32::EPSILON {
                resized = true;
                self.size += delta;
                self.clamp_size();
            }
            let size = self.size;

            // The last position should reflect the latest position of the dragbar.
            let last_position = match dragbar_side {
                // With a right-side dragbar, the latest position of the dragbar will
                // be the old origin of the element plus the new width/height.
                DragBarSide::Right => origin + vec2f(size, 0.),
                // With a left-side dragbar, the latest position of the dragbar will
                // be the old origin of the element minus the bounded delta of the drag.
                DragBarSide::Left => origin - vec2f(size - old_size, 0.),
                // With a bottom-side dragbar, the latest position of the dragbar will
                // be the old origin of the element plus the new height.
                DragBarSide::Bottom => origin + vec2f(0., size),
                // With a top-side dragbar, the latest position of the dragbar will
                // be the old origin of the element minus the bounded delta of the drag.
                DragBarSide::Top => origin - vec2f(0., size - old_size),
            };

            let origin_delta = match dragbar_side {
                DragBarSide::Right => Vector2F::zero(),
                DragBarSide::Left => vec2f(old_size - size, 0.),
                DragBarSide::Bottom => Vector2F::zero(),
                DragBarSide::Top => vec2f(0., old_size - size),
            };

            self.mode = ResizableMode::Dragging { last_position };

            if resized {
                Some(origin_delta)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn begin_resizing(&mut self, position: Vector2F) {
        self.mode = ResizableMode::Dragging {
            last_position: position,
        };
    }

    pub fn end_resizing(&mut self) {
        self.mode = ResizableMode::Stationary;
    }

    pub fn set_size(&mut self, new_size: f32) {
        self.size = new_size;
    }
}

struct Dragbar {
    bounds: Option<RectF>,
    origin: Option<Point>,
    size: Option<Vector2F>,
    z_index: Option<ZIndex>,
    color: Fill,
    side: DragBarSide,
}

#[derive(Copy, Clone, Default)]
pub enum DragBarSide {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

#[derive(Copy, Clone, Default)]
pub enum ResizeDirection {
    #[default]
    Horizontal,
    Vertical,
}

impl Dragbar {
    pub fn new() -> Self {
        let color = Fill::Solid(ColorU::transparent_black());
        Self {
            bounds: None,
            origin: None,
            size: None,
            z_index: None,
            color,
            side: Default::default(),
        }
    }
}

impl Resizable {
    pub fn new(state_handle: ResizableStateHandle, child: Box<dyn Element>) -> Self {
        Self {
            child,
            origin: None,
            state_handle,
            size: None,
            bounds_callback: None,
            resize_handler: None,
            start_resize_handler: None,
            end_resize_handler: None,
            dragbar: Dragbar::new(),
            hovering_dragbar: false,
            direction: ResizeDirection::Horizontal,
            origin_delta: Vector2F::zero(),
            dragbar_offset: 0.0,
        }
    }

    /// Adds a callback which will be called on a resize.
    /// Generally, this should trigger a re-render in the parent.
    pub fn on_resize<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut EventContext, &AppContext) + 'static,
    {
        self.resize_handler = Some(Box::new(callback));
        self
    }

    pub fn on_start_resizing<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut EventContext, &AppContext) + 'static,
    {
        self.start_resize_handler = Some(Box::new(callback));
        self
    }

    pub fn on_end_resizing<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut EventContext, &AppContext) + 'static,
    {
        self.end_resize_handler = Some(Box::new(callback));
        self
    }

    /// Sets a function that computes the (min, max) bounds on the width/height
    /// of the resizable. The bounds are updated at paint time.
    pub fn with_bounds_callback(mut self, callback: BoundsCallback) -> Self {
        self.bounds_callback = Some(callback);
        self
    }

    pub fn with_dragbar_color(mut self, color: Fill) -> Self {
        self.dragbar.color = color;
        self
    }

    pub fn with_dragbar_side(mut self, side: DragBarSide) -> Self {
        self.dragbar.side = side;
        // Automatically set direction based on side
        self.direction = match side {
            DragBarSide::Left | DragBarSide::Right => ResizeDirection::Horizontal,
            DragBarSide::Top | DragBarSide::Bottom => ResizeDirection::Vertical,
        };
        self
    }

    /// Sets an offset for the dragbar position.
    /// Positive values move the dragbar outwards (away from the center of the element).
    /// Negative values move the dragbar inwards (towards the center of the element).
    pub fn with_dragbar_offset(mut self, offset: f32) -> Self {
        self.dragbar_offset = offset;
        self
    }

    fn state(&mut self) -> MutexGuard<'_, ResizableState> {
        self.state_handle
            .lock()
            .expect("Resizable state should be accessible")
    }

    /// Determine if the mouse is hovering over the dragbar
    ///
    /// If there is another element above this one at the cursor position, then we treat that as
    /// outside the element for purposes of MouseState
    fn is_mouse_hovering_dragbar(&self, ctx: &EventContext, position: Vector2F) -> bool {
        let Some(dragbar_origin) = self.dragbar.origin else {
            log::warn!("self.origin was None in `Hoverable::is_mouse_in`");
            return false;
        };
        let Some(dragbar_size) = self.dragbar.size else {
            log::warn!("self.size() was None in `Hoverable::is_mouse_in`");
            return false;
        };
        let Some(z_index) = self.dragbar.z_index else {
            log::warn!("self.child_max_z_index was None in `Hoverable::is_mouse_in`");
            return false;
        };

        let is_hovering = ctx
            .visible_rect(dragbar_origin, dragbar_size)
            .is_some_and(|bound| bound.contains_point(position));

        let point = Point::from_vec2f(position, z_index);
        let is_covered = ctx.is_covered(point);

        is_hovering && !is_covered
    }
}

impl Element for Resizable {
    fn layout(
        &mut self,
        constraint: crate::SizeConstraint,
        ctx: &mut crate::LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // Use the window size to set bounds on the width/height
        if let Some(bounds_callback) = self.bounds_callback.as_mut() {
            let mut new_bounds = bounds_callback(ctx.window_size);
            if new_bounds.0 > new_bounds.1 {
                report_error!(
                    "Resizable: min bound is greater than max bound",
                    extra: {
                        "min" => ?new_bounds.0,
                        "max" => ?new_bounds.1
                    }
                );
                new_bounds = (new_bounds.0, new_bounds.0);
            }
            self.state().bounds = Some(new_bounds);

            // With new bounds, we should also clamp the current width/height.
            self.state().clamp_size();
        }

        let size = self.state().size;

        // We set the child constraints to never be greater than the current width/height constraint.
        let child_constraint = match self.direction {
            ResizeDirection::Horizontal => SizeConstraint {
                min: (constraint.min)
                    .max(Vector2F::zero())
                    .min(Vector2F::new(size, f32::MAX)),
                max: (constraint.max)
                    .max(Vector2F::zero())
                    .min(Vector2F::new(size, f32::MAX)),
            },
            ResizeDirection::Vertical => SizeConstraint {
                min: (constraint.min)
                    .max(Vector2F::zero())
                    .min(Vector2F::new(f32::MAX, size)),
                max: (constraint.max)
                    .max(Vector2F::zero())
                    .min(Vector2F::new(f32::MAX, size)),
            },
        };
        let child_size = self.child.layout(child_constraint, ctx, app);

        let size = child_size;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);

        // Draw the dragbar and record its size and position
        let child_size = self.child.size().unwrap();
        let (dragbar_origin, dragbar_size) = match self.dragbar.side {
            DragBarSide::Left => (
                origin - vec2f(self.dragbar_offset, 0.),
                vec2f(DRAGBAR_WIDTH, child_size.y()),
            ),
            DragBarSide::Right => (
                origin + vec2f(child_size.x() - DRAGBAR_WIDTH + self.dragbar_offset, 0.),
                vec2f(DRAGBAR_WIDTH, child_size.y()),
            ),
            DragBarSide::Top => (
                origin - vec2f(0., self.dragbar_offset),
                vec2f(child_size.x(), DRAGBAR_WIDTH),
            ),
            DragBarSide::Bottom => (
                origin + vec2f(0., child_size.y() - DRAGBAR_WIDTH + self.dragbar_offset),
                vec2f(child_size.x(), DRAGBAR_WIDTH),
            ),
        };

        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(dragbar_origin, dragbar_size))
            .with_background(self.dragbar.color);

        self.dragbar.bounds = Some(RectF::new(dragbar_origin, dragbar_size));
        self.dragbar.origin = Some(Point::from_vec2f(dragbar_origin, ctx.scene.z_index()));
        self.dragbar.size = Some(dragbar_size);
        self.dragbar.z_index = Some(ctx.scene.max_active_z_index());

        self.origin = Some(origin);
        self.origin_delta = Vector2F::zero();
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let child_handled = self.child.dispatch_event(event, ctx, app);

        match event.raw_event() {
            crate::Event::LeftMouseDown { position, .. } => {
                // If a mouse-down on the dragbar element occurred, put the view into resizing mode
                if self
                    .dragbar
                    .bounds
                    .is_some_and(|bounds| bounds.contains_point(*position))
                {
                    self.state().begin_resizing(*position);
                    dispatch_callback(self.resize_handler.as_mut(), ctx, app);
                    return true;
                }
            }

            crate::Event::LeftMouseUp { .. } => {
                // If a mouse-up occurs, take the view out of resizing mode
                if self.state().is_resizing() {
                    ctx.reset_cursor();
                    self.state().end_resizing();
                    dispatch_callback(self.end_resize_handler.as_mut(), ctx, app);
                    return true;
                }
            }

            crate::Event::LeftMouseDragged { position, .. } => {
                if self.state().is_resizing() {
                    let dragbar_side = self.dragbar.side;
                    let origin = self.origin.map(|origin| origin + self.origin_delta);
                    let resized = self
                        .state()
                        .check_for_resize(*position, origin, dragbar_side);
                    self.origin_delta += resized.unwrap_or_default();
                    if resized.is_some() {
                        dispatch_callback(self.resize_handler.as_mut(), ctx, app)
                    }
                    return true;
                }
            }
            crate::Event::MouseMoved { position, .. } => {
                // A mouse event over the dragbar should set the cursor
                let Some(z_index) = self.z_index() else {
                    log::warn!("self.z_index() was None in `Resizable`");
                    return false;
                };
                let hovering_dragbar = self.is_mouse_hovering_dragbar(ctx, *position);
                let was_already_hovering =
                    mem::replace(&mut self.hovering_dragbar, hovering_dragbar);

                if hovering_dragbar && !was_already_hovering {
                    let cursor = match self.direction {
                        ResizeDirection::Horizontal => Cursor::ResizeLeftRight,
                        ResizeDirection::Vertical => Cursor::ResizeUpDown,
                    };
                    ctx.set_cursor(cursor, z_index);
                } else if !hovering_dragbar && was_already_hovering {
                    ctx.reset_cursor();
                }

                return true;
            }
            _ => {}
        }
        child_handled
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}

fn dispatch_callback(callback: Option<&mut Handler>, ctx: &mut EventContext, app: &AppContext) {
    if let Some(callback) = callback {
        callback(ctx, app);
    }
}
