use std::cmp::Ordering;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

use itertools::Itertools;
use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};

use crate::elements::{DropTargetData, Point};
use crate::event::{DispatchedEvent, Event};
use crate::platform::Cursor;
use crate::presenter::PositionCache;
use crate::scene::{ClipBounds, ZIndex};
use crate::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// The default drag threshold used when no value is explicitly set by the creator
const DEFAULT_DRAG_THRESHOLD: f32 = 5.;

/// Opaque state container for maintaining drag across re-renders
///
/// Cheap to clone so that the owning View can easily create new Elements with the state
#[derive(Clone, Default)]
pub struct DraggableState {
    inner: Arc<Mutex<DragState>>,
    suppress_overlay_paint: Arc<AtomicBool>,
}

impl DraggableState {
    /// Determine if the current state represents an actual drag or not
    pub fn is_dragging(&self) -> bool {
        matches!(*self.inner.lock(), DragState::Dragging { .. })
    }

    /// When true, the drag overlay visual will not be painted.
    /// Used during preview capture to exclude the drag ghost from the captured frame.
    pub fn suppress_overlay_paint(&self) -> bool {
        self.suppress_overlay_paint.load(AtomicOrdering::Relaxed)
    }

    /// Set whether to suppress painting the drag overlay visual.
    pub fn set_suppress_overlay_paint(&self, suppress: bool) {
        self.suppress_overlay_paint
            .store(suppress, AtomicOrdering::Relaxed);
    }

    /// Copy the actual drag state value out of the container
    fn read(&self) -> DragState {
        *self.inner.lock()
    }

    /// Returns the cursor offset within the draggable element, if drag state is available.
    pub fn cursor_offset_within_element(&self) -> Option<Vector2F> {
        match self.read() {
            DragState::None => None,
            DragState::WaitingToDrag {
                mouse_down_offset, ..
            } => Some(-mouse_down_offset),
            DragState::Dragging { mouse_offset, .. } => Some(-mouse_offset),
        }
    }

    /// Returns the actual mouse position while dragging an element.
    pub fn dragging_mouse_position(&self) -> Option<Vector2F> {
        match self.read() {
            DragState::Dragging { mouse_position, .. } => Some(mouse_position),
            DragState::None | DragState::WaitingToDrag { .. } => None,
        }
    }

    pub fn adjust_mouse_position(&self, delta: Vector2F) {
        let mut guard = self.inner.lock();
        if let DragState::Dragging { mouse_position, .. } = &mut *guard {
            *mouse_position += delta;
        }
    }

    pub fn set_dragging(&self, new_mouse_position: Vector2F, new_mouse_offset: Vector2F) {
        self.store(DragState::Dragging {
            mouse_position: new_mouse_position,
            mouse_offset: new_mouse_offset,
            is_on_accepted_drop_target: false,
        });
    }

    pub fn cancel_drag(&self) {
        self.store(DragState::None);
    }

    /// Update the drag state with a new value
    fn store(&self, new_state: DragState) {
        *self.inner.lock() = new_state;
    }
}

/// Internal state tracking whether or not we are dragging the element and the parameters of the
/// drag
#[derive(Clone, Copy, Default)]
enum DragState {
    /// No dragging is happening
    #[default]
    None,
    /// The mouse is held down, but has not yet moved beyond the drag threshold
    WaitingToDrag {
        /// The position where the mouse down occurred, used to check against the threshold
        mouse_down_position: Vector2F,
        /// The offset from the mouse down position to the natural origin of the element
        mouse_down_offset: Vector2F,
    },
    Dragging {
        /// The most recently reported position of the mouse
        mouse_position: Vector2F,
        /// The offset from the mouse to the origin of the Element
        ///
        /// This is determined by the mouse position when dragging starts and is used during
        /// dragging to calculate the dragged origin via `origin = mouse_position + mouse_offset`
        mouse_offset: Vector2F,
        /// Whether the dragged element is currently on an accepted drop target.
        is_on_accepted_drop_target: bool,
    },
}

/// The axis to which a draggable is fixed, limiting it to only move in one direction
#[derive(Clone, Copy)]
pub enum DragAxis {
    HorizontalOnly,
    VerticalOnly,
}

type BoundsCallback = Box<dyn FnMut(&PositionCache, Vector2F) -> Option<RectF>>;

/// The bounds for dragging this element.
///
/// This can be set to a fixed rectangle in the scene or to a callback that is used to calculate
/// the bounds.
enum DragBounds {
    None,
    Fixed(RectF),
    Callback(BoundsCallback),
}

impl DragBounds {
    fn calculate(
        &mut self,
        position_cache: &PositionCache,
        window_size: Vector2F,
    ) -> Option<RectF> {
        match self {
            DragBounds::None => None,
            DragBounds::Fixed(bounds) => Some(*bounds),
            DragBounds::Callback(callback) => callback(position_cache, window_size),
        }
    }
}

type Handler = Box<dyn FnMut(&mut EventContext, &AppContext, RectF)>;

/// Handler when a `Draggable` is dragged and dropped. Includes the data of a [`crate::elements::DropTarget`] if
/// the `Draggable` was dropped on a `DropTarget`.
type DragDropHandler =
    Box<dyn FnMut(&mut EventContext, &AppContext, RectF, Option<&dyn DropTargetData>)>;

pub enum AcceptedByDropTarget {
    Yes,
    No,
}

/// Callback that determines whether this [`Draggable`] can be dropped on a `DropTarget` that
/// contains  [`DropTargetData`].
type AcceptedByDropTargetHandler =
    Box<dyn Fn(&dyn DropTargetData, &AppContext) -> AcceptedByDropTarget>;

/// A container element that can be freely dragged and dropped around the screen
///
/// Dragging starts when the mouse is held down and dragged at least a threshold distance (default
/// 5 pixels) away from where it started. Dragging stops when the mouse is released.
///
/// ## Layout and Painting
///
/// While dragging, the Element is still laid out using its original position in the Element tree,
/// however it is painted at the location of the mouse. Once dragging stops, it returns to being
/// painted in the normal tree element position (i.e. the dragged position is not maintained after
/// dragging stops).
///
/// ## Limiting the Draggable Area
///
/// There are two complementary ways to limit the space that the Element can be dragged:
///
/// 1. Specifying a drag axis so that the Element can only be dragged in one direction.
/// 2. Specifying bounds that limit the Element to only dragging within a specific rectangle.
///
/// ### Fixed Axis
///
/// To limit the Draggable to only move in a single direction, call `with_drag_axis` and pass it
/// the appropriate direction (either `DragAxis::HorizontalOnly` or `DragAxis::VerticalOnly`). The
/// Element will be able to freely move in the given direction, but will not move at all in the
/// perpendicular direction.
///
/// ### Bounding Box
///
/// To specify a bounding box in which the Element is confined, call either:
///
/// * `with_drag_bounds` - Takes a fixed `RectF` value and uses that for the bounds.
/// * `with_drag_bounds_callback` - Takes a callback—which accepts a `&StackContext` and the window
///   size—that returns an `Option<RectF>` to indicate the bounds (or lack thereof).
///
/// In either case, if a bounding rectangle exists, the Element will be confined to only drag
/// completely within that rectangle. If it happens that the bounding box is smaller than the
/// Element, the top-left corner will be fixed within the bounding box and any overflow will happen
/// to the right or below the bounds.
///
/// If you specify a callback for calculating the bounds, it will be called each time the Element
/// is painted and the value will be cached for subsequent events.
///
/// ## Callbacks
///
/// There are three event callbacks that can be used to react to dragging:
///
/// - `on_drag_start`: Called when the `drag_threshold` is crossed and dragging begins.
/// - `on_drag`: Called on mouse move while dragging.
/// - `on_drop`: Called on mouse up when dragging stops. If the `Draggable` was dropped on
///   a [`crate::elements::DropTarget`] the data of that `DropTarget` is passed as a parameter.
///
/// All of the callbacks receive three parameters:
///
/// - An `&mut EventContext`
/// - An `&AppContext`
/// - A `RectF` representing the current painted position and size of the Element.
///
/// Note: For `on_drag_start`, the `RectF` passed will be the original position of the Element when
/// the mouse was first pressed, not the shifted position after crossing the threshold.
///
/// ## Child Events
///
/// While this element is actively being dragged, all events to the child Element will be
/// suppressed, the drag behavior will take precedence over any other events.
///
///
/// ## Drop Targets
///
/// `Draggable`s can optionally be dropped on [`crate::elements::DropTarget`]s. When dropped, the
/// [`:DropTargetData`] of the `DropTarget` is included as a parameter to identify the `DropTarget`
/// the element was dropped on.
pub struct Draggable {
    state: DraggableState,
    child: Box<dyn Element>,
    alternate_drag_element: Option<Box<dyn Element>>,
    child_max_z_index: Option<ZIndex>,
    unmodified_origin: Option<Vector2F>,
    drag_threshold: f32,
    drag_axis: Option<DragAxis>,
    drag_bounds: DragBounds,
    // Cache of the bounds value, used to avoid redundant calls to `DragBounds::Callback`. This is
    // updated on every call to `paint` so that even if the Element is laid out again, the value is
    // accurate for subsequent events.
    bounds_cache: Option<RectF>,
    /// If true, keeps the original element visible in its original position during drag.
    keep_original_visible: bool,
    /// Defers to a nested `Draggable` that already claimed the mouse-down.
    defer_to_handled_child_mouse_down: bool,

    start_handler: Option<Handler>,
    drag_handler: Option<DragDropHandler>,
    is_accepted_by_drop_target_handler: Option<AcceptedByDropTargetHandler>,
    drop_handler: Option<DragDropHandler>,
    /// Whether to use the copy cursor while dragging on a valid drop target.
    use_copy_cursor_when_dragging_over_drop_target: bool,
}

impl Draggable {
    pub fn new(state: DraggableState, child: Box<dyn Element>) -> Self {
        Self {
            state,
            child,
            alternate_drag_element: None,
            child_max_z_index: None,
            unmodified_origin: None,
            drag_threshold: DEFAULT_DRAG_THRESHOLD,
            drag_axis: None,
            drag_bounds: DragBounds::None,
            bounds_cache: None,
            keep_original_visible: false,
            defer_to_handled_child_mouse_down: false,
            start_handler: None,
            drag_handler: None,
            is_accepted_by_drop_target_handler: None,
            drop_handler: None,
            use_copy_cursor_when_dragging_over_drop_target: false,
        }
    }

    /// Set a custom drag threshold, in pixels.
    pub fn with_drag_threshold(mut self, threshold: f32) -> Self {
        self.drag_threshold = threshold;
        self
    }

    /// Set a custom drag axis.
    pub fn with_drag_axis(mut self, axis: DragAxis) -> Self {
        self.drag_axis = Some(axis);
        self
    }

    /// Sets an alternate element to be rendered while the drag is active
    pub fn with_alternate_drag_element(mut self, element: Box<dyn Element>) -> Self {
        self.alternate_drag_element = Some(element);
        self
    }

    /// When true, keeps the original element visible in its original position during drag,
    /// showing both the original and the dragged copy.
    pub fn with_keep_original_visible(mut self, keep_visible: bool) -> Self {
        self.keep_original_visible = keep_visible;
        self
    }

    /// Set custom bounds to limit where the element can be dragged.
    ///
    /// Note: If the bounds are smaller than the element along either axis, the top-left corner
    /// will be clamped to the minimum value of the bounds along that axis.
    pub fn with_drag_bounds(mut self, bounds: RectF) -> Self {
        self.drag_bounds = DragBounds::Fixed(bounds);
        self
    }

    /// Skips initiating this drag when a nested `Draggable` already claimed the mouse-down.
    pub fn with_defer_to_handled_child_mouse_down(mut self) -> Self {
        self.defer_to_handled_child_mouse_down = true;
        self
    }

    /// Whether to use the copy cursor when dragging over a drop target.
    pub fn use_copy_cursor_when_dragging_over_drop_target(mut self) -> Self {
        self.use_copy_cursor_when_dragging_over_drop_target = true;
        self
    }
    /// Set a custom bounds callback used to calculate the limits of dragging.
    ///
    /// The value will be calculated whenever the element is painted and cached for use in
    /// subsequent events.
    ///
    /// Note: If the bounds are smaller than the element along either axis, the top-left corner
    /// will be clamped to the minimum value of the bounds along that axis.
    pub fn with_drag_bounds_callback<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&PositionCache, Vector2F) -> Option<RectF> + 'static,
    {
        self.drag_bounds = DragBounds::Callback(Box::new(callback));
        self
    }

    /// Add a callback which will be called on mouse down when dragging starts.
    pub fn on_drag_start<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut EventContext, &AppContext, RectF) + 'static,
    {
        self.start_handler = Some(Box::new(callback));
        self
    }

    /// Add a callback which will be called on mouse move while dragging.
    pub fn on_drag<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut EventContext, &AppContext, RectF, Option<&dyn DropTargetData>) + 'static,
    {
        self.drag_handler = Some(Box::new(callback));
        self
    }

    /// Add a callback which will be called on mouse up when dragging ends.
    pub fn on_drop<F>(mut self, callback: F) -> Self
    where
        F: FnMut(&mut EventContext, &AppContext, RectF, Option<&dyn DropTargetData>) + 'static,
    {
        self.drop_handler = Some(Box::new(callback));
        self
    }

    /// Add a callback to determine if a given DropTarget will accept this draggable
    /// for drop or drag callbacks
    pub fn with_accepted_by_drop_target_fn<F>(mut self, callback: F) -> Self
    where
        F: Fn(&dyn DropTargetData, &AppContext) -> AcceptedByDropTarget + 'static,
    {
        self.is_accepted_by_drop_target_handler = Some(Box::new(callback));
        self
    }

    /// Add a callback which will be called on mouse down when dragging starts.
    pub fn set_on_drag_start<F>(&mut self, callback: F)
    where
        F: FnMut(&mut EventContext, &AppContext, RectF) + 'static,
    {
        self.start_handler = Some(Box::new(callback));
    }

    /// Add a callback which will be called on mouse move while dragging.
    pub fn set_on_drag<F>(&mut self, callback: F)
    where
        F: FnMut(&mut EventContext, &AppContext, RectF, Option<&dyn DropTargetData>) + 'static,
    {
        self.drag_handler = Some(Box::new(callback));
    }

    /// Add a callback which will be called on mouse up when dragging ends.
    pub fn set_on_drop<F>(&mut self, callback: F)
    where
        F: FnMut(&mut EventContext, &AppContext, RectF, Option<&dyn DropTargetData>) + 'static,
    {
        self.drop_handler = Some(Box::new(callback));
    }

    /// Determine the drag origin based on the specified axis and any cached bounds.
    fn drag_origin(&self, mouse_position: Vector2F, mouse_offset: Vector2F) -> Vector2F {
        let unclamped_origin = match self.drag_axis {
            // By default, we allow full drag in both directions, so we can use Vector addition
            // to determine the appropriate origin.
            None => mouse_position + mouse_offset,
            Some(DragAxis::HorizontalOnly) => {
                // For horizontal-only drag, we use the x value from the mouse position and keep
                // the default y value from the laid-out element
                let x = mouse_position.x() + mouse_offset.x();
                let y = self.unmodified_origin.expect("origin should exist").y();
                Vector2F::new(x, y)
            }
            Some(DragAxis::VerticalOnly) => {
                // Similarly, for vertical-only drag, we use the x value from the laid-out element
                // and the y value from the mouse
                let x = self.unmodified_origin.expect("origin should exist").x();
                let y = mouse_position.y() + mouse_offset.y();
                Vector2F::new(x, y)
            }
        };

        match self.bounds_cache {
            Some(bounds) => {
                let size = self.size().expect("size should be set");

                let min_x = bounds.min_x();
                let max_x = (bounds.max_x() - size.x()).max(min_x);
                let x = unclamped_origin.x().clamp(min_x, max_x);

                let min_y = bounds.min_y();
                let max_y = (bounds.max_y() - size.y()).max(min_y);
                let y = unclamped_origin.y().clamp(min_y, max_y);

                Vector2F::new(x, y)
            }
            None => unclamped_origin,
        }
    }

    /// Returns the [`DropTargetData`] for a [`crate::elements::DropTarget`] that overlaps with
    /// `rect`. Returns `None` if there is a no `DropTarget` at the location.
    ///
    /// If multiple `DropTarget`s are matched, the one with the smallest size (by area) is
    /// returned. If the areas are the same, then we return the drop target with the closest center
    /// to the drag position center.
    fn compute_drop_target_data(
        rect: RectF,
        accepted_function: &AcceptedByDropTargetHandler,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> Option<Arc<dyn DropTargetData>> {
        ctx.drop_target_data()
            .filter(|drop_target_position| {
                drop_target_position.bounds().intersection(rect).is_some()
                    && match accepted_function(drop_target_position.data().as_ref(), app) {
                        AcceptedByDropTarget::Yes => true,
                        AcceptedByDropTarget::No => false,
                    }
            })
            .sorted_by(|position_a, position_b| {
                if position_a.area().round() != position_b.area().round() {
                    position_a.area().cmp(&position_b.area())
                } else {
                    let drag_center = rect.center();
                    let position_a_center_distance =
                        (drag_center - position_a.bounds().center()).length();
                    let position_b_center_distance =
                        (drag_center - position_b.bounds().center()).length();
                    if position_a_center_distance < position_b_center_distance {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
            })
            .map(|drop_target_position| drop_target_position.data().clone())
            .next()
    }

    /// Computes the mouse offset based on whether or not there's a specified alternate child. If there's
    /// an alternate child, the calculated offset will be based on the ratio of the sizes to the base child
    /// versus the alternate child.
    ///
    /// From the user's perspective, this ensures that the mouse position is in the same relative position
    /// in the alternate element as where they started the drag.
    fn compute_mouse_offset(&self, base_mouse_offset: Vector2F, child_size: Vector2F) -> Vector2F {
        if let Some(alternate_child) = &self.alternate_drag_element {
            let alternate_child_size = alternate_child.size().expect("size should exist");

            let size_difference_ratio = Vector2F::new(
                alternate_child_size.x() / child_size.x(),
                alternate_child_size.y() / child_size.y(),
            );
            Vector2F::new(
                base_mouse_offset.x() * size_difference_ratio.x(),
                base_mouse_offset.y() * size_difference_ratio.y(),
            )
        } else {
            base_mouse_offset
        }
    }
}

impl Element for Draggable {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        if let Some(alternate_child) = &mut self.alternate_drag_element {
            // For the alternate drag element, we ignore parent size contraints
            alternate_child.layout(
                SizeConstraint::new(vec2f(0.0, 0.0), vec2f(f32::INFINITY, f32::INFINITY)),
                ctx,
                app,
            );
        }
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        if let Some(alternate_child) = &mut self.alternate_drag_element {
            alternate_child.after_layout(ctx, app);
        }
        self.child.after_layout(ctx, app)
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        // We always cache the laid-out origin for the element, even if we are drawing it elsewhere
        // for a drag. This allows us to look up the unmodified origin when calculating position
        // for fixed-axis draggables
        self.unmodified_origin = Some(origin);

        // Update the bounds cache based on the provided drag bounds, if necessary
        self.bounds_cache = self
            .drag_bounds
            .calculate(ctx.position_cache, ctx.window_size);

        match self.state.read() {
            DragState::None | DragState::WaitingToDrag { .. } => {
                // If we aren't dragging or we haven't yet passed the drag threshold, we paint the
                // element in its normal location.
                self.child.paint(origin, ctx, app);
            }
            DragState::Dragging {
                mouse_position,
                mouse_offset,
                ..
            } => {
                if self.keep_original_visible || self.state.suppress_overlay_paint() {
                    self.child.paint(origin, ctx, app);
                }
                // Paint the dragged element on an overlay layer so it appears
                // above anything we drag over.
                if !self.state.suppress_overlay_paint() {
                    ctx.scene.start_overlay_layer(ClipBounds::None);
                    let drag_origin = self.drag_origin(mouse_position, mouse_offset);
                    if let Some(alternate_child) = &mut self.alternate_drag_element {
                        alternate_child.paint(drag_origin, ctx, app);
                    } else {
                        self.child.paint(drag_origin, ctx, app);
                    }
                    ctx.scene.stop_layer();
                }
            }
        }
        // After drawing the child (and stopping the overlay layer, if appropriate), the max
        // z-index in the scene will represent the highest point drawn by the child.
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let size = self.size().expect("size should exist");
        let current_state = self.state.read();

        let handled = match current_state {
            DragState::None | DragState::WaitingToDrag { .. } => {
                // If we have not yet started dragging, then we always pass events to the child
                self.child.dispatch_event(event, ctx, app)
            }
            DragState::Dragging { .. } => {
                // If we are dragging, then we suppress all child events for the duration
                false
            }
        };

        match event.raw_event() {
            Event::LeftMouseDown { position, .. } => {
                if self.defer_to_handled_child_mouse_down && ctx.descendant_draggable_initiated() {
                    return true;
                }
                let origin = self.origin().expect("origin should exist");
                if let Some(rect) = ctx.visible_rect(origin, size) {
                    let max_z_index = self.child_max_z_index.expect("child z index should exist");
                    // Only start dragging if the mouse is within the element and not covered by
                    // an element on a higher layer
                    if rect.contains_point(*position)
                        && !ctx.is_covered(Point::from_vec2f(*position, max_z_index))
                    {
                        let base_mouse_offset = origin.xy() - *position;
                        let mouse_down_offset = self.compute_mouse_offset(base_mouse_offset, size);
                        self.state.store(DragState::WaitingToDrag {
                            mouse_down_position: *position,
                            mouse_down_offset,
                        });
                        ctx.mark_descendant_draggable_initiated();

                        ctx.set_cursor(Cursor::PointingHand, max_z_index);
                        return true;
                    }
                }
                handled
            }
            Event::LeftMouseUp { .. } => match current_state {
                DragState::None => handled,
                DragState::WaitingToDrag { .. } => {
                    self.state.store(DragState::None);
                    ctx.reset_cursor();
                    true
                }
                DragState::Dragging {
                    mouse_offset,
                    mouse_position,
                    ..
                } => {
                    let origin = self.drag_origin(mouse_position, mouse_offset);
                    let rect = RectF::new(origin, size);

                    self.state.store(DragState::None);

                    let draggable_data = if let Some(accepted_fn) =
                        self.is_accepted_by_drop_target_handler.as_ref()
                    {
                        Self::compute_drop_target_data(rect, accepted_fn, ctx, app)
                    } else {
                        None
                    };

                    if let Some(callback) = self.drop_handler.as_mut() {
                        callback(ctx, app, rect, draggable_data.as_deref());
                    }

                    ctx.reset_cursor();
                    ctx.notify();
                    true
                }
            },
            Event::LeftMouseDragged { position, .. } => match current_state {
                DragState::None => handled,
                DragState::WaitingToDrag {
                    mouse_down_position,
                    mouse_down_offset,
                } => {
                    let drag_start_distance = (mouse_down_position - *position).length();
                    if drag_start_distance > self.drag_threshold {
                        // If the drag has moved beyond the `drag_threshold`, then we officially
                        // start the drag and fire the `on_drag_start` callback.
                        self.state.store(DragState::Dragging {
                            mouse_offset: mouse_down_offset,
                            mouse_position: *position,
                            is_on_accepted_drop_target: false,
                        });

                        // Note: For the `on_drag_start` callback, we pass the position that the
                        // mouse down happened, since that is where the element was at the start
                        // of the drag.
                        let origin = self.drag_origin(mouse_down_position, mouse_down_offset);
                        let rect = RectF::new(origin, size);
                        dispatch_callback(self.start_handler.as_mut(), ctx, app, rect);

                        ctx.notify();
                        true
                    } else {
                        handled
                    }
                }
                DragState::Dragging {
                    mouse_offset,
                    is_on_accepted_drop_target: was_on_accepted_drop_target,
                    ..
                } => {
                    let origin = self.drag_origin(*position, mouse_offset);
                    let rect = RectF::new(origin, size);

                    let draggable_data = if let Some(accepted_fn) =
                        self.is_accepted_by_drop_target_handler.as_ref()
                    {
                        Self::compute_drop_target_data(rect, accepted_fn, ctx, app)
                    } else {
                        None
                    };

                    let is_on_accepted_drop_target = draggable_data.is_some();

                    if self.use_copy_cursor_when_dragging_over_drop_target {
                        let max_z_index =
                            self.child_max_z_index.expect("child z index should exist");
                        match (was_on_accepted_drop_target, is_on_accepted_drop_target) {
                            (true, false) => {
                                ctx.set_cursor(Cursor::PointingHand, max_z_index);
                            }
                            (false, true) => {
                                ctx.set_cursor(Cursor::DragCopy, max_z_index);
                            }
                            _ => {}
                        }
                    }

                    self.state.store(DragState::Dragging {
                        mouse_offset,
                        mouse_position: *position,
                        is_on_accepted_drop_target,
                    });
                    dispatch_drag_drop_callback(
                        self.drag_handler.as_mut(),
                        ctx,
                        app,
                        rect,
                        draggable_data.as_deref(),
                    );

                    ctx.notify();
                    true
                }
            },
            _ => handled,
        }
    }

    fn size(&self) -> Option<Vector2F> {
        match self.state.read() {
            DragState::None | DragState::WaitingToDrag { .. } => self.child.size(),
            DragState::Dragging { .. } => {
                if let Some(alternate_child) = &self.alternate_drag_element {
                    alternate_child.size()
                } else {
                    self.child.size()
                }
            }
        }
    }

    fn origin(&self) -> Option<Point> {
        match self.state.read() {
            DragState::None | DragState::WaitingToDrag { .. } => self.child.origin(),
            DragState::Dragging { .. } => {
                if let Some(alternate_child) = &self.alternate_drag_element {
                    alternate_child.origin()
                } else {
                    self.child.origin()
                }
            }
        }
    }
}

fn dispatch_callback(
    callback: Option<&mut Handler>,
    ctx: &mut EventContext,
    app: &AppContext,
    rect: RectF,
) {
    if let Some(callback) = callback {
        callback(ctx, app, rect);
    }
}

fn dispatch_drag_drop_callback(
    callback: Option<&mut DragDropHandler>,
    ctx: &mut EventContext,
    app: &AppContext,
    rect: RectF,
    drop_target_data: Option<&dyn DropTargetData>,
) {
    if let Some(callback) = callback {
        callback(ctx, app, rect, drop_target_data);
    }
}
