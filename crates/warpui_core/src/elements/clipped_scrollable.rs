use parking_lot::Mutex;
use std::sync::Arc;

use pathfinder_geometry::{rect::RectF, vector::Vector2F};

use crate::scene::ClipBounds;
use crate::units::{IntoPixels, Pixels};
use crate::{
    event::DispatchedEvent, AfterLayoutContext, AppContext, Element, EventContext, LayoutContext,
    PaintContext, SizeConstraint,
};

use super::{
    new_scrollable::util::scroll_delta_for_axis, Axis, F32Ext, Fill, Point, ScrollData,
    ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Selection, Vector2FExt,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollToPositionMode {
    /// Scroll the minimum amount to bring as much of the element into view
    /// as possible.
    FullyIntoView,
    /// Show as much of the element as possible, prioritising the top (leading)
    /// edge. Behaves like [`FullyIntoView`] when the element fits within the
    /// viewport, but when the element is taller than the viewport it aligns
    /// the element's top with the viewport's top.
    TopIntoView,
}

#[derive(Clone)]
pub struct ScrollTarget {
    pub position_id: String,
    pub mode: ScrollToPositionMode,
}

#[derive(Clone, Default)]
pub struct ClippedScrollData {
    scroll_start_px: Pixels,
    pub(super) scroll_to_position: Option<ScrollTarget>,
    selection_scroll_anchor: Option<ClippedSelectionScrollAnchor>,
}

#[derive(Clone, Copy)]
struct ClippedSelectionScrollAnchor {
    selection: Selection,
    scroll_start_px: Pixels,
}

impl ClippedSelectionScrollAnchor {
    fn matches(&self, selection: Selection) -> bool {
        self.selection == selection
    }
}

#[derive(Clone, Default)]
pub struct ClippedScrollStateHandle {
    /// The scroll state for the [`Scrollable`] that wraps the [`ClippedScrollable`].
    /// This is included as part of this handle for ergonomics; otherwise, each
    /// [`ClippedScrollable`] consumer would need to separately maintain a
    /// [`ScrollStateHandle`] and a [`ClippedScrollStateHandle`].
    scrollable_data: ScrollStateHandle,
    pub(super) clipped_scroll_data: Arc<Mutex<ClippedScrollData>>,
}

impl ClippedScrollStateHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scroll_to(&self, start: Pixels) {
        self.clipped_scroll_data.lock().scroll_start_px = start.max(Pixels::zero());
    }

    pub fn scroll_start(&self) -> Pixels {
        self.clipped_scroll_data.lock().scroll_start_px
    }

    pub fn scroll_by(&self, delta: Pixels) {
        self.scroll_to(self.scroll_start() + delta);
    }

    /// Records `selection` as the current selection scroll anchor (if not already recorded) and
    /// returns a copy of it whose coordinates have been compensated for any scroll that has
    /// happened since the anchor was first recorded.
    ///
    /// This is **not** a pure transformation — it mutates the handle's internal
    /// `selection_scroll_anchor` state as a side effect:
    /// - Passing `None` clears the anchor and returns `None`.
    /// - Passing a `Selection` that does not match the currently recorded anchor replaces the
    ///   anchor with a new one capturing `selection` at the current scroll position and returns
    ///   `selection` unchanged.
    /// - Passing a `Selection` that matches the currently recorded anchor leaves the anchor in
    ///   place and returns `selection` shifted by the delta between the current scroll position
    ///   and the anchor's recorded scroll position.
    ///
    /// The net effect is that as long as callers feed the same selection in across repaints, the
    /// returned selection tracks the underlying content even while the surface is being scrolled.
    pub(crate) fn anchor_and_adjust_selection_for_scroll(
        &self,
        selection: Option<Selection>,
        axis: Axis,
    ) -> Option<Selection> {
        let Some(selection) = selection else {
            self.clipped_scroll_data.lock().selection_scroll_anchor = None;
            return None;
        };

        let mut scroll_data = self.clipped_scroll_data.lock();
        let scroll_start_px = scroll_data.scroll_start_px;
        let anchor_scroll_start = match scroll_data.selection_scroll_anchor {
            Some(anchor) if anchor.matches(selection) => anchor.scroll_start_px,
            _ => {
                scroll_data.selection_scroll_anchor = Some(ClippedSelectionScrollAnchor {
                    selection,
                    scroll_start_px,
                });
                scroll_start_px
            }
        };

        let scroll_delta = (scroll_start_px - anchor_scroll_start).as_f32().along(axis);
        Some(Selection {
            start: selection.start - scroll_delta,
            end: selection.end - scroll_delta,
            is_rect: selection.is_rect,
        })
    }

    pub(crate) fn clear_selection_scroll_anchor(&self) {
        self.clipped_scroll_data.lock().selection_scroll_anchor = None;
    }

    pub fn set_start(&self, position: f32) {
        self.scrollable_data.lock().unwrap().started = Some(position);
    }

    pub fn reset_start(&self) {
        self.scrollable_data.lock().unwrap().started = None;
    }

    pub fn start(&self) -> Option<f32> {
        self.scrollable_data.lock().unwrap().started
    }

    /// Scrolls the bounds of the element described by `target` into view.
    /// This is a no-op if the position is already in view or is not within
    /// the bounds of the `ClippedScrollable`.
    pub fn scroll_to_position(&self, target: ScrollTarget) {
        self.clipped_scroll_data.lock().scroll_to_position = Some(target);
    }

    pub fn hovered(&self) -> bool {
        self.scrollable_data.lock().unwrap().hovered
    }

    pub fn set_hovered(&self, hovered: bool) {
        self.scrollable_data.lock().unwrap().hovered = hovered;
    }

    pub(in crate::elements) fn set_child_hovered(&self, hovered: bool) {
        self.scrollable_data
            .lock()
            .expect("lock should be held")
            .child_hovered = hovered;
    }

    pub(in crate::elements) fn child_hovered(&self) -> bool {
        self.scrollable_data
            .lock()
            .expect("lock should be held")
            .child_hovered
    }
}

/// Implements a generic scrollable interface around an arbitrary child element
/// tree using clipping to control what's rendered.
/// Note that this scroll path is by its nature slow because in order to
/// use it we need to fully lay out the child tree, determine its size
/// paint it, and then clip it.
/// It's much better to have a child that explicitly implements ScrollableElement
/// where possible, but it's fine to use this when that's not possible.
///
/// TODO: there is currently a bug with constraint-passing when nesting
/// [`ClippedScrollable`]s (e.g. to get clipped scrolling in both directions).
pub struct ClippedScrollable {
    axis: Axis,
    child: Box<dyn Element>,
    state: ClippedScrollStateHandle,
    size: Option<Vector2F>,
    origin: Option<Point>,
    /// When true, the child constraint's min on the main axis is set to the
    /// incoming constraint's max on the main axis (if finite). This allows
    /// the child (e.g. an [`Align`]) to know the visible height and center
    /// content within the scrollable area.
    fill_min_main_axis: bool,
}

impl ClippedScrollable {
    fn new(axis: Axis, child: Box<dyn Element>, state: ClippedScrollStateHandle) -> Self {
        Self {
            axis,
            child,
            state,
            size: None,
            origin: None,
            fill_min_main_axis: false,
        }
    }

    /// Constructs a new [`Scrollable`] element that scrolls vertically,
    /// using a [`ClippedScrollable`] as the concrete [`ScrollableElement`].
    pub fn vertical(
        state: ClippedScrollStateHandle,
        child: Box<dyn Element>,
        scrollbar_size: ScrollbarWidth,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Scrollable {
        Scrollable::vertical(
            state.scrollable_data.clone(),
            ClippedScrollable::new(Axis::Vertical, child, state).finish_scrollable(),
            scrollbar_size,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
        )
    }

    /// Like [`vertical`](Self::vertical), but passes the visible height as
    /// the child's min-height constraint. This allows the child (e.g. wrapped
    /// in [`Align`]) to center its content within the visible area while
    /// still being scrollable when content overflows.
    pub fn vertical_centered(
        state: ClippedScrollStateHandle,
        child: Box<dyn Element>,
        scrollbar_size: ScrollbarWidth,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Scrollable {
        let mut cs = ClippedScrollable::new(Axis::Vertical, child, state.clone());
        cs.fill_min_main_axis = true;
        Scrollable::vertical(
            state.scrollable_data.clone(),
            cs.finish_scrollable(),
            scrollbar_size,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
        )
    }

    /// Constructs a new [`Scrollable`] element that scrolls horizontally,
    /// using a [`ClippedScrollable`] as the concrete [`ScrollableElement`].
    pub fn horizontal(
        state: ClippedScrollStateHandle,
        child: Box<dyn Element>,
        scrollbar_size: ScrollbarWidth,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Scrollable {
        Scrollable::horizontal(
            state.scrollable_data.clone(),
            ClippedScrollable::new(Axis::Horizontal, child, state).finish_scrollable(),
            scrollbar_size,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
        )
    }

    fn paint_internal(
        &mut self,
        origin: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
        size: Vector2F,
    ) {
        ctx.scene
            .start_layer(ClipBounds::BoundedBy(RectF::new(origin, size)));
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        // It's possible that children elements of this ClippedScrollabe are not a part
        // of a stack and therefore won't have their position's flushed to the position cache.
        // The start() and end() calls here ensure that the positions are saved so we can scroll
        // to the position of a child.
        ctx.position_cache.start();
        self.child.paint(
            origin - self.state.scroll_start().as_f32().along(self.axis),
            ctx,
            app,
        );
        ctx.position_cache.end();

        ctx.scene.stop_layer();
    }

    /// Scrolls the provided `position_id` into view, if it exists, and paints the object.
    fn scroll_to_position_and_paint(
        &mut self,
        origin: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
        size: Vector2F,
        position_id: String,
        mode: ScrollToPositionMode,
    ) {
        // The relevant position can be a child of the `ClippedScrollable` so we need to first paint the
        // `ClippedScrollable` before we can determine the position, scroll the position into view, and paint the element as intended.
        // In order to prevent the first paint from having side effects, we clone the scene
        // before we invoke the first paint.
        //
        // Cloning the scene is cheap! On a bundled app, the following operations take < 10 microseconds:
        // - 100 warp tabs open
        // - Set line height to 0.2 and fill the block list and make a large number of glyphs
        // - Expanded all folders in warp drive and opened command palette (to check non-view ported elements)
        // - Render many images (as it turns out the scene only holds a rect and Arc, not the image content itself)
        // We want to avoid excesively cloning the scene though, because calling clone on the scene on multiple
        // `ClippedScrollable` elements in the paint code path caused this latency to be an order of magnitude
        // higher (300 microseconds).
        let cached_scene = ctx.scene.clone();
        self.paint_internal(origin, ctx, app, size);

        if let Some(position_bounds) = ctx.position_cache.get_position(position_id) {
            let child_bounds = self.child.bounds().expect("bounds on child should be set");
            // It doesn't make sense to scroll to a position that is unrelated to the `ClippedScrollable`
            // so no-op if it is not within the bounds of the child element.
            if child_bounds.contains_rect(position_bounds) {
                let scroll_top = self.state.scroll_start();
                let viewport_bounds = self.bounds().expect("bounds should be set");

                let scroll_delta =
                    scroll_delta_for_axis(self.axis, viewport_bounds, position_bounds, mode);

                self.state
                    .scroll_to(scroll_top + scroll_delta.into_pixels());

                *ctx.scene = cached_scene;
                self.paint_internal(origin, ctx, app, size);
            }
        }
    }
}

impl Element for ClippedScrollable {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // The child should only be constrained horizontally, and allowed to grow
        // as tall as it desires.  The height of the ClippedScrollable will still be
        // constrained by the incoming constraints.
        let mut child_constraint = SizeConstraint::tight_on_cross_axis(self.axis, constraint);

        // When fill_min_main_axis is set, pass the visible size along the main
        // axis as the child's min constraint so centering elements (e.g. Align)
        // can fill and center their content within the visible area.
        if self.fill_min_main_axis {
            let visible = constraint.max.along(self.axis);
            if visible.is_finite() {
                match self.axis {
                    Axis::Vertical => child_constraint.min.set_y(visible),
                    Axis::Horizontal => child_constraint.min.set_x(visible),
                }
            }
        }

        let child_size = self.child.layout(child_constraint, ctx, app);
        let size = constraint.apply(child_size);
        self.size = Some(size);
        size
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let size = self.size().expect("size should be set by paint time");
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(origin, size));

        let scroll_target = self
            .state
            .clipped_scroll_data
            .lock()
            .scroll_to_position
            .take();
        if let Some(ScrollTarget { position_id, mode }) = scroll_target {
            self.scroll_to_position_and_paint(origin, ctx, app, size, position_id, mode);
        } else {
            self.paint_internal(origin, ctx, app, size);
        }
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

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
        // Make sure that the new layout doesn't put the scroll bar in an invalid
        // location.
        if let Some(scroll_data) = self.scroll_data(app) {
            let max_scroll_top =
                (scroll_data.total_size - scroll_data.visible_px).max(Pixels::zero());
            let scroll_top = scroll_data.scroll_start;
            if scroll_top > max_scroll_top {
                self.state.scroll_to(max_scroll_top);
            }
        }
    }
}

impl ScrollableElement for ClippedScrollable {
    fn scroll_data(&self, _app: &AppContext) -> Option<ScrollData> {
        Some(ScrollData {
            scroll_start: self.state.scroll_start(),
            visible_px: (self.size()?.along(self.axis)).into_pixels(),
            total_size: self.child.size()?.along(self.axis).into_pixels(),
        })
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        let scroll_start = self.state.scroll_start();
        let child_size: Pixels = self
            .child
            .size()
            .expect("child should be laid out before scrolling")
            .along(self.axis)
            .into_pixels();

        let clipped_size = self
            .size
            .expect("should be laid out before scrolling")
            .along(self.axis)
            .into_pixels();
        if child_size > clipped_size {
            let new_scroll_start = (scroll_start - delta)
                .max(Pixels::zero())
                .min(child_size - clipped_size);
            if (scroll_start - new_scroll_start).as_f32().abs() > f32::EPSILON {
                self.state.scroll_to(new_scroll_start);
                ctx.notify();
            }
        }
    }

    fn should_handle_scroll_wheel(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "clipped_scrollable_tests.rs"]
mod tests;
