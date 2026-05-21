use super::{
    AfterLayoutContext, AppContext, Axis, Element, Event, EventContext, Fill, LayoutContext,
    PaintContext, Point, SizeConstraint, Vector2FExt, ZIndex,
};
use crate::elements::F32Ext;
use crate::event::ModifiersState;
pub use crate::scene::CornerRadius;
use crate::units::{IntoPixels, Pixels};
use crate::ClipBounds;
use crate::{event::DispatchedEvent, scene::Radius};

use pathfinder_color::ColorU;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

pub const LEFT_PADDING: f32 = 2.;
const RIGHT_PADDING: f32 = 2.;
const MINIMUM_HEIGHT: f32 = 20.;

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
const NUM_PIXELS_PER_LINE: Pixels = Pixels::new(40.);

#[derive(Clone, Default)]
pub struct ScrollState {
    pub started: Option<f32>,
    pub hovered: bool,
    pub child_hovered: bool,
}

pub type ScrollStateHandle = Arc<Mutex<ScrollState>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollData {
    /// The number of pixels that the child element has been scrolled from its start.
    /// For a vertically scrollable element, this is equivalent to
    /// [`scrollTop`](https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollTop).
    /// For a horizontally scrollable element, this is equivalent to
    /// [`scrollLeft`](https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollLeft).
    pub scroll_start: Pixels,

    /// The number of pixels of the child element that are visible in the currently scrolled region.
    pub visible_px: Pixels,

    /// The size of the scrollable element's content.
    /// This is not necessarily the child element's size (e.g. if the child is viewported).
    /// For a vertically scrollable element, this is equivalent to
    /// [`scrollHeight`](https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollHeight).
    /// For a horizontally scrollable element, this is equivalent to
    /// [`scrollWidth`](https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollWidth).
    pub total_size: Pixels,
}

pub trait ScrollableElement: Element {
    /// Returns scrolling data that the child computes and that the [`Scrollable`]
    /// uses to update its internal state. If the child is scrollable
    /// (i.e. the child has been laid out), this must be [`Some`].
    fn scroll_data(&self, app: &AppContext) -> Option<ScrollData>;

    /// Scrolls the element by the given `delta` (in pixels).
    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext);

    /// By default, scrollable elements are responsible for their own wheel handling.
    /// Override to return true if you want the parent scrollable to handle the wheel.
    fn should_handle_scroll_wheel(&self) -> bool {
        false
    }

    fn finish_scrollable(self) -> Box<dyn ScrollableElement>
    where
        Self: 'static + Sized,
    {
        Box::new(self)
    }
}

/// An enum inspired by scrollbar-width css property.
/// It includes 2 basic sizes.
///
/// See [mdn](https://developer.mozilla.org/en-US/docs/Web/CSS/scrollbar-width).
///
/// # Examples
/// ```
/// use warpui_core::elements::ScrollbarWidth;
///
/// // Default width of 8.
/// let y = ScrollbarWidth::Auto;
///
/// // Width of 0. to make the scrollbar invisible
/// let z = ScrollbarWidth::None;
/// ```
#[derive(Default, Clone, Copy, Debug)]
pub enum ScrollbarWidth {
    #[default]
    Auto,
    None,
    Custom(f32),
}

impl ScrollbarWidth {
    pub const fn as_f32(&self) -> f32 {
        match *self {
            ScrollbarWidth::Auto => 8.,
            ScrollbarWidth::None => 0.,
            ScrollbarWidth::Custom(width) => width,
        }
    }
}

/// A generic element to handle scrolling of an underlying element.
/// Delegates to the underlying child element to update child-specific
/// scrolling parameters.
///
/// Supports both vertical and horizontal scrolling via the [`Scrollable::vertical`]
/// and [`Scrollable::horizontal`] APIs, respectively.
pub struct Scrollable {
    axis: Axis,
    child: Box<dyn ScrollableElement>,
    state: ScrollStateHandle,
    origin: Option<Point>,

    /// The size of the [`Scrollable`], as determined during layout.
    scrollable_size: Option<Vector2F>,

    /// The color of the scrollbar thumb when not hovered/active.
    nonactive_scrollbar_thumb_background: Fill,
    /// The color of the scrollbar thumb when hovered/active.
    active_scrollbar_thumb_background: Fill,
    /// The color of the scrollbar track.
    scrollbar_track_background: Fill,

    /// The size of the scrollbar in pixels.
    scrollbar_size: ScrollbarWidth,
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

    // This is a short-term solution for properly handling events on stacks. A stack will always
    // put its children on higher z-indexes than its origin, so a hit test using the standard
    // `z_index` method would always result in the event being covered (by the children of the
    // stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    // Then we use that upper bound to do the hit testing, which means a parent will always get
    // events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,

    // The scrollbar is the runway for the draggable scrollbar. By default the scollbox renders to
    // the side of the child element. This setting makes the scrollbar render over the child instead.
    overlayed_scrollbar: bool,
}

impl Scrollable {
    #[allow(clippy::too_many_arguments)]
    fn new(
        axis: Axis,
        state: ScrollStateHandle,
        child: Box<dyn ScrollableElement>,
        scrollbar_size: ScrollbarWidth,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Self {
        Self {
            axis,
            child,
            scrollbar_size,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
            state,
            origin: None,
            scrollable_size: None,
            scrollbar_track_bounds: None,
            scrollbar_position_percentage: None,
            scrollbar_size_percentage: None,
            scrollbar_thumb_bounds: None,
            scrollbar_thumb_origin: None,
            padding_between_child_and_scrollbar: LEFT_PADDING,
            padding_after_scrollbar: RIGHT_PADDING,
            child_max_z_index: None,
            overlayed_scrollbar: false,
        }
    }

    /// Creates a vertically scrollable element.
    #[allow(clippy::too_many_arguments)]
    pub fn vertical(
        state: ScrollStateHandle,
        child: Box<dyn ScrollableElement>,
        scrollbar_size: ScrollbarWidth,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Self {
        Self::new(
            Axis::Vertical,
            state,
            child,
            scrollbar_size,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
        )
    }

    /// Creates a horizontally scrollable element.
    #[allow(clippy::too_many_arguments)]
    pub fn horizontal(
        state: ScrollStateHandle,
        child: Box<dyn ScrollableElement>,
        scrollbar_size: ScrollbarWidth,
        nonactive_scrollbar_thumb_background: Fill,
        active_scrollbar_thumb_background: Fill,
        scrollbar_track_background: Fill,
    ) -> Self {
        Self::new(
            Axis::Horizontal,
            state,
            child,
            scrollbar_size,
            nonactive_scrollbar_thumb_background,
            active_scrollbar_thumb_background,
            scrollbar_track_background,
        )
    }

    /// Sets the padding between the child element and the scrollbar.
    pub fn with_padding_start(mut self, padding_start: f32) -> Self {
        self.padding_between_child_and_scrollbar = padding_start;
        self
    }

    /// Sets the padding after the scrollbar.
    pub fn with_padding_end(mut self, padding_end: f32) -> Self {
        self.padding_after_scrollbar = padding_end;
        self
    }

    pub fn with_overlayed_scrollbar(mut self) -> Self {
        self.overlayed_scrollbar = true;
        self
    }

    fn state(&mut self) -> MutexGuard<'_, ScrollState> {
        self.state.lock().unwrap()
    }

    fn mouse_dragged(&mut self, position: Vector2F, ctx: &mut EventContext, app: &AppContext) {
        let previous_dragging_position = self.state().started;
        if let Some(previous_dragging_position) = previous_dragging_position {
            let position_along_axis = position.along(self.axis);
            self.start_scrolling(position);
            self.jump_to_position(
                previous_dragging_position.into_pixels(),
                position_along_axis.into_pixels(),
                ctx,
                app,
            );
        }
    }

    fn jump_to_position(
        &mut self,
        previous_position_along_axis: Pixels,
        new_position_along_axis: Pixels,
        ctx: &mut EventContext,
        app: &AppContext,
    ) {
        let total_size = self.total_size(app);
        let scroll_start = self.scroll_start(app);
        let scroll_remaining = self.scroll_remaining(app);

        // We need to use the original scrollbar size before resizing to calculate the scroll speed.
        let scrollbar_size_percentage_before_resize =
            (total_size - scroll_start - scroll_remaining) / total_size;

        // We don't want to update the scroll position if you're scrolled to the top and the cursor is above
        // the element or if you're scrolled to the bottom and the cursor is below the element.
        // TODO(kevin): Do we need the scroll_start <= 0 check?
        if (scroll_remaining <= Pixels::zero()
            && new_position_along_axis > previous_position_along_axis)
            || (scroll_start <= Pixels::zero()
                && previous_position_along_axis > new_position_along_axis)
        {
            return;
        }

        let delta = previous_position_along_axis - new_position_along_axis;

        // The scroll speed should be proportional to the total number of lines.
        // Assume we have moved the scrollbar by a distance x, the number of lines scrolled
        // should be calculated by x / total_height * total_number_of_lines.
        self.child
            .scroll(delta / scrollbar_size_percentage_before_resize, ctx);
    }

    fn mousewheel(&mut self, delta: Vector2F, precise: bool, ctx: &mut EventContext) {
        if self
            .scrollbar_size_percentage
            .expect("should be set at event dispatching time")
            < 1.
        {
            let delta_along_axis = delta.along(self.axis);
            if precise {
                self.child.scroll(delta_along_axis.into_pixels(), ctx);
            } else {
                // If the scroll was not `precise`, we need to convert the delta (which is
                // actually in terms of `Lines`) to the right number of `Pixels`.
                // See the comment on [`SCROLLBAR_PIXELS_PER_COCOA_TICK`] for more details.
                self.child.scroll(
                    (delta_along_axis * NUM_PIXELS_PER_LINE.as_f32()).into_pixels(),
                    ctx,
                );
            }
        }
    }

    /// Returns the child's [`ScrollData`], assuming the child has been laid out.
    fn scroll_data(&self, app: &AppContext) -> ScrollData {
        self.child
            .scroll_data(app)
            .expect("ScrollData should be some to be scrollable")
    }

    fn scroll_start(&self, app: &AppContext) -> Pixels {
        self.scroll_data(app).scroll_start
    }

    /// The number of pixels that the child is still scrollable (biased towards its end).
    /// For example, for a vertically scrollable element, this would be the number of pixels
    /// that the child can still be scrolled down.
    fn scroll_remaining(&self, app: &AppContext) -> Pixels {
        let scroll_data = self.scroll_data(app);
        scroll_data.total_size - scroll_data.scroll_start - scroll_data.visible_px
    }

    fn total_size(&self, app: &AppContext) -> Pixels {
        self.scroll_data(app).total_size
    }

    fn start_scrolling(&mut self, position: Vector2F) {
        self.state().started = Some(position.along(self.axis));
    }

    fn end_scrolling(&mut self) {
        self.state().started = None
    }

    /// Returns the `original_size` that has its inverted axis dimension changed to `dimension_along_inverted_axis`.
    fn size_along_inverted_axis(
        &self,
        original_size: Vector2F,
        dimension_along_inverted_axis: f32,
    ) -> Vector2F {
        match self.axis {
            Axis::Horizontal => vec2f(original_size.x(), dimension_along_inverted_axis),
            Axis::Vertical => vec2f(dimension_along_inverted_axis, original_size.y()),
        }
    }
}

impl Element for Scrollable {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let scrollbar_size = self.scrollbar_size.as_f32().along(self.axis.invert());
        let padding = (self.padding_between_child_and_scrollbar + self.padding_after_scrollbar)
            .along(self.axis.invert());

        let child_constraint = if self.overlayed_scrollbar {
            // If the scrollbar is overlayed, the child can span the entire constraint.
            SizeConstraint {
                min: constraint.min.max(Vector2F::zero()),
                max: constraint.max.max(Vector2F::zero()),
            }
        } else {
            // If the scrollbar is not overlayed, we must save room for the scrollbar.
            SizeConstraint {
                min: (constraint.min - scrollbar_size - padding).max(Vector2F::zero()),
                max: (constraint.max - scrollbar_size - padding).max(Vector2F::zero()),
            }
        };

        let child_size = self.child.layout(child_constraint, ctx, app);
        debug_assert!(
            child_size.y().is_finite(),
            "Scrollable's child should not have infinite height"
        );
        debug_assert!(
            child_size.x().is_finite(),
            "Scrollable's child should not have infinite width"
        );

        // If the scrollbar is not overlayed, we add back its size to get the overall size
        // of the scrollable element.
        let size = if self.overlayed_scrollbar {
            child_size
        } else {
            child_size + scrollbar_size + padding
        };

        self.scrollable_size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);

        let scroll_data = self.scroll_data(app);
        let total_size = scroll_data.total_size;

        let minimum_size_percentage =
            (MINIMUM_HEIGHT / self.scrollable_size.unwrap().along(self.axis)).min(1.);
        let size_percentage =
            (scroll_data.visible_px / total_size).max(minimum_size_percentage.into_pixels());

        self.scrollbar_size_percentage = Some(size_percentage.as_f32());

        // The scrollbar position is calculated with the ratio between scroll top and scroll bottom.
        let scroll_start = self.scroll_start(app);
        let scroll_remaining = self.scroll_remaining(app);

        let scrollbar_position_percentage = scroll_start / (scroll_start + scroll_remaining);
        self.scrollbar_position_percentage = Some(scrollbar_position_percentage.as_f32());
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.child.paint(origin, ctx, app);
        let scrollable_size = self
            .scrollable_size
            .expect("size should have been set during layout");

        // The origin of the scrollbar track is the maximum coordinate (along the inverted axis)
        // subtracted by the size of the scrollbar. For example, for a vertically scrollable element,
        // the origin will be the maximum x coordinate subtracted by the size of the scrollbar.
        let scrollbar_track_length = self.scrollbar_size.as_f32()
            + self.padding_between_child_and_scrollbar
            + self.padding_after_scrollbar;
        let scrollbar_track_origin = origin + scrollable_size.project_onto(self.axis.invert())
            - scrollbar_track_length.along(self.axis.invert());
        let scrollbar_track_size =
            self.size_along_inverted_axis(scrollable_size, scrollbar_track_length);

        let scrollbar_track_bounds = RectF::new(scrollbar_track_origin, scrollbar_track_size);
        self.scrollbar_track_bounds = Some(scrollbar_track_bounds);

        // If the scrollbar is overlayed over the child, it should be at a higher z-index.
        if self.overlayed_scrollbar {
            ctx.scene
                .start_layer(ClipBounds::BoundedBy(scrollbar_track_bounds));
        }
        let scrollbar = ctx
            .scene
            .draw_rect_with_hit_recording(scrollbar_track_bounds);

        // If the scrollbar is overlayed, make it transparent. If neither the scrollbar nor the child
        // is hovered, make it have no fill.
        if !self.state().hovered && !self.state().child_hovered {
            scrollbar.with_background(Fill::None);
        } else if self.overlayed_scrollbar {
            scrollbar.with_background(Fill::Solid(ColorU::transparent_black()));
        } else {
            scrollbar.with_background(self.scrollbar_track_background);
        }

        let scrollbar_size_percentage = self.scrollbar_size_percentage.unwrap();
        let scrollbar_position_percentage = self.scrollbar_position_percentage.unwrap();
        if scrollbar_size_percentage < 1. {
            let scrollbar_thumb_size = self.size_along_inverted_axis(
                scrollable_size * scrollbar_size_percentage,
                self.scrollbar_size.as_f32(),
            );
            let scrollbar_thumb_origin = scrollbar_track_origin
                + self.size_along_inverted_axis(
                    (scrollable_size - scrollbar_thumb_size) * scrollbar_position_percentage,
                    self.padding_between_child_and_scrollbar,
                );

            self.scrollbar_thumb_bounds =
                Some(RectF::new(scrollbar_thumb_origin, scrollbar_thumb_size));
            self.scrollbar_thumb_origin = Some(scrollbar_thumb_origin);

            let hovered = self.state().hovered;
            let child_hovered = self.state().child_hovered;
            let background = if hovered {
                self.active_scrollbar_thumb_background
            } else if child_hovered {
                self.nonactive_scrollbar_thumb_background
            } else {
                Fill::None
            };

            ctx.scene
                .draw_rect_with_hit_recording(RectF::new(
                    scrollbar_thumb_origin,
                    scrollbar_thumb_size,
                ))
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));
        } else {
            self.scrollbar_thumb_origin = Some(origin);
            self.scrollbar_thumb_bounds = Some(RectF::new(vec2f(0., 0.), vec2f(0., 0.)));
        }

        // See comment above about the layering of the scrollbar and scrollbar.
        if self.overlayed_scrollbar {
            ctx.scene.stop_layer();
        }

        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled = self.child.dispatch_event(event, ctx, app);
        let z_index = *self.child_max_z_index.as_ref().unwrap();

        match event.raw_event() {
            Event::LeftMouseDragged { position, .. } => {
                let is_dragging = self.state().started.is_some();
                if !is_dragging {
                    return handled;
                }
                self.mouse_dragged(*position, ctx, app);
                true
            }
            Event::LeftMouseDown { position, .. } => {
                if ctx.is_covered(Point::from_vec2f(*position, z_index)) {
                    return handled;
                }

                let Some(thumb_bounds) = self.scrollbar_thumb_bounds else {
                    log::warn!(
                        "Expected scrollbar thumb bounds to exist in dispatch_event, but got None"
                    );
                    return handled;
                };

                if thumb_bounds.contains_point(*position) {
                    self.start_scrolling(*position);

                    // Dispatch an action in tests so we can perform assertions
                    // on clicks.
                    #[cfg(test)]
                    ctx.dispatch_action("scrollable_click::on_thumb", ());

                    true
                } else if self
                    .scrollbar_track_bounds
                    .is_some_and(|bounds| bounds.contains_point(*position))
                {
                    // If mouse down happens in the x range of scrollbar but not on the thumb,
                    // we should scroll to the mouse down position.
                    let previous_position = thumb_bounds.center().along(self.axis);
                    self.jump_to_position(
                        previous_position.into_pixels(),
                        position.along(self.axis).into_pixels(),
                        ctx,
                        app,
                    );

                    // Dispatch an action in tests so we can perform assertions
                    // on clicks.
                    #[cfg(test)]
                    ctx.dispatch_action("scrollable_click::on_gutter", ());

                    true
                } else {
                    handled
                }
            }
            Event::LeftMouseUp { .. } => {
                let previous_dragging_position = self.state().started;
                if previous_dragging_position.is_some() {
                    self.end_scrolling();
                    true
                } else {
                    handled
                }
            }
            Event::MouseMoved { position, .. } => {
                let is_dragging = self.state().started.is_some();

                if is_dragging {
                    return handled;
                }
                let is_covered = ctx.is_covered(Point::from_vec2f(*position, z_index));

                let mouse_in = self
                    .scrollbar_thumb_bounds
                    .unwrap()
                    .contains_point(*position)
                    && !is_covered;
                let was_hovered = mem::replace(&mut self.state().hovered, mouse_in);

                let mouse_in_child = self
                    .child
                    .bounds()
                    .unwrap_or_default()
                    .contains_point(*position)
                    && !is_covered;
                let child_was_hovered =
                    mem::replace(&mut self.state().child_hovered, mouse_in_child);

                if was_hovered != mouse_in || child_was_hovered != mouse_in_child {
                    ctx.notify();
                }

                if mouse_in {
                    true
                } else {
                    handled
                }
            }
            Event::ScrollWheel {
                position,
                delta,
                precise,
                modifiers: ModifiersState { ctrl: false, .. },
            } => {
                if !self.child.should_handle_scroll_wheel() {
                    return handled;
                }

                if self.bounds().unwrap().contains_point(*position)
                    && !ctx.is_covered(Point::from_vec2f(*position, z_index))
                {
                    self.mousewheel(*delta, *precise, ctx);
                    return true;
                }
                handled
            }
            _ => handled,
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.scrollable_size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

#[cfg(test)]
#[path = "scrollable_tests.rs"]
mod tests;
