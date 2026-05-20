#![allow(dead_code)]

mod dual_axis_config;
mod single_axis_config;
pub(crate) mod util;

pub use dual_axis_config::*;
pub use single_axis_config::*;

use pathfinder_color::ColorU;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};

use crate::{
    elements::Vector2FExt,
    event::{DispatchedEvent, ModifiersState},
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
    units::{IntoPixels, Pixels},
    AfterLayoutContext, AppContext, ClipBounds, Element, Event, EventContext, LayoutContext,
    PaintContext, SizeConstraint,
};

use self::util::adjust_scroll_delta_with_sensitivity_config;

use super::{
    scrollbar_size, Axis, ClippedScrollStateHandle, CornerRadius, F32Ext, Fill, Point, Radius,
    ScrollData, ScrollbarWidth, SelectableElement, SelectionFragment, ZIndex,
};

const LEFT_PADDING: f32 = 2.;
const RIGHT_PADDING: f32 = 2.;
const MINIMUM_HEIGHT: f32 = 20.;

// TODO: we might want this to be configurable.
const DUAL_AXES_SCROLL_SENSITIVITY: f32 = 1.0;

/// The number of pixels-per-line when dealing with a cocoa scroll event
/// that lacks precision (i.e. [`hasPreciseScrollingDeltas`](https://developer.apple.com/documentation/appkit/nsevent/1525758-hasprecisescrollingdeltas?language=objc))
/// is false. While some mouse devices provide finer scroll deltas
/// (in pixels), other generic devices don't and we thus have to convert the
/// provided non-precise scroll deltas (which are in terms of lines) into pixels.
///
/// While we could use the application line-height to calculate the number of pixels,
/// this requires us to couple the scrolling APIs with `Lines`, which doesn't apply
/// for horizontal scrolling.
///
/// We also decided to not use [`CGEventSourceGetPixelsPerLine`](https://developer.apple.com/documentation/coregraphics/1408775-cgeventsourcegetpixelsperline)
/// because it defaults to ~10 pixels per line, which makes scrolling feel slow compared to other applications.
///
/// The value we chose is inspired by the value that Chromium and Flutter use:
/// - https://chromium.googlesource.com/chromium/src/+/9306606fbbd1ebf51cfe23ea6bcfa19a1ff43363/ui/events/cocoa/events_mac.mm#158
/// - https://github.com/flutter/engine/blob/cc925b0021330759e18960e1ccbd7e55dec3c375/shell/platform/darwin/macos/framework/Source/FlutterViewController.mm#L768-L775.
///
/// TODO: currently, this constant reflects the value that makes sense for MacOS (cocoa) scroll events.
/// Ideally, we should hide this implementation detail at the platform level and have consumers
/// solely operate with pixel-based scroll events.
const NUM_PIXELS_PER_LINE: f32 = 40.;

/// Trait a scrollable child element needs to implement to enable manual scrolling.
/// The element could support scrolling on: horizontal axis, vertical axis, or both axes.
pub trait NewScrollableElement: Element {
    /// What axis the child is scrollable on. It's the implementer's responsibility to
    /// make sure this accurately reflects the element's scrolling behavior. Scrollable uses this
    /// information to validate if the caller's configuration is valid.
    fn axis(&self) -> ScrollableAxis;

    /// Returns scrolling data that the child computes and that the [`Scrollable`]
    /// uses to update its internal state. If the child is scrollable
    /// (i.e. the child has been laid out), this must be [`Some`].
    fn scroll_data(&self, axis: Axis, app: &AppContext) -> Option<ScrollData>;

    /// Scrolls the element by the given `delta` (in pixels).
    fn scroll(&mut self, delta: Pixels, axis: Axis, ctx: &mut EventContext);

    /// By default, scrollable elements are responsible for their own wheel handling.
    /// Override to return true if you want the parent scrollable to handle the wheel.
    fn axis_should_handle_scroll_wheel(&self, _axis: Axis) -> bool {
        false
    }

    fn finish_scrollable(self) -> Box<dyn NewScrollableElement>
    where
        Self: 'static + Sized,
    {
        Box::new(self)
    }
}

/// Which axis the child element is scrollable on.
#[derive(Debug)]
pub enum ScrollableAxis {
    Horizontal,
    Vertical,
    Both,
}

/// The appearance configuration of each scrollbar. Scrollable supports different appearance settings
/// on each axis when the element is scrollable on both axes.
#[derive(Default, Clone, Copy, Debug)]
pub struct ScrollableAppearance {
    /// The size of the scrollbar in pixels.
    scrollbar_size: ScrollbarWidth,
    // The scrollbar is the runway for the draggable scrollbar. By default the scollbox renders to
    // the side of the child element. This setting makes the scrollbar render over the child instead.
    overlaid_scrollbar: bool,
}

impl ScrollableAppearance {
    pub fn new(scrollbar_size: ScrollbarWidth, overlaid_scrollbar: bool) -> Self {
        Self {
            scrollbar_size,
            overlaid_scrollbar,
        }
    }

    fn scrollbar_size(&self, include_overlaid_scrollbar: bool) -> f32 {
        if !include_overlaid_scrollbar && self.overlaid_scrollbar {
            0.
        } else {
            self.scrollbar_size.as_f32()
        }
    }
}

/// Internal state of the scrollable configuration.
enum ScrollableState {
    SingleAxis {
        axis: Axis,
        config: SingleAxisConfig,
        appearance: ScrollableAppearance,
        render_state: ScrollbarRenderState,
    },
    BothAxes {
        config: DualAxisConfig,
        horizontal_appearance: ScrollableAppearance,
        vertical_appearance: ScrollableAppearance,
        horizontal_state: ScrollbarRenderState,
        vertical_state: ScrollbarRenderState,
    },
}

impl ScrollableState {
    /// Returns the size the scrollable's scrollbar(s) would take on each axis.
    fn scrollbar_size(&self, include_overlaid_scrollbar: bool) -> Vector2F {
        match self {
            Self::SingleAxis {
                axis, appearance, ..
            } => appearance
                .scrollbar_size(include_overlaid_scrollbar)
                .along(axis.invert()),
            Self::BothAxes {
                horizontal_appearance,
                vertical_appearance,
                ..
            } => vec2f(
                vertical_appearance.scrollbar_size(include_overlaid_scrollbar),
                horizontal_appearance.scrollbar_size(include_overlaid_scrollbar),
            ),
        }
    }

    /// Returns the total padding the scrollable's scrollbar(s) would take on each axis.
    fn scrollbar_padding(&self, include_overlaid_scrollbar: bool) -> Vector2F {
        match self {
            Self::SingleAxis {
                axis,
                render_state,
                appearance,
                ..
            } => render_state
                .scrollbar_padding(*appearance, include_overlaid_scrollbar)
                .along(axis.invert()),
            Self::BothAxes {
                horizontal_state,
                vertical_state,
                vertical_appearance,
                horizontal_appearance,
                ..
            } => vec2f(
                vertical_state.scrollbar_padding(*vertical_appearance, include_overlaid_scrollbar),
                horizontal_state
                    .scrollbar_padding(*horizontal_appearance, include_overlaid_scrollbar),
            ),
        }
    }

    /// Layout the child element with the incoming size constraint.
    fn layout_child(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let scrollbar_size_with_padding =
            self.scrollbar_size(false) + self.scrollbar_padding(false);
        match self {
            Self::SingleAxis { axis, config, .. } => {
                config.layout_child(*axis, constraint, scrollbar_size_with_padding, ctx, app)
            }
            Self::BothAxes { config, .. } => {
                config.layout_child(constraint, scrollbar_size_with_padding, ctx, app)
            }
        }
    }

    fn after_layout(
        &mut self,
        scrollable_size: Vector2F,
        ctx: &mut AfterLayoutContext,
        app: &AppContext,
    ) {
        let viewport_size = self.viewport_size(scrollable_size);
        match self {
            Self::BothAxes {
                config,
                horizontal_state,
                vertical_state,
                ..
            } => {
                // First invoke after_layout on the child.
                let (horizontal_data, vertical_data) = config.after_layout(viewport_size, ctx, app);

                // Update the render state with the latest ScrollData.
                horizontal_state.update_with_scroll_data(
                    horizontal_data,
                    viewport_size.along(Axis::Horizontal),
                );
                vertical_state
                    .update_with_scroll_data(vertical_data, viewport_size.along(Axis::Vertical));
            }
            Self::SingleAxis {
                axis,
                config,
                render_state,
                ..
            } => {
                // First invoke after_layout on the child.
                let scroll_data = config.after_layout(*axis, viewport_size, ctx, app);
                // Update the render state with the latest ScrollData.
                render_state.update_with_scroll_data(scroll_data, viewport_size.along(*axis));
            }
        }
    }

    /// Paint the child element.
    fn paint_child(
        &mut self,
        origin: Vector2F,
        size: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        match self {
            Self::BothAxes { config, .. } => config.paint_child(origin, size, ctx, app),
            Self::SingleAxis { axis, config, .. } => {
                config.paint_child(*axis, origin, size, ctx, app)
            }
        }
    }

    fn dispatch_event_to_child(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        match self {
            Self::BothAxes { config, .. } => config.dispatch_event_to_child(event, ctx, app),
            Self::SingleAxis { config, .. } => config.dispatch_event_to_child(event, ctx, app),
        }
    }

    fn child_as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        match self {
            Self::BothAxes { config, .. } => config.child_as_selectable_element(),
            Self::SingleAxis { config, .. } => config.child_as_selectable_element(),
        }
    }

    fn scroll_offset(&self) -> Vector2F {
        match self {
            Self::BothAxes { config, .. } => config.scroll_offset(),
            Self::SingleAxis { axis, config, .. } => config.scroll_offset(*axis),
        }
    }

    /// Paint the scrollbars on both axes.
    fn draw_scrollbars(
        &mut self,
        origin: Vector2F,
        scrollable_size: Vector2F,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
        ctx: &mut PaintContext,
    ) {
        // Consider the overlaid scrollbar when calculating sizing. This will be used for drawing the scrollbar.
        let scrollbar_size_with_padding = self.scrollbar_size(true) + self.scrollbar_padding(true);
        match self {
            Self::BothAxes {
                config,
                horizontal_appearance,
                vertical_appearance,
                vertical_state,
                horizontal_state,
            } => {
                vertical_state.draw_scrollbar(
                    Axis::Vertical,
                    scrollable_size,
                    scrollbar_size_with_padding,
                    origin,
                    if config.hovered(Axis::Vertical) {
                        active_scrollbar_thumb_background
                    } else if !config.child_hovered() {
                        Fill::None
                    } else {
                        nonactive_scrollbar_thumb_background
                    },
                    scrollbar_track_background,
                    *vertical_appearance,
                    ctx,
                );

                horizontal_state.draw_scrollbar(
                    Axis::Horizontal,
                    scrollable_size,
                    scrollbar_size_with_padding,
                    origin,
                    if config.hovered(Axis::Horizontal) {
                        active_scrollbar_thumb_background
                    } else if !config.child_hovered() {
                        Fill::None
                    } else {
                        nonactive_scrollbar_thumb_background
                    },
                    scrollbar_track_background,
                    *horizontal_appearance,
                    ctx,
                );

                // If both scrollbars are not overlaid. There will be a bottom right area (marked with asterisk)
                // ============================================
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // |                                       |  |
                // ============================================
                // |                                       |**|
                // ============================================
                //
                // Paint it with the scrollbar track background.
                let viewport_size =
                    (scrollable_size - scrollbar_size_with_padding).max(Vector2F::zero());
                if !vertical_appearance.overlaid_scrollbar
                    && !horizontal_appearance.overlaid_scrollbar
                {
                    let bottom_right_rect =
                        RectF::new(origin + viewport_size, scrollbar_size_with_padding);
                    ctx.scene
                        .draw_rect_with_hit_recording(bottom_right_rect)
                        .with_background(scrollbar_track_background);
                }
            }
            Self::SingleAxis {
                axis,
                config,
                appearance,
                render_state,
            } => render_state.draw_scrollbar(
                *axis,
                scrollable_size,
                scrollbar_size_with_padding,
                origin,
                if config.hovered() {
                    active_scrollbar_thumb_background
                } else if !config.child_hovered() {
                    Fill::None
                } else {
                    nonactive_scrollbar_thumb_background
                },
                scrollbar_track_background,
                *appearance,
                ctx,
            ),
        }
    }

    /// Size of the current viewport. This excludes the scrollbar tracks.
    fn viewport_size(&self, scrollable_size: Vector2F) -> Vector2F {
        (scrollable_size - self.scrollbar_padding(false) - self.scrollbar_size(false))
            .max(Vector2F::zero())
    }

    /// Handle a mouse down event. If the click position is inbound of the scrollbar thumb,
    /// start a scroll drag event. If the click position is inbound of the scrollbar track but not the thumb,
    /// jump to the mouse down position.
    fn mouse_down(
        &mut self,
        position: Vector2F,
        scrollable_size: Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        match self {
            Self::BothAxes {
                horizontal_state,
                vertical_state,
                config,
                ..
            } => {
                let drag_start_direction = if horizontal_state
                    .scrollbar_thumb_bounds
                    .expect("Thumb bound should exist")
                    .contains_point(position)
                {
                    Some(Axis::Horizontal)
                } else if vertical_state
                    .scrollbar_thumb_bounds
                    .expect("Thumb bound should exist")
                    .contains_point(position)
                {
                    Some(Axis::Vertical)
                } else {
                    None
                };

                if let Some(axis) = drag_start_direction {
                    config.set_drag_start(position, axis);

                    // Dispatch an action in tests so we can perform assertions
                    // on clicks.
                    #[cfg(test)]
                    ctx.dispatch_action("scrollable_click::on_thumb", ());

                    return true;
                }

                let scroll_track_hit = if horizontal_state
                    .scrollbar_track_bounds
                    .expect("Track bound should exist")
                    .contains_point(position)
                {
                    Some((
                        Axis::Horizontal,
                        horizontal_state
                            .scrollbar_thumb_bounds
                            .expect("Thumb bound should exist"),
                    ))
                } else if vertical_state
                    .scrollbar_track_bounds
                    .expect("Track bound should exist")
                    .contains_point(position)
                {
                    Some((
                        Axis::Vertical,
                        vertical_state
                            .scrollbar_thumb_bounds
                            .expect("Thumb bound should exist"),
                    ))
                } else {
                    None
                };

                if let Some((axis, scrollbar_thumb_bounds)) = scroll_track_hit {
                    // If the scrollbar thumb has no area, then the `Scrollable` is not large enough
                    // in this axis for scrolling to be relevant (i.e. no scrollbar thumb will be painted).
                    // In such cases, we should return `false` so that child elements can still
                    // receive the `LeftMouseDown` event (i.e. for editor text selection).
                    if scrollbar_thumb_bounds.is_empty() {
                        return false;
                    }

                    // If mouse down happens in the x range of scrollbar but not on the thumb,
                    // we should scroll to the mouse down position.
                    let previous_position = scrollbar_thumb_bounds.center().along(axis);

                    self.jump_to_position(
                        previous_position.into_pixels(),
                        position.along(axis).into_pixels(),
                        scrollable_size,
                        axis,
                        ctx,
                        app,
                    );

                    // Dispatch an action in tests so we can perform assertions
                    // on clicks.
                    #[cfg(test)]
                    ctx.dispatch_action("scrollable_click::on_gutter", ());

                    return true;
                }

                false
            }
            Self::SingleAxis {
                config,
                axis,
                render_state,
                ..
            } => {
                let current_axis = *axis;
                if render_state
                    .scrollbar_thumb_bounds
                    .expect("Thumb bound should exist")
                    .contains_point(position)
                {
                    config.set_drag_start(position, current_axis);

                    // Dispatch an action in tests so we can perform assertions
                    // on clicks.
                    #[cfg(test)]
                    ctx.dispatch_action("scrollable_click::on_thumb", ());

                    true
                } else if render_state
                    .scrollbar_track_bounds
                    .expect("Track bound should exist")
                    .contains_point(position)
                {
                    // If the scrollbar thumb has no area, then the `Scrollable` is not large enough
                    // in this axis for scrolling to be relevant (i.e. no scrollbar thumb will be painted).
                    // In such cases, we should return `false` so that child elements can still
                    // receive the `LeftMouseDown` event (i.e. for editor text selection).
                    if render_state
                        .scrollbar_thumb_bounds
                        .expect("Thumb bound should exist")
                        .is_empty()
                    {
                        return false;
                    }

                    let previous_position = render_state
                        .scrollbar_thumb_bounds
                        .expect("Thumb bound should exist")
                        .center()
                        .along(current_axis);

                    self.jump_to_position(
                        previous_position.into_pixels(),
                        position.along(current_axis).into_pixels(),
                        scrollable_size,
                        current_axis,
                        ctx,
                        app,
                    );

                    // Dispatch an action in tests so we can perform assertions
                    // on clicks.
                    #[cfg(test)]
                    ctx.dispatch_action("scrollable_click::on_gutter", ());

                    true
                } else {
                    false
                }
            }
        }
    }

    fn mouse_dragged(
        &mut self,
        position: Vector2F,
        scrollable_size: Vector2F,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let (previous_position_along_axis, axis) = match self {
            Self::BothAxes { config, .. } => {
                // If we have not started a drag session, early return.
                let Some((previous_position, axis)) = config.drag_start() else {
                    return false;
                };

                // Update the drag start state of the scroll handle.
                config.set_drag_start(position, axis);
                (previous_position.into_pixels(), axis)
            }
            Self::SingleAxis { axis, config, .. } => {
                // If we have not started a drag session, early return.
                let Some(previous_position) = config.drag_start() else {
                    return false;
                };

                // Update the drag start state of the scroll handle.
                config.set_drag_start(position, *axis);
                (previous_position.into_pixels(), *axis)
            }
        };

        // Scroll to the new position along the axis of the active drag session.
        self.jump_to_position(
            previous_position_along_axis,
            position.along(axis).into_pixels(),
            scrollable_size,
            axis,
            ctx,
            app,
        );

        true
    }

    fn mouse_up(&self) -> bool {
        match self {
            Self::BothAxes { config, .. } => {
                // If we have not started a drag session, early return.
                let Some((_, axis)) = config.drag_start() else {
                    return false;
                };

                config.end_drag(axis);
            }
            Self::SingleAxis { config, .. } => {
                // If we have not started a drag session, early return.
                if config.drag_start().is_none() {
                    return false;
                }

                config.end_drag();
            }
        }
        true
    }

    fn mouse_moved(&self, position: Vector2F, is_covered: bool, ctx: &mut EventContext) -> bool {
        match self {
            Self::BothAxes {
                config,
                horizontal_state,
                vertical_state,
                ..
            } => {
                // If we are in a drag session, we don't need to update the scrollbar thumb hover state. Early return.
                if config.drag_start().is_some() {
                    return false;
                }

                let was_hovered_horizontal = config.hovered(Axis::Horizontal);
                let was_hovered_vertical = config.hovered(Axis::Vertical);
                let was_child_hovered = config.child_hovered();

                let mouse_in_horizontal_thumb = !is_covered
                    && horizontal_state
                        .scrollbar_thumb_bounds
                        .expect("Bounds should exist")
                        .contains_point(position);
                let mouse_in_vertical_thumb = !is_covered
                    && vertical_state
                        .scrollbar_thumb_bounds
                        .expect("Bounds should exist")
                        .contains_point(position);

                let mouse_in_child = !is_covered
                    && config
                        .child_bounds()
                        .expect("Bounds should exist")
                        .contains_point(position);

                let mut hover_state_changed = false;
                if mouse_in_horizontal_thumb != was_hovered_horizontal {
                    config.set_hovered(Axis::Horizontal, mouse_in_horizontal_thumb);
                    hover_state_changed = true;
                }

                if mouse_in_vertical_thumb != was_hovered_vertical {
                    config.set_hovered(Axis::Vertical, mouse_in_vertical_thumb);
                    hover_state_changed = true;
                }

                if mouse_in_child != was_child_hovered {
                    config.set_child_hovered(mouse_in_child);
                    hover_state_changed = true;
                }

                // Re-render if either the horizontal / vertical scrollbar state changed.
                if hover_state_changed {
                    ctx.notify();
                }

                mouse_in_horizontal_thumb || mouse_in_vertical_thumb
            }
            Self::SingleAxis {
                config,
                render_state,
                ..
            } => {
                if config.drag_start().is_some() {
                    return false;
                }

                let was_hovered = config.hovered();
                let was_child_hovered = config.child_hovered();
                let mouse_in = !is_covered
                    && render_state
                        .scrollbar_thumb_bounds
                        .expect("Bounds should exist")
                        .contains_point(position);

                let mouse_in_child = !is_covered
                    && config
                        .child_bounds()
                        .expect("Bounds should exist")
                        .contains_point(position);

                let mut hover_state_changed = false;
                if was_hovered != mouse_in {
                    config.set_hovered(mouse_in);
                    hover_state_changed = true;
                }

                if mouse_in_child != was_child_hovered {
                    config.set_child_hovered(mouse_in_child);
                    hover_state_changed = true;
                }

                if hover_state_changed {
                    ctx.notify();
                }

                mouse_in
            }
        }
    }

    fn mousewheel(
        &mut self,
        mut delta: Vector2F,
        precise: bool,
        scrollable_size: Vector2F,
        propagate_if_not_handled: bool,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let viewport_size = self.viewport_size(scrollable_size);
        match self {
            Self::BothAxes {
                config,
                horizontal_state,
                vertical_state,
                ..
            } => {
                delta = adjust_scroll_delta_with_sensitivity_config(
                    delta,
                    DUAL_AXES_SCROLL_SENSITIVITY,
                );

                // Set horizontal delta to 0 if it is not scrollable on that axis.
                if !config.should_handle_scroll_wheel(Axis::Horizontal)
                    || horizontal_state
                        .scrollbar_size_percentage
                        .expect("should be set at event dispatching time")
                        >= 1.
                {
                    delta = delta.project_onto(Axis::Vertical);
                }

                // Set vertical delta to 0 if it is not scrollable on that axis.
                if !config.should_handle_scroll_wheel(Axis::Vertical)
                    || vertical_state
                        .scrollbar_size_percentage
                        .expect("should be set at event dispatching time")
                        >= 1.
                {
                    delta = delta.project_onto(Axis::Horizontal);
                }

                if delta.is_zero() {
                    return false;
                }

                if !precise {
                    delta *= NUM_PIXELS_PER_LINE;
                }

                // If there would be no change from the scroll, don't handle it
                // so that other parent elements in the tree can handle it.
                if propagate_if_not_handled && !config.can_scroll_delta(viewport_size, delta, app) {
                    return false;
                }

                // Dispatch scroll event on each axis.
                config.scroll_to(
                    viewport_size,
                    delta.along(Axis::Horizontal).into_pixels(),
                    Axis::Horizontal,
                    ctx,
                );
                config.scroll_to(
                    viewport_size,
                    delta.along(Axis::Vertical).into_pixels(),
                    Axis::Vertical,
                    ctx,
                );
            }
            Self::SingleAxis {
                axis,
                config,
                render_state,
                ..
            } => {
                if !config.should_handle_scroll_wheel(*axis) {
                    return false;
                }

                if render_state
                    .scrollbar_size_percentage
                    .expect("should be set at event dispatching time")
                    >= 1.
                {
                    return !propagate_if_not_handled;
                }

                if !precise {
                    delta *= NUM_PIXELS_PER_LINE;
                }

                // If there would be no change from the scroll, don't handle it
                // so that other parent elements in the tree can handle it.
                if propagate_if_not_handled
                    && !config.can_scroll_delta(*axis, viewport_size, delta, app)
                {
                    return false;
                }

                config.scroll_to(viewport_size, delta.along(*axis).into_pixels(), *axis, ctx);
            }
        }
        true
    }

    /// Scroll the child element to match the delta scrolled from the previous to current scrollbar thumb position.
    fn jump_to_position(
        &mut self,
        previous_position_along_axis: Pixels,
        new_position_along_axis: Pixels,
        scrollable_size: Vector2F,
        axis: Axis,
        ctx: &mut EventContext,
        app: &AppContext,
    ) {
        let viewport_size = self.viewport_size(scrollable_size);
        let data = match self {
            Self::BothAxes { config, .. } => config.scroll_data(viewport_size, axis, app),
            Self::SingleAxis {
                config,
                axis: scroll_axis,
                ..
            } if *scroll_axis == axis => config.scroll_data(axis, viewport_size, app),
            Self::SingleAxis { .. } => {
                log::warn!("Trying to jump to position on a non-scrollable axis");
                return;
            }
        };

        let total_size = data.total_size;
        let scroll_start = data.scroll_start;
        let scroll_remaining = data.total_size - data.scroll_start - data.visible_px;

        // We need to use the original scrollbar size before resizing to calculate the scroll speed.
        let scrollbar_size_percentage_before_resize = data.visible_px / total_size;

        // We don't want to update the scroll position if you're scrolled to the top and the cursor is above
        // the element or if you're scrolled to the bottom and the cursor is below the element.
        if (scroll_remaining <= Pixels::zero()
            && new_position_along_axis > previous_position_along_axis)
            || (scroll_start <= Pixels::zero()
                && previous_position_along_axis > new_position_along_axis)
        {
            return;
        }

        let delta = previous_position_along_axis - new_position_along_axis;
        let adjusted_delta = delta / scrollbar_size_percentage_before_resize;

        match self {
            Self::BothAxes { config, .. } => {
                config.scroll_to(viewport_size, adjusted_delta, axis, ctx)
            }
            Self::SingleAxis { config, .. } => {
                config.scroll_to(viewport_size, adjusted_delta, axis, ctx)
            }
        }
    }
}

/// Keep track of the render state of a single scrollbar.
/// A scrollable could have multiple scrollbars when it is scrollable on both axes.
struct ScrollbarRenderState {
    /// The bounds of the whole scrollbar gutter.
    scrollbar_track_bounds: Option<RectF>,
    /// The relative position of the thumb within the scrollbar.
    scrollbar_position_percentage: Option<f32>,
    /// The relative height of the thumb compared to the whole scrollbar.
    scrollbar_size_percentage: Option<f32>,
    /// The bounds for the scrollbar thumb.
    scrollbar_thumb_bounds: Option<RectF>,
    /// The origin for the scrollbar thumb.
    scrollbar_thumb_origin: Option<Vector2F>,
    /// Padding between child element and the scrollbar.
    padding_between_child_and_scrollbar: f32,
    /// Padding after the scrollbar.
    padding_after_scrollbar: f32,
}

impl ScrollbarRenderState {
    fn new() -> Self {
        Self {
            scrollbar_track_bounds: None,
            scrollbar_position_percentage: None,
            scrollbar_size_percentage: None,
            scrollbar_thumb_bounds: None,
            scrollbar_thumb_origin: None,
            padding_between_child_and_scrollbar: LEFT_PADDING,
            padding_after_scrollbar: RIGHT_PADDING,
        }
    }

    /// The additional spacing the scrollbar will take on the cross axis of the scrollable.
    fn cross_axis_spacing(&self) -> f32 {
        self.padding_between_child_and_scrollbar + self.padding_after_scrollbar
    }

    /// Update the render state with the latest scroll data.
    fn update_with_scroll_data(&mut self, scroll_data: ScrollData, scrollable_pixels: f32) {
        // If total_size is zero (e.g., empty content), there's nothing to scroll.
        // Set size_percentage to 1.0 so no scrollbar thumb renders.
        if scroll_data.total_size <= Pixels::zero() {
            self.scrollbar_size_percentage = Some(1.0);
            self.scrollbar_position_percentage = Some(0.0);
            return;
        }

        let total_size = scroll_data.total_size;

        // Calculate the size percentage to render the scrollbar thumb.
        let minimum_size_percentage = (MINIMUM_HEIGHT / scrollable_pixels).min(1.);
        let size_percentage =
            (scroll_data.visible_px / total_size).max(minimum_size_percentage.into_pixels());

        self.scrollbar_size_percentage = Some(size_percentage.as_f32());

        // The scrollbar position is calculated with the ratio between scroll top and scroll bottom.
        let scroll_start = scroll_data.scroll_start;
        let scroll_remaining = total_size - scroll_data.scroll_start - scroll_data.visible_px;

        let scrollbar_position_percentage = scroll_start / (scroll_start + scroll_remaining);
        self.scrollbar_position_percentage = Some(scrollbar_position_percentage.as_f32());
    }

    /// Draw the scrollbar based on the current render state.
    #[allow(clippy::too_many_arguments)]
    fn draw_scrollbar(
        &mut self,
        axis: Axis,
        scrollable_size: Vector2F,
        scrollable_size_with_padding: Vector2F,
        origin: Vector2F,
        scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
        appearance: ScrollableAppearance,
        ctx: &mut PaintContext,
    ) {
        // The size of scrollbar track length is just the offset of the scrollbars projected to the cross axis.
        let scrollbar_track_length = scrollable_size_with_padding.along(axis.invert());
        let viewport_size = (scrollable_size - scrollable_size_with_padding).max(Vector2F::zero());
        let scrollbar_track_origin = origin + scrollable_size.project_onto(axis.invert())
            - scrollbar_track_length.along(axis.invert());
        let scrollbar_track_size = scrollbar_size(axis, viewport_size, scrollbar_track_length);

        let scrollbar_track_bounds = RectF::new(scrollbar_track_origin, scrollbar_track_size);
        self.scrollbar_track_bounds = Some(scrollbar_track_bounds);

        let scrollbar_size_percentage = self.scrollbar_size_percentage.unwrap();
        let scrollbar_position_percentage = self.scrollbar_position_percentage.unwrap();

        // If the scrollbar is overlaid over the child, it should be rendered at a higher z-index.
        // However, this doesn't apply if the scrollbar is non-functional (i.e. there's no thumb),
        // as we wouldn't want a dummy scrollbar to block events dispatched to underlying children
        // at a lower z-index (i.e. for editor text selection).
        let render_scrollbar_thumb = scrollbar_size_percentage < 1.;
        let render_at_higher_z_index = appearance.overlaid_scrollbar && render_scrollbar_thumb;

        if render_at_higher_z_index {
            ctx.scene
                .start_layer(ClipBounds::BoundedByActiveLayerAnd(scrollbar_track_bounds));
        }

        let scrollbar = ctx
            .scene
            .draw_rect_with_hit_recording(scrollbar_track_bounds);

        // If the scrollbar is overlaid, make it transparent.
        if appearance.overlaid_scrollbar {
            scrollbar.with_background(Fill::Solid(ColorU::transparent_black()));
        } else {
            scrollbar.with_background(scrollbar_track_background);
        }

        if render_scrollbar_thumb {
            let scrollbar_thumb_size = scrollbar_size(
                axis,
                viewport_size * scrollbar_size_percentage,
                appearance.scrollbar_size.as_f32(),
            );
            let scrollbar_thumb_origin = scrollbar_track_origin
                + scrollbar_size(
                    axis,
                    (viewport_size - scrollbar_thumb_size).max(Vector2F::zero())
                        * scrollbar_position_percentage,
                    self.padding_between_child_and_scrollbar,
                );

            self.scrollbar_thumb_bounds =
                Some(RectF::new(scrollbar_thumb_origin, scrollbar_thumb_size));
            self.scrollbar_thumb_origin = Some(scrollbar_thumb_origin);

            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    scrollbar_thumb_origin,
                    scrollbar_thumb_size,
                ))
                .with_background(scrollbar_thumb_background)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));
        } else {
            self.scrollbar_thumb_origin = Some(origin);
            self.scrollbar_thumb_bounds = Some(RectF::new(vec2f(0., 0.), vec2f(0., 0.)));
        }

        // See comment above about the layering of the scrollbar.
        if render_at_higher_z_index {
            ctx.scene.stop_layer();
        }
    }

    fn scrollbar_padding(
        &self,
        appearance: ScrollableAppearance,
        include_overlaid_scrollbar: bool,
    ) -> f32 {
        if !include_overlaid_scrollbar && appearance.overlaid_scrollbar {
            0.
        } else {
            self.cross_axis_spacing()
        }
    }
}

/// A wrapper element that makes the underlying child scrollable within a visible
/// viewport.
pub struct NewScrollable {
    state: ScrollableState,
    scrollable_size: Option<Vector2F>,
    origin: Option<Point>,

    /// The color of the scrollbar thumb when not hovered/active.
    nonactive_scrollbar_thumb_background: Fill,
    /// The color of the scrollbar thumb when hovered/active.
    active_scrollbar_thumb_background: Fill,
    /// The color of the scrollbar track.
    scrollbar_track_background: Fill,

    // This is a short-term solution for properly handling events on stacks. A stack will always
    // put its children on higher z-indexes than its origin, so a hit test using the standard
    // `z_index` method would always result in the event being covered (by the children of the
    // stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    // Then we use that upper bound to do the hit testing, which means a parent will always get
    // events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,

    // If true, propagate mousewheel events to the parent if the scrollable is scrolled to the edge
    // and a scrollwheel event would scroll further in the direction of the edge.
    // This is useful for nested scrollables where the inner scrollable is at the edge and the outer
    // scrollable should scroll instead.
    propagate_mousewheel_if_not_handled: bool,

    // If true, always handle scroll wheel events even if the scrollable is not scrolled to the edge.
    always_handle_events_first: bool,

    // If true, remap a single-axis scrollable's cross-axis wheel deltas
    // onto its main axis before dispatching them. Opt-in.
    remap_cross_axis_wheel_to_main_axis: bool,
}

impl NewScrollable {
    /// Internal method for creating a scrollable with an initial scrollable state.
    fn new_internal(
        state: ScrollableState,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
        always_handle_events_first: bool,
    ) -> Self {
        Self {
            state,
            scrollable_size: None,
            origin: None,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
            child_max_z_index: None,
            propagate_mousewheel_if_not_handled: false,
            always_handle_events_first,
            remap_cross_axis_wheel_to_main_axis: false,
        }
    }

    /// Thin wrapper that forwards to the underlying clipped handle(s) on each scrollable axis.
    ///
    /// Like the handle method, this mutates the scroll anchor(s) as a side effect; see
    /// [`ClippedScrollStateHandle::anchor_and_adjust_selection_for_scroll`] for the exact
    /// semantics.
    fn anchor_and_adjust_selection_for_scroll(
        &self,
        current_selection: Option<super::Selection>,
    ) -> Option<super::Selection> {
        match &self.state {
            ScrollableState::SingleAxis { axis, config, .. } => match config {
                SingleAxisConfig::Clipped { handle, .. } => {
                    handle.anchor_and_adjust_selection_for_scroll(current_selection, *axis)
                }
                SingleAxisConfig::Manual { .. } => current_selection,
            },
            ScrollableState::BothAxes { config, .. } => match config {
                DualAxisConfig::Clipped {
                    horizontal,
                    vertical,
                    ..
                } => {
                    let selection = horizontal.handle.anchor_and_adjust_selection_for_scroll(
                        current_selection,
                        Axis::Horizontal,
                    );
                    vertical
                        .handle
                        .anchor_and_adjust_selection_for_scroll(selection, Axis::Vertical)
                }
                DualAxisConfig::Manual { .. } => current_selection,
            },
        }
    }

    fn clear_selection_scroll_anchor(&self) {
        match &self.state {
            ScrollableState::SingleAxis { config, .. } => {
                if let SingleAxisConfig::Clipped { handle, .. } = config {
                    handle.clear_selection_scroll_anchor();
                }
            }
            ScrollableState::BothAxes { config, .. } => {
                if let DualAxisConfig::Clipped {
                    horizontal,
                    vertical,
                    ..
                } = config
                {
                    horizontal.handle.clear_selection_scroll_anchor();
                    vertical.handle.clear_selection_scroll_anchor();
                }
            }
        }
    }

    /// Create a scroll element that is only scrollable on the vertical axis.
    pub fn vertical(
        config: SingleAxisConfig,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Self {
        config.validate(Axis::Vertical);
        let state = ScrollableState::SingleAxis {
            axis: Axis::Vertical,
            appearance: Default::default(),
            config,
            render_state: ScrollbarRenderState::new(),
        };
        Self::new_internal(
            state,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
            true,
        )
    }

    /// Create a scroll element that is only scrollable on the horizontal axis.
    pub fn horizontal(
        config: SingleAxisConfig,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Self {
        config.validate(Axis::Horizontal);
        let state = ScrollableState::SingleAxis {
            axis: Axis::Horizontal,
            appearance: Default::default(),
            config,
            render_state: ScrollbarRenderState::new(),
        };
        Self::new_internal(
            state,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
            true,
        )
    }

    /// Create a scroll element that is scrollable on both axes.
    pub fn horizontal_and_vertical(
        config: DualAxisConfig,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Self {
        config.validate();
        let state = ScrollableState::BothAxes {
            config,
            horizontal_appearance: Default::default(),
            vertical_appearance: Default::default(),
            horizontal_state: ScrollbarRenderState::new(),
            vertical_state: ScrollbarRenderState::new(),
        };
        Self::new_internal(
            state,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
            true,
        )
    }

    /// Override the default appearance for the vertical scrollbar. This will be a no-op (panic on local build)
    /// if the scrollable has no vertical scrollbar.
    pub fn with_vertical_scrollbar(mut self, new_appearance: ScrollableAppearance) -> Self {
        match &mut self.state {
            ScrollableState::BothAxes {
                vertical_appearance,
                ..
            } => *vertical_appearance = new_appearance,
            ScrollableState::SingleAxis {
                axis, appearance, ..
            } => {
                if matches!(axis, Axis::Horizontal) {
                    if cfg!(debug_assertions) {
                        panic!(
                            "Trying to apply vertical scrollbar appearance on a horizontal scrollable"
                        );
                    } else {
                        return self;
                    }
                }
                *appearance = new_appearance;
            }
        }

        self
    }

    /// Override the default appearance for the horizontal scrollbar. This will be a no-op (panic on local build)
    /// if the scrollable has no horizontal scrollbar.
    pub fn with_horizontal_scrollbar(mut self, new_appearance: ScrollableAppearance) -> Self {
        match &mut self.state {
            ScrollableState::BothAxes {
                horizontal_appearance,
                ..
            } => *horizontal_appearance = new_appearance,
            ScrollableState::SingleAxis {
                axis, appearance, ..
            } => {
                if matches!(axis, Axis::Vertical) {
                    if cfg!(debug_assertions) {
                        panic!(
                            "Trying to apply horizontal scrollbar appearance on a vertical scrollable"
                        );
                    } else {
                        return self;
                    }
                }
                *appearance = new_appearance;
            }
        }

        self
    }

    pub fn with_propagate_mousewheel_if_not_handled(mut self, propagate: bool) -> Self {
        self.propagate_mousewheel_if_not_handled = propagate;
        self
    }

    pub fn with_always_handle_events_first(mut self, always_handle_events_first: bool) -> Self {
        self.always_handle_events_first = always_handle_events_first;
        self
    }

    /// On a single-axis scrollable, remap a scroll-wheel event whose delta
    /// lies only on the cross axis onto the scrollable's own axis. Lets a
    /// vertical mouse wheel drive a horizontal-only scrollable (and vice
    /// versa) without a modifier or horizontal trackpad swipe. No-op for
    /// dual-axis scrollables.
    pub fn with_remap_cross_axis_wheel_to_main_axis(mut self, remap: bool) -> Self {
        self.remap_cross_axis_wheel_to_main_axis = remap;
        self
    }

    fn handle_event(
        &mut self,
        z_index: ZIndex,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        match event.raw_event() {
            Event::LeftMouseDown { position, .. } => {
                if ctx.is_covered(Point::from_vec2f(*position, z_index)) {
                    false
                } else {
                    self.state.mouse_down(
                        *position,
                        self.scrollable_size.expect("Size should exist"),
                        ctx,
                        app,
                    )
                }
            }
            Event::LeftMouseUp { .. } => self.state.mouse_up(),
            Event::LeftMouseDragged { position, .. } => self.state.mouse_dragged(
                *position,
                self.scrollable_size.expect("Size should exist"),
                ctx,
                app,
            ),
            Event::MouseMoved { position, .. } => {
                let is_covered = ctx.is_covered(Point::from_vec2f(*position, z_index));
                self.state.mouse_moved(*position, is_covered, ctx)
            }
            Event::ScrollWheel {
                delta,
                precise,
                position,
                modifiers: ModifiersState { ctrl: false, .. },
            } => {
                let is_covered = ctx.is_covered(Point::from_vec2f(*position, z_index));
                let in_bound = self
                    .origin
                    .zip(self.scrollable_size)
                    .and_then(|(origin, size)| ctx.visible_rect(origin, size))
                    .map(|visible| visible.contains_point(*position))
                    .unwrap_or(false);

                if !in_bound || is_covered {
                    false
                } else {
                    // When opted in on a single-axis scrollable, swap a
                    // pure cross-axis wheel delta onto the main axis so
                    // the existing mousewheel path can consume it. Exact
                    // `== 0.0` is intentional: discrete mouse wheels emit
                    // 0 on the cross axis, trackpads always carry a tiny
                    // drift on both axes and shouldn't be remapped.
                    let delta = if self.remap_cross_axis_wheel_to_main_axis {
                        if let ScrollableState::SingleAxis { axis, .. } = &self.state {
                            let main = delta.along(*axis);
                            let cross = delta.along(axis.invert());
                            if main == 0.0 && cross != 0.0 {
                                cross.along(*axis)
                            } else {
                                *delta
                            }
                        } else {
                            *delta
                        }
                    } else {
                        *delta
                    };
                    self.state.mousewheel(
                        delta,
                        *precise,
                        self.scrollable_size.expect("Size should exist"),
                        self.propagate_mousewheel_if_not_handled,
                        ctx,
                        app,
                    )
                }
            }
            _ => false,
        }
    }
}

impl Element for NewScrollable {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = self.state.layout_child(constraint, ctx, app);
        self.scrollable_size = Some(size);
        size
    }

    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        Some(self as &dyn SelectableElement)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        let scrollable_size = match self.scrollable_size {
            Some(size) => size,
            None => {
                log::warn!("Calling after_layout on NewScrollable without laying out the element");
                return;
            }
        };
        self.state.after_layout(scrollable_size, ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let size = match self.scrollable_size {
            Some(size) => size,
            None => {
                log::warn!("Calling paint on NewScrollable without laying out the element");
                return;
            }
        };
        // Technically, we only need to start a new layer if one of the axes is clipped.
        // For simplicity, always start the layer with scrollable bound. This will be just
        // no-op for the case when both axes are managed manually.
        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(RectF::new(
                origin, size,
            )));

        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let original_selection = ctx.current_selection;
        ctx.current_selection = self.anchor_and_adjust_selection_for_scroll(original_selection);
        self.state.paint_child(origin, size, ctx, app);
        ctx.current_selection = original_selection;

        self.state.draw_scrollbars(
            origin,
            size,
            self.nonactive_scrollbar_thumb_background,
            self.active_scrollbar_thumb_background,
            self.scrollbar_track_background,
            ctx,
        );
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
        ctx.scene.stop_layer();
    }

    fn size(&self) -> Option<Vector2F> {
        self.scrollable_size
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
        let Some(z_index) = self.child_max_z_index else {
            log::warn!("Tried to handle event in scrollable before the element is painted");
            return false;
        };

        if self.always_handle_events_first {
            // Different from other elements, scrollable always tries to handle the event first. It only
            // dispatches event to its child if the event is not handled by scrollable. This ensures we
            // never have additional events firing together with scrolling. Because of this requirement,
            // scrollable should strictly only handle events if either:
            // 1. There is an active scrolling session.
            // 2. The mouse event happens exactly on the scrollbar track and is not covered.
            let handled_by_scrollbar = self.handle_event(z_index, event, ctx, app);

            if !handled_by_scrollbar {
                if matches!(event.raw_event(), Event::LeftMouseDown { .. }) {
                    self.clear_selection_scroll_anchor();
                }
                self.state.dispatch_event_to_child(event, ctx, app)
            } else {
                true
            }
        } else {
            let handled_by_child = self.state.dispatch_event_to_child(event, ctx, app);
            if !handled_by_child {
                self.handle_event(z_index, event, ctx, app)
            } else {
                true
            }
        }
    }
}

impl SelectableElement for NewScrollable {
    fn get_selection(
        &self,
        selection_start: Vector2F,
        selection_end: Vector2F,
        is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>> {
        let selection = self.anchor_and_adjust_selection_for_scroll(Some(super::Selection {
            start: selection_start,
            end: selection_end,
            is_rect,
        }))?;
        self.state
            .child_as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.get_selection(selection.start, selection.end, selection.is_rect)
            })
    }

    fn expand_selection(
        &self,
        point: Vector2F,
        direction: SelectionDirection,
        unit: SelectionType,
        word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F> {
        self.state
            .child_as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.expand_selection(point, direction, unit, word_boundaries_policy)
            })
    }

    fn is_point_semantically_before(
        &self,
        absolute_point: Vector2F,
        absolute_point_other: Vector2F,
    ) -> Option<bool> {
        self.state
            .child_as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.is_point_semantically_before(absolute_point, absolute_point_other)
            })
    }

    fn smart_select(
        &self,
        absolute_point: Vector2F,
        smart_select_fn: crate::elements::SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)> {
        self.state
            .child_as_selectable_element()
            .and_then(|selectable_child| {
                selectable_child.smart_select(absolute_point, smart_select_fn)
            })
    }

    fn calculate_clickable_bounds(
        &self,
        current_selection: Option<super::Selection>,
    ) -> Vec<RectF> {
        self.state
            .child_as_selectable_element()
            .map(|selectable_child| {
                selectable_child.calculate_clickable_bounds(
                    self.anchor_and_adjust_selection_for_scroll(current_selection),
                )
            })
            .unwrap_or_default()
    }
}

impl ClippedScrollStateHandle {
    fn scroll_data(&self, viewport_size: Vector2F, child_size: Vector2F, axis: Axis) -> ScrollData {
        ScrollData {
            scroll_start: self.scroll_start(),
            visible_px: (viewport_size.along(axis)).into_pixels(),
            total_size: child_size.along(axis).into_pixels(),
        }
    }
}

#[cfg(test)]
#[path = "scrollable_tests.rs"]
mod tests;
