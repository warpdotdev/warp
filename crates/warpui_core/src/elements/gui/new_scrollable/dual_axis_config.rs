use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};

use super::util::{scroll_clipped_scrollable_handle_with_delta, scroll_delta_for_axis};
use super::{NewScrollableElement, SingleAxisConfig};
use crate::elements::new_scrollable::util::child_constraint_for_axis;
use crate::elements::new_scrollable::ScrollableAxis;
use crate::elements::{
    Axis, ClippedScrollStateHandle, ScrollData, ScrollStateHandle, ScrollTarget, SelectableElement,
    Vector2FExt,
};
use crate::event::DispatchedEvent;
use crate::units::{IntoPixels, Pixels};
use crate::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

/// Holds different scroll state handle type that depends on
/// whether the caller wants automatic or manual scrolling.
///
/// This config is used for dual axis scrolling.
pub enum AxisConfiguration {
    /// The child element is responsible for managing the scroll state manually.
    /// This means it has to 1) report scroll position to the scrollable at every
    /// frame. 2) expose API to allow scrollable to scroll to a certain position.
    Manual(ScrollStateHandle),
    /// The scrolling behavior is managed automatically by the scrollable. Note that
    /// this has worse performance than the manual variation since we need to layout the
    /// child with infinite bounds and clip to the visible viewport.
    Clipped(ClippedAxisConfiguration),
}

#[derive(Default)]
pub struct ClippedAxisConfiguration {
    pub handle: ClippedScrollStateHandle,
    /// An optional max size the child should be laid out with in this axis.
    pub max_size: Option<f32>,
    /// Equivalent of [`crate::elements::CrossAxisAlignment::Stretch`].
    pub stretch_child: bool,
}

impl AxisConfiguration {
    /// Scroll data with the given axis' configuration. If it's clipped, we will read it from the scroll state handle.
    /// Otherwise, read it from the child element.
    fn scroll_data(
        &self,
        viewport_size: Vector2F,
        child: &dyn NewScrollableElement,
        axis: Axis,
        app: &AppContext,
    ) -> ScrollData {
        match self {
            Self::Manual(_) => child
                .scroll_data(axis, app)
                .expect("Axis is set to manual scrolling. Child should implement this axis"),
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => {
                handle.scroll_data(viewport_size, child.size().expect("Should exist"), axis)
            }
        }
    }

    /// Scroll the underlying element with the given axis' configuration. If it's clipped, update the scroll state handle.
    /// Otherwise, call scroll on the child element.
    fn scroll_to(
        &self,
        child: &mut dyn NewScrollableElement,
        viewport_size: Vector2F,
        delta: Pixels,
        axis: Axis,
        ctx: &mut EventContext,
    ) {
        match self {
            Self::Manual(_) => child.scroll(delta, axis, ctx),
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => {
                scroll_clipped_scrollable_handle_with_delta(
                    handle,
                    child
                        .size()
                        .expect("Size should exist")
                        .along(axis)
                        .into_pixels(),
                    viewport_size.along(axis).into_pixels(),
                    delta,
                    ctx,
                );
            }
        }
    }

    /// Set the start drag position for the scroll state.
    fn set_start(&self, position: f32) {
        match self {
            Self::Manual(handle) => handle.lock().unwrap().started = Some(position),
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => handle.set_start(position),
        }
    }

    /// Reset the start drag position to None for the scroll state.
    fn reset_start(&self) {
        match self {
            Self::Manual(handle) => handle.lock().unwrap().started = None,
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => handle.reset_start(),
        }
    }

    /// Read out the start drag postion state from scroll handle.
    fn start(&self) -> Option<f32> {
        match self {
            Self::Manual(handle) => handle.lock().unwrap().started,
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => handle.start(),
        }
    }

    /// The offset of the paint origin. If the child is managed manually on this axis, this should
    /// return 0 (child does viewporting itself). Otherwise, return the current scroll start from
    /// handle.
    fn paint_origin_offset(&self) -> f32 {
        match self {
            Self::Manual(_) => 0.,
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => {
                handle.scroll_start().as_f32()
            }
        }
    }

    /// Whether the axis' scrollbar is hovered.
    fn hovered(&self) -> bool {
        match self {
            Self::Manual(handle) => handle.lock().unwrap().hovered,
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => handle.hovered(),
        }
    }

    fn set_hovered(&self, hovered: bool) {
        match self {
            Self::Manual(handle) => handle.lock().unwrap().hovered = hovered,
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => handle.set_hovered(hovered),
        }
    }

    fn child_hovered(&self) -> bool {
        match self {
            Self::Manual(handle) => handle.lock().expect("lock should be held").child_hovered,
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => handle.child_hovered(),
        }
    }

    fn set_child_hovered(&self, hovered: bool) {
        match self {
            Self::Manual(handle) => {
                handle.lock().expect("lock should be held").child_hovered = hovered
            }
            Self::Clipped(ClippedAxisConfiguration { handle, .. }) => {
                handle.set_child_hovered(hovered)
            }
        }
    }

    // Calculate the updated size constraint based on the axis configuration.
    fn size_constraint(
        &self,
        axis: Axis,
        constraint: SizeConstraint,
        scrollbar_size_with_padding: Vector2F,
    ) -> (f32, f32) {
        let (mut min_size, mut max_size) = child_constraint_for_axis(
            axis,
            constraint,
            matches!(self, AxisConfiguration::Clipped { .. }),
            scrollbar_size_with_padding,
        )
        .constraint_for_axis(axis);

        if let AxisConfiguration::Clipped(ClippedAxisConfiguration {
            max_size: Some(max_width),
            stretch_child,
            ..
        }) = self
        {
            max_size = max_size.min(*max_width);
            if *stretch_child {
                min_size = match axis {
                    Axis::Horizontal => constraint.max.x(),
                    Axis::Vertical => constraint.max.y(),
                }
            }
        }
        (min_size, max_size)
    }
}

/// For manual scrolling, there could be three different scenarios:
/// * Manually managed scrolling on vertical + automatically managed scrolling on horizontal
/// * Manually managed scrolling on horizontal + automatically managed scrolling on vertical
/// * Manually managed scrolling on vertical and horizontal
///
/// For automatic scrolling, both axes have to be automatically managed. This relaxes
/// the child element requirement to be Element instead of ScrollableElement.
pub enum DualAxisConfig {
    /// The child element is responsible for managing the scroll state manually.
    /// This means it has to 1) report scroll position to the scrollable at every
    /// frame. 2) expose API to allow scrollable to scroll to a certain position.
    Manual {
        horizontal: AxisConfiguration,
        vertical: AxisConfiguration,
        child: Box<dyn NewScrollableElement>,
    },
    /// The scrolling behavior is managed automatically by the scrollable. Note that
    /// this has worse performance than the manual variation since we need to layout the
    /// child with infinite bounds and clip to the visible viewport.
    Clipped {
        horizontal: ClippedAxisConfiguration,
        vertical: ClippedAxisConfiguration,
        child: Box<dyn Element>,
    },
}

impl DualAxisConfig {
    /// At run-time, validate if the passed-in axis config is valid.
    pub(super) fn validate(&self) {
        #[cfg(debug_assertions)]
        {
            if let DualAxisConfig::Manual {
                horizontal,
                vertical,
                child,
            } = self
            {
                if matches!(horizontal, AxisConfiguration::Clipped { .. })
                    && matches!(vertical, AxisConfiguration::Clipped { .. })
                {
                    panic!(
                        "Tried to render a Manual scrollable with Clipped scrolling on both axes. Consider using DualAxisConfig::Clipped instead."
                    );
                }

                if matches!(horizontal, AxisConfiguration::Manual(_))
                    && matches!(child.axis(), ScrollableAxis::Vertical)
                {
                    panic!(
                        "Set horizontal scrolling to be manual when the child element could only be scrolled on vertical axis"
                    );
                }

                if matches!(vertical, AxisConfiguration::Manual(_))
                    && matches!(child.axis(), ScrollableAxis::Horizontal)
                {
                    panic!(
                        "Set vertical scrolling to be manual when the child element could only be scrolled on horizontal axis"
                    );
                }
            }
            log::trace!("Validated axes constructor");
        }
    }

    /// Return ScrollData for the given axis.
    pub(super) fn scroll_data(
        &self,
        viewport_size: Vector2F,
        axis: Axis,
        app: &AppContext,
    ) -> ScrollData {
        match &self {
            Self::Manual {
                horizontal,
                vertical,
                child,
            } => match axis {
                Axis::Horizontal => {
                    horizontal.scroll_data(viewport_size, child.as_ref(), Axis::Horizontal, app)
                }
                Axis::Vertical => {
                    vertical.scroll_data(viewport_size, child.as_ref(), Axis::Vertical, app)
                }
            },
            Self::Clipped {
                horizontal,
                vertical,
                child,
            } => match axis {
                Axis::Horizontal => horizontal.handle.scroll_data(
                    viewport_size,
                    child.size().expect("Should exist"),
                    axis,
                ),
                Axis::Vertical => vertical.handle.scroll_data(
                    viewport_size,
                    child.size().expect("Should exist"),
                    axis,
                ),
            },
        }
    }

    /// Layout the child element in the dual axis case and return the final scrollable size.
    pub(super) fn layout_child(
        &mut self,
        constraint: SizeConstraint,
        scrollbar_size_with_padding: Vector2F,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let (horizontal_min, horizontal_max) = match &self {
            // Take just the constraint for the horizontal axis.
            Self::Manual { horizontal, .. } => horizontal.size_constraint(
                Axis::Horizontal,
                constraint,
                scrollbar_size_with_padding,
            ),
            // If clipped, this should be just 0 to infinity.
            Self::Clipped {
                horizontal:
                    ClippedAxisConfiguration {
                        stretch_child,
                        max_size,
                        ..
                    },
                ..
            } => {
                let max_size = max_size.unwrap_or(f32::INFINITY);
                (
                    if *stretch_child {
                        max_size.min(constraint.max.x()) - scrollbar_size_with_padding.x()
                    } else {
                        0.
                    },
                    max_size,
                )
            }
        };

        let (vertical_min, vertical_max) = match &self {
            // Take just the constraint for the vertical axis.
            Self::Manual { vertical, .. } => {
                vertical.size_constraint(Axis::Vertical, constraint, scrollbar_size_with_padding)
            }
            // If clipped, this should be just 0 to infinity.
            Self::Clipped {
                vertical:
                    ClippedAxisConfiguration {
                        stretch_child,
                        max_size,
                        ..
                    },
                ..
            } => {
                let max_size = max_size.unwrap_or(f32::INFINITY);
                (
                    if *stretch_child {
                        max_size.min(constraint.max.y()) - scrollbar_size_with_padding.y()
                    } else {
                        0.
                    },
                    max_size,
                )
            }
        };

        let child_constraint = SizeConstraint {
            min: vec2f(horizontal_min, vertical_min),
            max: vec2f(horizontal_max, vertical_max),
        };

        let child_size = match self {
            Self::Manual { child, .. } => child.layout(child_constraint, ctx, app),
            Self::Clipped {
                child,
                horizontal,
                vertical,
            } => {
                let child_size = child.layout(child_constraint, ctx, app);
                // Reset scroll position if child becomes smaller than current scroll position
                // OR if viewport becomes larger than child size
                if child_size.x() < horizontal.handle.scroll_start().as_f32()
                    || constraint.max.x() >= child_size.x()
                {
                    horizontal.handle.scroll_to(Pixels::zero());
                } else {
                    // If viewport is still smaller than child but would cause unnecessary clipping,
                    // adjust scroll position to show rightmost content
                    let max_scroll = (child_size.x() - constraint.max.x()).max(0.0);
                    if horizontal.handle.scroll_start().as_f32() > max_scroll {
                        horizontal.handle.scroll_to(max_scroll.into_pixels());
                    }
                }

                if child_size.y() < vertical.handle.scroll_start().as_f32()
                    || constraint.max.y() >= child_size.y()
                {
                    vertical.handle.scroll_to(Pixels::zero());
                } else {
                    let max_scroll = (child_size.y() - constraint.max.y()).max(0.0);
                    if vertical.handle.scroll_start().as_f32() > max_scroll {
                        vertical.handle.scroll_to(max_scroll.into_pixels());
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
        viewport_size: Vector2F,
        ctx: &mut AfterLayoutContext,
        app: &AppContext,
    ) -> (ScrollData, ScrollData) {
        match self {
            Self::Manual { child, .. } => {
                child.after_layout(ctx, app);
            }
            Self::Clipped { child, .. } => {
                child.after_layout(ctx, app);
            }
        }

        let horizontal = self.scroll_data(viewport_size, Axis::Horizontal, app);
        let vertical = self.scroll_data(viewport_size, Axis::Vertical, app);
        (horizontal, vertical)
    }

    pub(super) fn paint_child(
        &mut self,
        origin: Vector2F,
        size: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        match self {
            Self::Clipped {
                horizontal,
                vertical,
                child,
            } => {
                let vertical_scroll_target = vertical
                    .handle
                    .clipped_scroll_data
                    .lock()
                    .scroll_to_position
                    .take();
                let horizontal_scroll_target = horizontal
                    .handle
                    .clipped_scroll_data
                    .lock()
                    .scroll_to_position
                    .take();
                if let Ok(scroll_to_position) =
                    ScrollToPosition::try_from((horizontal_scroll_target, vertical_scroll_target))
                {
                    scroll_to_position_and_paint_clipped(
                        child,
                        origin,
                        size,
                        scroll_to_position,
                        &vertical.handle,
                        &horizontal.handle,
                        ctx,
                        app,
                    );
                } else {
                    paint_clipped_internal(
                        child,
                        origin,
                        &vertical.handle,
                        &horizontal.handle,
                        ctx,
                        app,
                    );
                }
            }
            Self::Manual {
                horizontal,
                vertical,
                child,
            } => {
                let offset = vec2f(
                    horizontal.paint_origin_offset(),
                    vertical.paint_origin_offset(),
                );
                let child_origin = origin - offset;
                child.paint(child_origin, ctx, app);
            }
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

    pub(super) fn scroll_offset(&self) -> Vector2F {
        match self {
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => vec2f(
                horizontal.paint_origin_offset(),
                vertical.paint_origin_offset(),
            ),
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => vec2f(
                horizontal.handle.scroll_start().as_f32(),
                vertical.handle.scroll_start().as_f32(),
            ),
        }
    }

    pub(super) fn child_hovered(&self) -> bool {
        match self {
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => horizontal.child_hovered() || vertical.child_hovered(),
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => horizontal.handle.child_hovered() || vertical.handle.child_hovered(),
        }
    }

    pub(super) fn set_child_hovered(&self, hovered: bool) {
        match self {
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => {
                horizontal.set_child_hovered(hovered);
                vertical.set_child_hovered(hovered);
            }
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => {
                horizontal.handle.set_child_hovered(hovered);
                vertical.handle.set_child_hovered(hovered);
            }
        }
    }

    /// Returns whether the given axis scrollbar is hovered.
    pub(super) fn hovered(&self, axis: Axis) -> bool {
        match self {
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.handle.hovered(),
                Axis::Vertical => vertical.handle.hovered(),
            },
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.hovered(),
                Axis::Vertical => vertical.hovered(),
            },
        }
    }

    pub(super) fn set_hovered(&self, axis: Axis, hovered: bool) {
        match self {
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.handle.set_hovered(hovered),
                Axis::Vertical => vertical.handle.set_hovered(hovered),
            },
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.set_hovered(hovered),
                Axis::Vertical => vertical.set_hovered(hovered),
            },
        }
    }

    pub(super) fn set_drag_start(&self, position: Vector2F, axis: Axis) {
        match self {
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.handle.set_start(position.along(axis)),
                Axis::Vertical => vertical.handle.set_start(position.along(axis)),
            },
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.set_start(position.along(axis)),
                Axis::Vertical => vertical.set_start(position.along(axis)),
            },
        }
    }

    /// Read the current drag start position. We also return the axis here to distinguish between
    /// the two scrollbars.
    pub(super) fn drag_start(&self) -> Option<(f32, Axis)> {
        match self {
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => horizontal
                .start()
                .map(|start| (start, Axis::Horizontal))
                .or_else(|| vertical.start().map(|start| (start, Axis::Vertical))),
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => horizontal
                .handle
                .start()
                .map(|start| (start, Axis::Horizontal))
                .or_else(|| vertical.handle.start().map(|start| (start, Axis::Vertical))),
        }
    }

    /// End the current drag session.
    pub(super) fn end_drag(&self, axis: Axis) {
        match self {
            Self::Manual {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.reset_start(),
                Axis::Vertical => vertical.reset_start(),
            },
            Self::Clipped {
                horizontal,
                vertical,
                ..
            } => match axis {
                Axis::Horizontal => horizontal.handle.reset_start(),
                Axis::Vertical => vertical.handle.reset_start(),
            },
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
            Self::Manual {
                horizontal,
                vertical,
                child,
            } => match axis {
                Axis::Horizontal => {
                    horizontal.scroll_to(child.as_mut(), viewport_size, delta, axis, ctx)
                }
                Axis::Vertical => {
                    vertical.scroll_to(child.as_mut(), viewport_size, delta, axis, ctx)
                }
            },
            Self::Clipped {
                horizontal,
                vertical,
                child,
            } => {
                let child_size = child.size().expect("Size should exist");
                let axis_config = match axis {
                    Axis::Horizontal => horizontal,
                    Axis::Vertical => vertical,
                };
                scroll_clipped_scrollable_handle_with_delta(
                    &axis_config.handle,
                    child_size.along(axis).into_pixels(),
                    viewport_size.along(axis).into_pixels(),
                    delta,
                    ctx,
                )
            }
        }
    }

    /// Calculate whether given the current scroll state, would the scroll delta have any effect on the scrollable.
    /// We can then use this to filter whether to handle a scroll wheel event or not.
    pub(super) fn can_scroll_delta(
        &self,
        viewport_size: Vector2F,
        delta: Vector2F,
        app: &AppContext,
    ) -> bool {
        let horizontal_data = self.scroll_data(viewport_size, Axis::Horizontal, app);
        let vertical_data = self.scroll_data(viewport_size, Axis::Vertical, app);

        SingleAxisConfig::can_scroll_delta_dimension(&horizontal_data, delta.x())
            || SingleAxisConfig::can_scroll_delta_dimension(&vertical_data, delta.y())
    }

    pub(super) fn should_handle_scroll_wheel(&self, axis: Axis) -> bool {
        match self {
            // If the scrolling is managed automatically, assume we should handle scroll wheel.
            Self::Clipped { .. } => true,
            Self::Manual { child, .. } => child.axis_should_handle_scroll_wheel(axis),
        }
    }
}

/// Contains position ID(s) on either horizontal, vertical, or both axes.
///
/// This enum is similar to representing each axis with an Option<String> except that it prevents
/// (None, None) from being a possible state.
enum ScrollToPosition {
    Dual {
        horizontal: ScrollTarget,
        vertical: ScrollTarget,
    },
    Horizontal(ScrollTarget),
    Vertical(ScrollTarget),
}

impl TryFrom<(Option<ScrollTarget>, Option<ScrollTarget>)> for ScrollToPosition {
    type Error = ();

    fn try_from(value: (Option<ScrollTarget>, Option<ScrollTarget>)) -> Result<Self, Self::Error> {
        match value {
            (None, None) => Err(()),
            (None, Some(target)) => Ok(Self::Vertical(target)),
            (Some(target), None) => Ok(Self::Horizontal(target)),
            (Some(horizontal), Some(vertical)) => Ok(Self::Dual {
                horizontal,
                vertical,
            }),
        }
    }
}

fn paint_clipped_internal(
    child: &mut Box<dyn Element>,
    origin: Vector2F,
    vertical: &ClippedScrollStateHandle,
    horizontal: &ClippedScrollStateHandle,
    ctx: &mut PaintContext,
    app: &AppContext,
) {
    // If the child is clipped on an axis, the offset there is just the scroll_start
    // of the scroll handle.
    let offset = vec2f(
        horizontal.scroll_start().as_f32(),
        vertical.scroll_start().as_f32(),
    );
    let child_origin = origin - offset;

    // It's possible that children elements of this ClippedScrollabe are not a part
    // of a stack and therefore won't have their position's flushed to the position cache.
    // The start() and end() calls here ensure that the positions are saved so we can scroll
    // to the position of a child.
    ctx.position_cache.start();
    child.paint(child_origin, ctx, app);
    ctx.position_cache.end();
}

/// Scrolls the provided `position_id` into view, if it exists, and paints the object.
#[allow(clippy::too_many_arguments)]
fn scroll_to_position_and_paint_clipped(
    child: &mut Box<dyn Element>,
    origin: Vector2F,
    size: Vector2F,
    scroll_to_position: ScrollToPosition,
    vertical: &ClippedScrollStateHandle,
    horizontal: &ClippedScrollStateHandle,
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
    paint_clipped_internal(child, origin, vertical, horizontal, ctx, app);

    let child_bounds = child.bounds().expect("bounds on child should be set");
    let viewport_bounds = RectF::new(origin, size);

    if let ScrollToPosition::Horizontal(ref target)
    | ScrollToPosition::Dual {
        horizontal: ref target,
        ..
    } = scroll_to_position
    {
        if let Some(position_bounds) = ctx.position_cache.get_position(&target.position_id) {
            // It doesn't make sense to scroll to a position that is unrelated to the
            // `ClippedScrollable` so no-op if it is not within the bounds of the child element.
            if child_bounds.intersects(position_bounds) {
                let horizontal_delta = scroll_delta_for_axis(
                    Axis::Horizontal,
                    viewport_bounds,
                    position_bounds,
                    target.mode,
                );
                horizontal.scroll_to(horizontal.scroll_start() + horizontal_delta.into_pixels());
            } else {
                log::warn!(
                    "bounds of position ID {}, {position_bounds:?}, are not contained \
                    in scrollable child bounds, {child_bounds:?}",
                    target.position_id,
                );
            }
        } else {
            log::warn!("Position cache does not contain id: {}", target.position_id);
        }
    }

    if let ScrollToPosition::Vertical(ref target)
    | ScrollToPosition::Dual {
        vertical: ref target,
        ..
    } = scroll_to_position
    {
        if let Some(position_bounds) = ctx.position_cache.get_position(&target.position_id) {
            // It doesn't make sense to scroll to a position that is unrelated to the
            // `ClippedScrollable` so no-op if it is not within the bounds of the child element.
            if child_bounds.intersects(position_bounds) {
                let vertical_delta = scroll_delta_for_axis(
                    Axis::Vertical,
                    viewport_bounds,
                    position_bounds,
                    target.mode,
                );
                vertical.scroll_to(vertical.scroll_start() + vertical_delta.into_pixels());
            } else {
                log::warn!(
                    "bounds of position ID {}, {position_bounds:?}, are not contained \
                    in scrollable child bounds, {child_bounds:?}",
                    target.position_id,
                );
            }
        } else {
            log::warn!("Position cache does not contain id: {}", target.position_id);
        }
    }

    *ctx.scene = cached_scene;
    paint_clipped_internal(child, origin, vertical, horizontal, ctx, app);
}
