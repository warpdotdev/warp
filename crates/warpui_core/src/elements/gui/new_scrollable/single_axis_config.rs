use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::util::{scroll_clipped_scrollable_handle_with_delta, scroll_delta_for_axis};
use super::{NewScrollableElement, ScrollableAxis};
use crate::elements::new_scrollable::util::child_constraint_for_axis;
use crate::elements::{
    Axis, ClippedScrollStateHandle, F32Ext, ScrollData, ScrollStateHandle, ScrollTarget,
    ScrollToPositionMode, SelectableElement, Vector2FExt,
};
use crate::event::DispatchedEvent;
use crate::units::{IntoPixels, Pixels};
use crate::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// Holds state that depends on whether the scrolling axis should
/// be managed automatically by the scrollable (clipped) or it should
/// be managed manually by the child.
///
/// This config should only be used for single axis scrolling.
pub enum SingleAxisConfig {
    /// The child element is responsible for managing the scroll state manually.
    /// This means it has to 1) report scroll position to the scrollable at every
    /// frame. 2) expose API to allow scrollable to scroll to a certain position.
    Manual {
        handle: ScrollStateHandle,
        child: Box<dyn NewScrollableElement>,
    },
    /// The scrolling behavior is managed automatically by the scrollable. Note that
    /// this has worse performance than the manual variation since we need to layout the
    /// child with infinite bounds and clip to the visible viewport.
    Clipped {
        handle: ClippedScrollStateHandle,
        child: Box<dyn Element>,
    },
}

impl SingleAxisConfig {
    /// At run-time, validate if the passed-in axis config is valid.
    pub(super) fn validate(&self, axis: Axis) {
        #[cfg(debug_assertions)]
        {
            if let SingleAxisConfig::Manual { child, .. } = self {
                if matches!(axis, Axis::Horizontal)
                    && matches!(child.axis(), ScrollableAxis::Vertical)
                {
                    panic!(
                        "Set horizontal scrolling to be manual when the child element could only be scrolled on vertical axis"
                    );
                }

                if matches!(axis, Axis::Vertical)
                    && matches!(child.axis(), ScrollableAxis::Horizontal)
                {
                    panic!(
                        "Set vertical scrolling to be manual when the child element could only be scrolled on horizontal axis"
                    );
                }
            }
        }
    }

    /// Layout the child element in the single axis case and return the final scrollable size.
    pub(super) fn layout_child(
        &mut self,
        axis: Axis,
        constraint: SizeConstraint,
        scrollbar_size_with_padding: Vector2F,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let child_constraint = child_constraint_for_axis(
            axis,
            constraint,
            matches!(self, Self::Clipped { .. }),
            scrollbar_size_with_padding,
        );

        let child_size = match self {
            Self::Manual { child, .. } => child.layout(child_constraint, ctx, app),
            Self::Clipped { child, handle } => {
                let child_size = child.layout(child_constraint, ctx, app);
                let axis_size = match axis {
                    Axis::Horizontal => child_size.x(),
                    Axis::Vertical => child_size.y(),
                };
                let viewport_size = match axis {
                    Axis::Horizontal => constraint.max.x(),
                    Axis::Vertical => constraint.max.y(),
                };

                if axis_size < handle.scroll_start().as_f32() || viewport_size >= axis_size {
                    handle.scroll_to(Pixels::zero());
                } else {
                    // If viewport is still smaller than child but would cause unnecessary clipping,
                    // adjust scroll position to show rightmost/bottommost content
                    let max_scroll = (axis_size - viewport_size).max(0.0);
                    if handle.scroll_start().as_f32() > max_scroll {
                        handle.scroll_to(max_scroll.into_pixels());
                    }
                }
                child_size
            }
        };

        debug_assert!(
            child_size.y().is_finite(),
            "Scrollable's child should not have infinite height"
        );
        debug_assert!(
            child_size.x().is_finite(),
            "Scrollable's child should not have infinite width"
        );

        constraint.apply(child_size + scrollbar_size_with_padding)
    }

    /// Invoke child's after_layout and return the updated ScrollData.
    pub(super) fn after_layout(
        &mut self,
        axis: Axis,
        viewport_size: Vector2F,
        ctx: &mut AfterLayoutContext,
        app: &AppContext,
    ) -> ScrollData {
        match self {
            Self::Manual { child, .. } => {
                child.after_layout(ctx, app);
                child
                    .scroll_data(axis, app)
                    .expect("Child should have size at after layout")
            }
            Self::Clipped { child, handle } => {
                child.after_layout(ctx, app);
                handle.scroll_data(
                    viewport_size,
                    child
                        .size()
                        .expect("Child should have size at after layout"),
                    axis,
                )
            }
        }
    }

    pub(super) fn paint_child(
        &mut self,
        axis: Axis,
        origin: Vector2F,
        size: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        match self {
            Self::Clipped { handle, child } => {
                let scroll_target = handle.clipped_scroll_data.lock().scroll_to_position.take();
                if let Some(ScrollTarget { position_id, mode }) = scroll_target {
                    scroll_to_position_and_paint_clipped(
                        child,
                        axis,
                        origin,
                        size,
                        position_id,
                        mode,
                        handle,
                        ctx,
                        app,
                    );
                } else {
                    paint_clipped_internal(child, axis, origin, handle, ctx, app);
                }
            }
            Self::Manual { child, .. } => {
                child.paint(origin, ctx, app);
            }
        }
    }

    pub(super) fn child_bounds(&self) -> Option<RectF> {
        match self {
            Self::Manual { child, .. } => child.bounds(),
            Self::Clipped { child, .. } => child.bounds(),
        }
    }

    pub(super) fn child_as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        match self {
            Self::Manual { child, .. } => child.as_selectable_element(),
            Self::Clipped { child, .. } => child.as_selectable_element(),
        }
    }

    pub(super) fn scroll_offset(&self, axis: Axis) -> Vector2F {
        match self {
            Self::Manual { .. } => Vector2F::zero(),
            Self::Clipped { handle, .. } => handle.scroll_start().as_f32().along(axis),
        }
    }

    pub(super) fn child_hovered(&self) -> bool {
        match self {
            Self::Manual { handle, .. } => {
                handle.lock().expect("lock should be held").child_hovered
            }
            Self::Clipped { handle, .. } => handle.child_hovered(),
        }
    }

    pub(super) fn set_child_hovered(&self, hovered: bool) {
        match self {
            Self::Manual { handle, .. } => {
                handle.lock().expect("lock should be held").child_hovered = hovered
            }
            Self::Clipped { handle, .. } => handle.set_child_hovered(hovered),
        }
    }

    pub(super) fn hovered(&self) -> bool {
        match self {
            Self::Clipped { handle, .. } => handle.hovered(),
            Self::Manual { handle, .. } => handle.lock().unwrap().hovered,
        }
    }

    pub(super) fn set_hovered(&self, hovered: bool) {
        match self {
            Self::Clipped { handle, .. } => handle.set_hovered(hovered),
            Self::Manual { handle, .. } => handle.lock().unwrap().hovered = hovered,
        }
    }

    pub(super) fn dispatch_event_to_child(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        match self {
            Self::Manual { child, .. } => child.dispatch_event(event, ctx, app),
            Self::Clipped { child, .. } => child.dispatch_event(event, ctx, app),
        }
    }

    pub(super) fn set_drag_start(&self, position: Vector2F, axis: Axis) {
        match self {
            Self::Clipped { handle, .. } => handle.set_start(position.along(axis)),
            Self::Manual { handle, .. } => {
                handle.lock().unwrap().started = Some(position.along(axis))
            }
        }
    }

    pub(super) fn drag_start(&self) -> Option<f32> {
        match self {
            Self::Clipped { handle, .. } => handle.start(),
            Self::Manual { handle, .. } => handle.lock().unwrap().started,
        }
    }

    pub(super) fn end_drag(&self) {
        match self {
            Self::Clipped { handle, .. } => handle.reset_start(),
            Self::Manual { handle, .. } => handle.lock().unwrap().started = None,
        }
    }

    pub(super) fn scroll_data(
        &self,
        axis: Axis,
        viewport_size: Vector2F,
        app: &AppContext,
    ) -> ScrollData {
        match self {
            Self::Clipped { handle, child } => handle.scroll_data(
                viewport_size,
                child.size().expect("Size should exist"),
                axis,
            ),
            Self::Manual { child, .. } => child
                .scroll_data(axis, app)
                .expect("Axis is set to manual scrolling. Child should implement this axis"),
        }
    }

    /// Scroll child on the given axis with delta.
    pub(super) fn scroll_to(
        &mut self,
        viewport_size: Vector2F,
        delta: Pixels,
        axis: Axis,
        ctx: &mut EventContext,
    ) {
        // Early return if scroll delta is below sensitivity threshold.
        if delta.as_f32().abs() < f32::EPSILON {
            return;
        }

        match self {
            Self::Manual { child, .. } => child.scroll(delta, axis, ctx),
            Self::Clipped { handle, child } => {
                let child_size = child.size().expect("Size should exist");
                scroll_clipped_scrollable_handle_with_delta(
                    handle,
                    child_size.along(axis).into_pixels(),
                    viewport_size.along(axis).into_pixels(),
                    delta,
                    ctx,
                )
            }
        }
    }

    pub(super) fn should_handle_scroll_wheel(&self, axis: Axis) -> bool {
        match self {
            // If the scrolling is managed automatically, assume we should handle scroll wheel.
            Self::Clipped { .. } => true,
            Self::Manual { child, .. } => child.axis_should_handle_scroll_wheel(axis),
        }
    }

    /// Calculate whether given the current scroll state, would the scroll delta have any effect on the scrollable.
    /// We can then use this to filter whether to handle a scroll wheel event or not.
    pub(super) fn can_scroll_delta(
        &self,
        axis: Axis,
        viewport_size: Vector2F,
        delta: Vector2F,
        app: &AppContext,
    ) -> bool {
        let scroll_data = self.scroll_data(axis, viewport_size, app);
        let delta = delta.along(axis);

        Self::can_scroll_delta_dimension(&scroll_data, delta)
    }

    /// Calculate whether given the current scroll state, would the scroll delta have any effect on the scrollable in a single dimension.
    pub(super) fn can_scroll_delta_dimension(scroll_data: &ScrollData, delta: f32) -> bool {
        // If the scroll delta is 0, there is no effect.
        // If the scrollable is at the start of travel, and the delta is positive, there is no effect.
        // If the scrollable is at the end of travel, and the delta is negative, there is no effect.
        if (delta == 0.0)
            || (delta > 0.0 && scroll_data.scroll_start <= 0.0.into_pixels())
            || (delta < 0.0
                && scroll_data.scroll_start + scroll_data.visible_px >= scroll_data.total_size)
        {
            return false;
        }
        true
    }
}

fn paint_clipped_internal(
    child: &mut Box<dyn Element>,
    axis: Axis,
    origin: Vector2F,
    scroll_state: &ClippedScrollStateHandle,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    let offset = scroll_state.scroll_start().as_f32().along(axis);
    let child_origin = origin - offset;

    // It's possible that children elements of this ClippedScrollable are not a part
    // of a stack and therefore won't have their position's flushed to the position cache.
    // The start() and end() calls here ensure that the positions are saved so we can scroll
    // to the position of a child.
    ctx.position_cache.start();
    child.paint(child_origin, ctx, app);
    ctx.position_cache.end();
}

#[allow(clippy::too_many_arguments)]
fn scroll_to_position_and_paint_clipped(
    child: &mut Box<dyn Element>,
    axis: Axis,
    origin: Vector2F,
    size: Vector2F,
    position_id: String,
    mode: ScrollToPositionMode,
    scroll_state: &ClippedScrollStateHandle,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    // The relevant position can be a child of the `ClippedScrollable` so we need to first paint the
    // `ClippedScrollable` before we can determine the position, scroll the position into view, and
    // paint the element as intended. In order to prevent the first paint from having side effects,
    // we clone the scene before we invoke the first paint.
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
    paint_clipped_internal(child, axis, origin, scroll_state, ctx, app);

    if let Some(position_bounds) = ctx.position_cache.get_position(&position_id) {
        let child_bounds = child.bounds().expect("bounds on child should be set");
        // It doesn't make sense to scroll to a position that is unrelated to the
        // `ClippedScrollable` so no-op if it is not within the bounds of the child element.
        if child_bounds.intersects(position_bounds) {
            let viewport_bounds = RectF::new(origin, size);
            let delta = scroll_delta_for_axis(axis, viewport_bounds, position_bounds, mode);
            scroll_state.scroll_to(scroll_state.scroll_start() + delta.into_pixels());
        } else {
            log::warn!(
                "bounds of position ID {position_id}, {position_bounds:?}, are not contained \
                    in scrollable child bounds, {child_bounds:?}"
            );
        }
    } else {
        log::warn!("Position cache does not contain id: {position_id}");
    }

    *ctx.scene = cached_scene;
    paint_clipped_internal(child, axis, origin, scroll_state, ctx, app);
}
