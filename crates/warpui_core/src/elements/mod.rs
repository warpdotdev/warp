mod align;
mod child_view;
mod clipped;
mod clipped_scrollable;
mod constrained_box;
mod container;
#[cfg(debug_assertions)]
mod debug;
mod dismiss;
mod drag;
pub mod drag_resize;
mod empty;
mod event_handler;
mod flex;
mod formatted_text_element;
mod hoverable;
mod icon;
mod image;
mod list;
mod live;
mod min_size;
pub mod new_scrollable;
mod percentage;
mod rect;
pub mod resizable;
mod scrollable;
mod selectable_area;
pub mod shared_scrollbar;
pub mod shimmering_text;
mod size_constraint_switch;
mod stack;
pub mod table;
mod text;
mod uniform_list;
mod viewported_list;

pub use align::*;
pub use child_view::*;
pub use clipped::*;
pub use clipped_scrollable::*;
pub use constrained_box::*;
pub use container::*;
#[cfg(debug_assertions)]
pub use debug::*;
pub use dismiss::*;
pub use drag::*;
pub use drag_resize::*;
pub use empty::*;
pub use event_handler::*;
pub use flex::*;
pub use formatted_text_element::*;
pub use hoverable::*;
pub use icon::*;
pub use image::*;
pub use list::*;
pub use live::*;
pub use min_size::*;
pub use new_scrollable::NewScrollable;
pub use percentage::*;
pub use rect::*;
pub use resizable::*;
pub use scrollable::*;
pub use selectable_area::*;
pub use shared_scrollbar::*;
pub use size_constraint_switch::*;
pub use stack::*;
pub use table::{
    RowBackground, Table, TableColumnWidth, TableConfig, TableHeader, TableState, TableStateHandle,
    TableVerticalSizing,
};
pub use text::*;
pub use uniform_list::*;
pub use viewported_list::*;

use crate::event::ModifiersState;
use crate::platform::Cursor;
use crate::{
    event::DispatchedEvent,
    text::{word_boundaries::WordBoundariesPolicy, IsRect, SelectionDirection, SelectionType},
    Gradient,
};
pub use crate::{
    scene::Dash, scene::ZIndex, AfterLayoutContext, AppContext, Event, EventContext, LayoutContext,
    PaintContext, SizeConstraint,
};
use core::fmt;
use pathfinder_color::ColorU;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use std::any::Any;
use std::borrow::Cow;
use std::ops::Range;
use std::sync::MutexGuard;

/// The result of dispatching an event.
/// This is (future) return type of `dispatch_event`.
/// This will eventually replace the current boolean return type, to be more explicit about
/// which events should continue to propagate to parent elements and which should stop.
pub enum DispatchEventResult {
    /// The event should continue to propagate to parent elements.
    PropagateToParent,
    /// The event should not propagate to parent elements.
    StopPropagation,
}

pub trait Element {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F;

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext);

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext);

    fn size(&self) -> Option<Vector2F>;

    fn origin(&self) -> Option<Point>;

    fn z_index(&self) -> Option<ZIndex> {
        self.origin().map(|p| p.z_index())
    }

    fn bounds(&self) -> Option<RectF> {
        try_rect_with_z(self.origin(), self.size())
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        None
    }

    /// Should be implemented alongside the SelectableElement trait. If implemented, it
    /// should return the element as a SelectableElement.
    fn as_selectable_element(&self) -> Option<&dyn SelectableElement> {
        None
    }

    /// Handle an event from the OS (e.g. Mouse or Keyboard events)
    ///
    /// Note: For each OS event, this is called on the root Element of the Element tree. Each
    /// Element is then itself responsible for calling `dispatch_event` on its children. The
    /// expectations for how an event propagates through the Element tree are:
    ///
    /// 1. Each Element that handles an event in some meaningful way will first verify that the
    ///    event applies to them by doing any necessary hit testing.
    /// 2. Each parent Element will unconditionally pass the event to its children by calling
    ///    `dispatch_event` on them, which allows the children to make their own determination
    ///    of whether or not the event applies.
    /// 3. Elements should return true if they handled the event and don't want it to propagate
    ///    to parent elements, and false if they want it to propagate to parent elements.
    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool;

    fn finish(self) -> Box<dyn Element>
    where
        Self: 'static + Sized,
    {
        Box::new(self)
    }

    #[cfg(debug_assertions)]
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Returns the text content of this element, if it contains text.
    /// This is primarily used for testing to verify rendered text content.
    /// Container elements should aggregate text from their children.
    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        None
    }
}

pub trait ParentElement: Extend<Box<dyn Element>> + Sized {
    #[cfg_attr(debug_assertions, track_caller)]
    fn add_children(&mut self, children: impl IntoIterator<Item = Box<dyn Element>>) {
        self.extend(children);
    }

    #[cfg_attr(debug_assertions, track_caller)]
    fn add_child(&mut self, child: Box<dyn Element>) {
        self.extend(Some(child))
    }

    #[cfg_attr(debug_assertions, track_caller)]
    fn with_children(mut self, children: impl IntoIterator<Item = Box<dyn Element>>) -> Self {
        self.add_children(children);
        self
    }

    #[cfg_attr(debug_assertions, track_caller)]
    fn with_child(self, child: Box<dyn Element>) -> Self {
        self.with_children(Some(child))
    }
}

impl<T> ParentElement for T where T: Extend<Box<dyn Element>> {}

#[derive(Clone, Debug)]
pub struct SelectionFragment {
    pub text: String,
    pub origin: Point,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    xy: Vector2F,
    z_index: ZIndex,
}

impl Point {
    pub fn new(x: f32, y: f32, z_index: ZIndex) -> Self {
        Self {
            xy: vec2f(x, y),
            z_index,
        }
    }

    pub fn from_vec2f(xy: Vector2F, z_index: ZIndex) -> Self {
        Self { xy, z_index }
    }

    pub fn x(&self) -> f32 {
        self.xy.x()
    }

    pub fn y(&self) -> f32 {
        self.xy.y()
    }

    pub fn xy(&self) -> Vector2F {
        self.xy
    }

    pub fn z_index(&self) -> ZIndex {
        self.z_index
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    pub fn invert(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }

    pub fn to_point(self, pos_along_main_axis: f32, pos_along_inverse_axis: f32) -> Vector2F {
        match self {
            Self::Horizontal => vec2f(pos_along_main_axis, pos_along_inverse_axis),
            Self::Vertical => vec2f(pos_along_inverse_axis, pos_along_main_axis),
        }
    }
}

pub enum AxisOrientation {
    Normal,
    Reverse,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Fill {
    #[default]
    None,
    Solid(ColorU),
    Gradient {
        start: Vector2F,
        end: Vector2F,
        start_color: ColorU,
        end_color: ColorU,
    },
}

impl From<ColorU> for Fill {
    fn from(color: ColorU) -> Self {
        Fill::Solid(color)
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Margin {
    top: f32,
    left: f32,
    bottom: f32,
    right: f32,
}

impl Margin {
    pub const fn uniform(margin: f32) -> Self {
        Margin {
            top: margin,
            left: margin,
            bottom: margin,
            right: margin,
        }
    }

    pub const fn with_left(mut self, margin: f32) -> Self {
        self.left = margin;
        self
    }

    pub const fn with_right(mut self, margin: f32) -> Self {
        self.right = margin;
        self
    }

    pub const fn with_top(mut self, margin: f32) -> Self {
        self.top = margin;
        self
    }

    pub const fn with_bottom(mut self, margin: f32) -> Self {
        self.bottom = margin;
        self
    }

    pub fn top(&self) -> f32 {
        self.top
    }

    pub fn left(&self) -> f32 {
        self.left
    }

    pub fn bottom(&self) -> f32 {
        self.bottom
    }

    pub fn right(&self) -> f32 {
        self.right
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    top: f32,
    left: f32,
    bottom: f32,
    right: f32,
}

impl Padding {
    pub const fn uniform(padding: f32) -> Self {
        Self {
            top: padding,
            left: padding,
            bottom: padding,
            right: padding,
        }
    }

    pub const fn with_top(mut self, padding: f32) -> Self {
        self.top = padding;
        self
    }

    pub const fn with_left(mut self, padding: f32) -> Self {
        self.left = padding;
        self
    }

    pub const fn with_bottom(mut self, padding: f32) -> Self {
        self.bottom = padding;
        self
    }

    pub const fn with_right(mut self, padding: f32) -> Self {
        self.right = padding;
        self
    }

    pub fn with_vertical(mut self, vertical: f32) -> Self {
        self.top = vertical;
        self.bottom = vertical;
        self
    }

    pub fn with_horizontal(mut self, horizontal: f32) -> Self {
        self.left = horizontal;
        self.right = horizontal;
        self
    }

    pub fn top(&self) -> f32 {
        self.top
    }

    pub fn left(&self) -> f32 {
        self.left
    }

    pub fn bottom(&self) -> f32 {
        self.bottom
    }

    pub fn right(&self) -> f32 {
        self.right
    }
}

#[derive(Default)]
pub struct Overdraw {
    top: f32,
    left: f32,
    bottom: f32,
    right: f32,
}

impl Border {
    pub const fn new(width: f32) -> Self {
        Self {
            width,
            color: Fill::None,
            top: false,
            left: false,
            bottom: false,
            right: false,
            dash: None,
        }
    }

    pub fn all(width: f32) -> Self {
        Self {
            width,
            color: Fill::None,
            top: true,
            left: true,
            bottom: true,
            right: true,
            dash: None,
        }
    }

    pub fn top(width: f32) -> Self {
        let mut border = Self::new(width);
        border.top = true;
        border
    }

    pub fn left(width: f32) -> Self {
        let mut border = Self::new(width);
        border.left = true;
        border
    }

    pub fn bottom(width: f32) -> Self {
        let mut border = Self::new(width);
        border.bottom = true;
        border
    }

    pub fn right(width: f32) -> Self {
        let mut border = Self::new(width);
        border.right = true;
        border
    }

    pub fn with_sides(mut self, top: bool, left: bool, bottom: bool, right: bool) -> Self {
        self.top = top;
        self.left = left;
        self.bottom = bottom;
        self.right = right;
        self
    }

    pub fn with_border_fill<F>(mut self, fill: F) -> Self
    where
        F: Into<Fill>,
    {
        self.color = fill.into();
        self
    }

    pub fn with_border_color(mut self, color: ColorU) -> Self {
        self.color = Fill::Solid(color);
        self
    }

    pub fn with_horizontal_border_gradient(mut self, gradient: Gradient) -> Self {
        self.color = Fill::Gradient {
            start: vec2f(0.0, 0.0),
            end: vec2f(1.0, 0.0),
            start_color: gradient.start,
            end_color: gradient.end,
        };
        self
    }

    pub fn with_border_gradient(
        mut self,
        start: Vector2F,
        end: Vector2F,
        gradient: Gradient,
    ) -> Self {
        self.color = Fill::Gradient {
            start,
            end,
            start_color: gradient.start,
            end_color: gradient.end,
        };
        self
    }

    /// Note: only implemented for sharp corners. ***DO NOT*** use for elements with corner radius != 0, as this causes visual bugs.
    pub fn with_dashed_border(mut self, dash: Dash) -> Self {
        self.dash = Some(dash);
        self
    }
}

impl From<ColorU> for Border {
    fn from(value: ColorU) -> Self {
        Border::all(1.).with_border_color(value)
    }
}

impl Fill {
    pub fn start(&self) -> Vector2F {
        match self {
            Self::Gradient { start, .. } => *start,
            _ => vec2f(0.0, 0.0),
        }
    }

    pub fn end(&self) -> Vector2F {
        match self {
            Self::Gradient { end, .. } => *end,
            _ => vec2f(1.0, 0.0),
        }
    }

    pub fn start_color(&self) -> ColorU {
        match self {
            Self::Gradient { start_color, .. } => *start_color,
            Self::Solid(color) => *color,
            Self::None => ColorU::transparent_black(),
        }
    }

    pub fn end_color(&self) -> ColorU {
        match self {
            Self::Gradient { end_color, .. } => *end_color,
            Self::Solid(color) => *color,
            Self::None => ColorU::transparent_black(),
        }
    }
}

/// Extends the `Vector2F` API to provider richer APIs for
/// element-related computations.
pub trait Vector2FExt {
    /// Converts the 2D vector to a scalar according to the given `axis`.
    fn along(self, axis: Axis) -> f32;

    /// Projects the 2D vector onto the given `axis`.
    /// e.g. (5, 2) -> (5, 0), along the x-axis.
    fn project_onto(self, axis: Axis) -> Vector2F;

    /// [`fmt::Display`] impl to format this `Vector2F` as a point.
    fn display_point(self) -> Vector2FDisplayPoint;

    /// [`fmt::Display`] impl to format this `Vector2F` as a size.
    fn display_size(self) -> Vector2FDisplaySize;
}

impl Vector2FExt for Vector2F {
    fn along(self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.x(),
            Axis::Vertical => self.y(),
        }
    }

    fn project_onto(self, axis: Axis) -> Vector2F {
        match axis {
            Axis::Horizontal => vec2f(self.x(), 0.),
            Axis::Vertical => vec2f(0., self.y()),
        }
    }

    fn display_point(self) -> Vector2FDisplayPoint {
        Vector2FDisplayPoint(self)
    }

    fn display_size(self) -> Vector2FDisplaySize {
        Vector2FDisplaySize(self)
    }
}

pub struct Vector2FDisplaySize(Vector2F);

impl fmt::Display for Vector2FDisplaySize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // We have to call .fmt directly so that formatting options are propagated.
        self.0.x().fmt(f)?;
        f.write_str("x")?;
        self.0.y().fmt(f)
    }
}

pub struct Vector2FDisplayPoint(Vector2F);

impl fmt::Display for Vector2FDisplayPoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // We have to call .fmt directly so that formatting options are propagated.
        f.write_str("(")?;
        self.0.x().fmt(f)?;
        f.write_str(", ")?;
        self.0.y().fmt(f)?;
        f.write_str(")")
    }
}

/// Extends the `f32` API to provider richer APIs for
/// element-related computations.
pub trait F32Ext {
    /// Converts the `f32` to a 2D vector along the provided `axis`.
    fn along(self, axis: Axis) -> Vector2F;
}

impl F32Ext for f32 {
    fn along(self, axis: Axis) -> Vector2F {
        match axis {
            Axis::Horizontal => vec2f(self, 0.),
            Axis::Vertical => vec2f(0., self),
        }
    }
}

/// Extends the `RectF` API to provider richer APIs for
/// element-related computations.
pub trait RectFExt {
    /// Returns the minimum value along the given `axis`.
    fn min_along(self, axis: Axis) -> f32;

    /// Returns the maximum value along the given `axis`.
    fn max_along(self, axis: Axis) -> f32;
}

impl RectFExt for RectF {
    fn min_along(self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.min_x(),
            Axis::Vertical => self.min_y(),
        }
    }

    fn max_along(self, axis: Axis) -> f32 {
        match axis {
            Axis::Horizontal => self.max_x(),
            Axis::Vertical => self.max_y(),
        }
    }
}

pub fn try_rect(origin: Option<Vector2F>, size: Option<Vector2F>) -> Option<RectF> {
    origin.and_then(|origin| size.map(|size| RectF::new(origin, size)))
}

pub fn try_rect_with_z(origin: Option<Point>, size: Option<Vector2F>) -> Option<RectF> {
    origin.and_then(|origin| size.map(|size| RectF::new(origin.xy(), size)))
}

/// The click handler provides the caller with the clicked text chunk index in
/// the provided clickable char ranges and the string corresponds to that chunk,
/// if one of the clickable chunks were clicked
pub type ClickHandler = Box<dyn FnMut(&ModifiersState, &mut EventContext, &AppContext)>;
/// The hover handler is called when the mouse either hovers or unhovers over a
/// hoverable char range, with the first argument being is_hovering.
pub type HoverHandler = Box<dyn FnMut(bool, &mut EventContext, &AppContext)>;

pub(crate) struct ClickableCharRange {
    pub(crate) char_range: Range<usize>,
    pub(crate) click_handler: ClickHandler,
}

pub(crate) struct HoverableCharRange {
    pub(crate) char_range: Range<usize>,
    pub(crate) hover_handler: HoverHandler,
    pub(crate) cursor_on_hover: Option<Cursor>,
    pub(crate) mouse_state: MouseStateHandle,
}

impl HoverableCharRange {
    fn mouse_state(&self) -> MutexGuard<'_, MouseState> {
        self.mouse_state
            .lock()
            .expect("The hoverable range should lock mouse state")
    }
}

/// SecretRange is used to store both the char range and byte range of a secret.
/// We need to do this since several APIs e.g. hover/click APIs, use char ranges,
/// whereas text-related APIs e.g. Regex and replace_range, use byte ranges.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct SecretRange {
    pub char_range: Range<usize>,
    pub byte_range: Range<usize>,
}

impl SecretRange {
    /// Extends the current range to include the provided range.
    pub fn extend_range_end(&mut self, other: &SecretRange) {
        self.char_range.end = self.char_range.end.max(other.char_range.end);
        self.byte_range.end = self.byte_range.end.max(other.byte_range.end);
    }
}

pub trait PartialClickableElement {
    /// clickable_char_ranges is the vector of char ranges that the caller can
    /// specify, where the callback will be called if any character in one of
    /// those char ranges was clicked
    fn with_clickable_char_range<F>(
        self,
        _clickable_char_range: Range<usize>,
        _callback: F,
    ) -> Self
    where
        F: 'static + FnMut(&ModifiersState, &mut EventContext, &AppContext);

    /// Registers a callback that is called when a character in the given hoverable_char_range
    /// is hovered or unhovered.
    fn with_hoverable_char_range<F>(
        self,
        hoverable_char_range: Range<usize>,
        mouse_state: MouseStateHandle,
        cursor_on_hover: Option<Cursor>,
        callback: F,
    ) -> Self
    where
        F: 'static + FnMut(bool, &mut EventContext, &AppContext);

    /// Replace in the given range of the text with the replacement text.
    fn replace_text_range(&mut self, range: SecretRange, replacement: Cow<'static, str>);
}

/// An element that can be selected, for use with the SelectableArea element.
/// It is expected that an element implementing this trait (i.e. Text)
/// also implements as_selectable_element().
pub trait SelectableElement {
    /// Return the element's selected fragments.
    fn get_selection(
        &self,
        _selection_start: Vector2F,
        _selection_end: Vector2F,
        _is_rect: IsRect,
    ) -> Option<Vec<SelectionFragment>>;

    /// Semantically expands the absolute selection point based on the unit.
    /// Does nothing if the unit is Char because there is no need to expand.
    /// Expands to the start of the unit if expand_to_start is true, otherwise
    /// expands to the end of the unit.
    /// If the absolute point before the element's bounds and expand_to_start is true,
    /// should expand to the start of the element. Similarly, if the absolute point is after
    /// the element's bounds and expand_to_start is false, should expand to the end of the element.
    /// Otherwise, should return None.
    fn expand_selection(
        &self,
        _absolute_point: Vector2F,
        _direction: SelectionDirection,
        _unit: SelectionType,
        _word_boundaries_policy: &WordBoundariesPolicy,
    ) -> Option<Vector2F>;

    /// Returns None if neither point is in the element.
    fn is_point_semantically_before(
        &self,
        _absolute_point: Vector2F,
        _absolute_point_other: Vector2F,
    ) -> Option<bool>;

    /// Runs smart selection on a point.
    /// Should return None if the point is outside the element vertically,
    /// but should snap to the nearest line if out of bounds horizontally.
    fn smart_select(
        &self,
        _absolute_point: Vector2F,
        _smart_select_fn: SmartSelectFn,
    ) -> Option<(Vector2F, Vector2F)>;

    /// The union of the returned regions defines the area within which a mouse click is considered
    /// to be a click performed on the element's selection. Should return an empty vector for
    /// elements that don't define any selection-specific click behaviors.
    fn calculate_clickable_bounds(&self, _current_selection: Option<Selection>) -> Vec<RectF>;
}
