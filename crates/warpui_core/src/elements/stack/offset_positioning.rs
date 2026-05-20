use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};

use crate::{presenter::PositionCache, SizeConstraint};

/// Defines the positioning for an element in a stack on both X and Y axes relative to the parent of
/// the stack or another child element within the stack. The child element is anchored to
/// the other element by `Anchor` and then offset on both axes by the specified offset.
#[derive(Default, Clone)]
pub struct OffsetPositioning {
    pub(super) x_axis: PositioningAxis<XAxisAnchor>,
    pub(super) y_axis: PositioningAxis<YAxisAnchor>,
}

impl OffsetPositioning {
    pub fn from_axes(
        x_axis: PositioningAxis<XAxisAnchor>,
        y_axis: PositioningAxis<YAxisAnchor>,
    ) -> Self {
        OffsetPositioning { x_axis, y_axis }
    }

    /// Returns an `OffsetPositioning` that may be used to position a stack child element relative
    /// to the stack parent's bounding rectangle.
    ///
    /// `bounds` specifies bounding behavior, if any, that should be used when calculating the stack
    /// child's final size and position. See docs on [`Bound`] for more detail.
    ///
    /// `parent_anchor` is used to determine the exact position on the parent's bounding rectangle
    /// relative to which the child will be positioned, using the child_anchor-determined position
    /// on the child's bounding rect.
    ///
    /// For example, to position the bottom right of the child offset from the top-left corner of
    /// the parent, pass `parent_anchor`: [`ParentAnchor::TopLeft`] and
    /// `child_anchor`: [`ChildAnchor::BottomRight`].
    pub fn offset_from_parent(
        offset: Vector2F,
        bound: ParentOffsetBounds,
        parent_anchor: ParentAnchor,
        child_anchor: ChildAnchor,
    ) -> Self {
        let parent_anchor: Anchor = parent_anchor.into();
        let child_anchor: Anchor = child_anchor.into();
        Self::from_axes(
            PositioningAxis::relative_to_parent(
                bound,
                OffsetType::Pixel(offset.x()),
                AnchorPair::new(parent_anchor.x(), child_anchor.x()),
            ),
            PositioningAxis::relative_to_parent(
                bound,
                OffsetType::Pixel(offset.y()),
                AnchorPair::new(parent_anchor.y(), child_anchor.y()),
            ),
        )
    }

    /// Returns an `OffsetPositioning` that may be used to position a stack child element relative
    /// to an arbitrary 'anchor' element that is wrapped and rendered within a [`SavePosition`].
    ///
    /// `bound` specifies bounding behavior, if any, that should be used when calculating the stack
    /// child's final size and position. See docs on [`Bound`] for more detail.
    ///
    /// `save_position_element_anchor` is used to determine the exact position on the anchor
    /// element's bounding rectangle relative to which the child will be positioned, using the
    /// child_anchor-determined position on the child's bounding rect.
    ///
    /// For example, to position the bottom right of the child offset from the top-left corner of
    /// the anchor, pass `save_position_element_anchor`: [`PositionedElementAnchor::TopLeft`] and
    /// `child_anchor`: [`ChildAnchor::BottomRight`].
    pub fn offset_from_save_position_element(
        saved_position_id: impl Into<String>,
        offset: Vector2F,
        bounds: PositionedElementOffsetBounds,
        save_position_element_anchor: PositionedElementAnchor,
        child_anchor: ChildAnchor,
    ) -> Self {
        let position_id = saved_position_id.into();
        let child_anchor: Anchor = child_anchor.into();
        let save_position_element_anchor: Anchor = save_position_element_anchor.into();
        Self::from_axes(
            PositioningAxis::relative_to_stack_child(
                position_id.clone(),
                bounds,
                OffsetType::Pixel(offset.x()),
                AnchorPair::new(save_position_element_anchor.x(), child_anchor.x()),
            ),
            PositioningAxis::relative_to_stack_child(
                position_id,
                bounds,
                OffsetType::Pixel(offset.y()),
                AnchorPair::new(save_position_element_anchor.y(), child_anchor.y()),
            ),
        )
    }

    /// Returns the size constraint to be used for the stack child, according to the
    /// anchors and bounding behaviors specified in the [`PositionAxis`]'s.
    pub fn size_constraint(
        &self,
        parent_size: Vector2F,
        window_size: Vector2F,
        default_constraint: SizeConstraint,
        position_cache: &PositionCache,
    ) -> SizeConstraint {
        let default_max_width = default_constraint.max.x();
        let size_constraint_max_x = self
            .x_axis
            .compute_max_child_width(parent_size.x(), window_size.x(), position_cache)
            .unwrap_or_else(|err| {
                // In production, or if the element is conditionally-rendered by
                // its x-axis anchor, use the default max width instead of panicking.
                debug_assert!(
                    self.x_axis.anchor.is_conditional(),
                    "Couldn't compute max child width: {err}"
                );
                None
            })
            .unwrap_or(default_max_width);

        let default_max_height = default_constraint.max.y();
        let size_constraint_max_y = self
            .y_axis
            .compute_max_child_height(parent_size.y(), window_size.y(), position_cache)
            .unwrap_or_else(|err| {
                debug_assert!(
                    self.y_axis.anchor.is_conditional(),
                    "Couldn't compute max child height: {err}"
                );
                None
            })
            .unwrap_or(default_max_height);

        SizeConstraint {
            min: vec2f(
                default_constraint.min.x().min(size_constraint_max_x),
                default_constraint.min.y().min(size_constraint_max_y),
            ),
            max: vec2f(size_constraint_max_x, size_constraint_max_y),
        }
    }
}

/// Bounding behaviors that may be applied to parent-offset stack children (added via
/// [`OffsetPositioning::offset_from_parent`].
#[derive(Clone, Copy)]
pub enum ParentOffsetBounds {
    /// The element's position may be adjusted to ensure it does not overflow its parent's bounds
    /// or the window's bounds. Size remains fixed.
    ParentByPosition,

    /// The element's size may be adjusted to ensure it does not overflow its parent's bounds.
    /// Position remains fixed.
    ParentBySize,

    /// The element's position may be adjusted to ensure it does not overflow the window's bounds.
    WindowByPosition,

    /// The element's position and size is unbounded relative to its parent element or window.
    Unbounded,
}

/// Bounding behaviors that may be applied to [`SavePosition`] element-offset stack children
/// (added via [`OffsetPositioning::offset_from_save_position_element`].
#[derive(Clone, Copy)]
pub enum PositionedElementOffsetBounds {
    /// The element's position may be adjusted to ensure it does not overflow its parent's bounds
    /// or the window's bounds. Size remains fixed.
    ParentByPosition,

    /// The element's position may be adjusted to ensure it does not overflow the bound of the
    /// element it is anchored to. Size remains fixed.
    AnchoredElement,

    /// The element's position may be adjusted to ensure it does not overflow the window's bounds.
    /// Size remains fixed.
    WindowByPosition,

    /// The element's size may be adjusted to ensure it does not overflow the window's bounds.
    /// Position remains fixed.
    WindowBySize,

    /// The element's position and size is unbounded relative to its parent element or window.
    Unbounded,
}

/// An 'anchor' point on the child element representing the point on the child's bounding rectangle
/// that should be positioned to the parent or [`SavePosition`]ed element.
#[derive(Clone, Copy, Debug)]
pub enum ChildAnchor {
    TopLeft,
    TopRight,
    TopMiddle,
    MiddleLeft,
    MiddleRight,
    Center,
    BottomLeft,
    BottomRight,
    BottomMiddle,
}

/// An 'anchor' point on the parent element representing the point on the parents's bounding
/// rectangle that should be used in concert with offset to position the [`Stack`]'s child element.
#[derive(Clone, Copy, Debug)]
pub enum ParentAnchor {
    TopLeft,
    TopRight,
    TopMiddle,
    MiddleLeft,
    MiddleRight,
    Center,
    BottomLeft,
    BottomRight,
    BottomMiddle,
}

/// An 'anchor' point on the [`SavePosition`]ed element representing the point on its bounding
/// rectangle that should be used in concert with offset to position the [`Stack`]'s child element.
#[derive(Clone, Copy, Debug)]
pub enum PositionedElementAnchor {
    TopLeft,
    TopRight,
    TopMiddle,
    MiddleLeft,
    MiddleRight,
    Center,
    BottomLeft,
    BottomRight,
    BottomMiddle,
}

// An 'anchor' point on an element's bounding rectangle.
//
// A pair of [`Anchor`]s constitutes an [`AnchorPair`], which can be used to determine the position
// of one element relative to another.
#[derive(Clone, Debug)]
enum Anchor {
    TopLeft,
    TopRight,
    TopMiddle,
    MiddleLeft,
    MiddleRight,
    Center,
    BottomLeft,
    BottomRight,
    BottomMiddle,
}

impl Anchor {
    fn x(&self) -> XAxisAnchor {
        match self {
            Anchor::TopLeft | Anchor::MiddleLeft | Anchor::BottomLeft => XAxisAnchor::Left,
            Anchor::TopRight | Anchor::MiddleRight | Anchor::BottomRight => XAxisAnchor::Right,
            Anchor::Center | Anchor::TopMiddle | Anchor::BottomMiddle => XAxisAnchor::Middle,
        }
    }

    fn y(&self) -> YAxisAnchor {
        match self {
            Anchor::TopLeft | Anchor::TopRight | Anchor::TopMiddle => YAxisAnchor::Top,
            Anchor::MiddleLeft | Anchor::MiddleRight | Anchor::Center => YAxisAnchor::Middle,
            Anchor::BottomLeft | Anchor::BottomRight | Anchor::BottomMiddle => YAxisAnchor::Bottom,
        }
    }
}

impl From<ChildAnchor> for Anchor {
    fn from(child_anchor_point: ChildAnchor) -> Self {
        match child_anchor_point {
            ChildAnchor::TopLeft => Anchor::TopLeft,
            ChildAnchor::TopRight => Anchor::TopRight,
            ChildAnchor::TopMiddle => Anchor::TopMiddle,
            ChildAnchor::MiddleLeft => Anchor::MiddleLeft,
            ChildAnchor::MiddleRight => Anchor::MiddleRight,
            ChildAnchor::Center => Anchor::Center,
            ChildAnchor::BottomLeft => Anchor::BottomLeft,
            ChildAnchor::BottomRight => Anchor::BottomRight,
            ChildAnchor::BottomMiddle => Anchor::BottomMiddle,
        }
    }
}

impl From<ParentAnchor> for Anchor {
    fn from(parent_anchor_point: ParentAnchor) -> Self {
        match parent_anchor_point {
            ParentAnchor::TopLeft => Anchor::TopLeft,
            ParentAnchor::TopRight => Anchor::TopRight,
            ParentAnchor::TopMiddle => Anchor::TopMiddle,
            ParentAnchor::MiddleLeft => Anchor::MiddleLeft,
            ParentAnchor::MiddleRight => Anchor::MiddleRight,
            ParentAnchor::Center => Anchor::Center,
            ParentAnchor::BottomLeft => Anchor::BottomLeft,
            ParentAnchor::BottomRight => Anchor::BottomRight,
            ParentAnchor::BottomMiddle => Anchor::BottomMiddle,
        }
    }
}

impl From<PositionedElementAnchor> for Anchor {
    fn from(positioned_element_anchor_point: PositionedElementAnchor) -> Self {
        match positioned_element_anchor_point {
            PositionedElementAnchor::TopLeft => Anchor::TopLeft,
            PositionedElementAnchor::TopRight => Anchor::TopRight,
            PositionedElementAnchor::TopMiddle => Anchor::TopMiddle,
            PositionedElementAnchor::MiddleLeft => Anchor::MiddleLeft,
            PositionedElementAnchor::MiddleRight => Anchor::MiddleRight,
            PositionedElementAnchor::Center => Anchor::Center,
            PositionedElementAnchor::BottomLeft => Anchor::BottomLeft,
            PositionedElementAnchor::BottomRight => Anchor::BottomRight,
            PositionedElementAnchor::BottomMiddle => Anchor::BottomMiddle,
        }
    }
}

#[derive(Clone, Copy)]
pub enum XAxisAnchor {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy)]
pub enum YAxisAnchor {
    Top,
    Bottom,
    Middle,
}

pub trait AxisAnchor {}
impl AxisAnchor for XAxisAnchor {}
impl AxisAnchor for YAxisAnchor {}

#[derive(Clone)]
pub struct AnchorPair<T>
where
    T: AxisAnchor + Clone,
{
    from: T,
    to: T,
}

impl<T> AnchorPair<T>
where
    T: AxisAnchor + Clone,
{
    pub fn new(from: T, to: T) -> Self {
        AnchorPair { from, to }
    }
}

/// Internal enum used to represents the bounds of an element.
///
/// Users of the [`Stack`] should refer to [`ParentOffsetBounds`] and
/// [`PositionedElementOffsetBounds`], which define the public API surface for specifying bounding
/// behavior.
#[derive(Clone, Copy)]
enum Bounds {
    /// The element's position or size is bound to the parent element.
    Parent(ParentOffsetBounds),

    /// The element's position or size is bound to the anchor [`SavePosition`]-ed element.
    PositionedElement(PositionedElementOffsetBounds),
}

/// Type of offposition offsets.
#[derive(Clone, Copy)]
pub enum OffsetType {
    /// Pixel value of the offset.
    Pixel(f32),
    /// Percentage offset based on the size of the anchored element. For example
    /// if the percentage is 0.5 and the anchored element has a width of 100, the
    /// pixel value of the offset will be 0.5 * 100. = 50. Note that this value
    /// can only be between 0. and 1.
    Percentage(f32),
}

/// Specifies how to position a child element on a given axis. The child element is anchored from
/// a corner (left/right on the x-axis, top-bottom on the y-axis) of the parent or relative element
/// onto a corner of the child element and then offset by `offset`.
#[derive(Clone)]
pub struct PositioningAxis<T>
where
    T: AxisAnchor + Clone,
{
    /// The anchor to position the element relative to.
    pub(super) anchor: PositioningAnchor,

    /// Specifies the bounding behavior of the positioned element.
    bounds: Bounds,

    /// Specifies the pair of points on the anchor element and positioned element's bounding
    /// rectangles which are used to calculate the element's final offset position.
    anchor_pair: AnchorPair<T>,

    /// Constant 'offset' applied to the element's final position, after calculating the anchor or
    /// parent element's anchored position. This could be either a pixel or percentage value.
    offset: OffsetType,
}

/// The anchor element that an element is positioned relative to. Currently,
/// we support positioning relative to:
/// * The element's parent
/// * An anchor element identified by its saved position
#[derive(Clone)]
pub(super) enum PositioningAnchor {
    RelativeToSavedPosition {
        /// The ID in the saved positions cache to anchor against. This ID must have been
        /// passed to the `SavePosition` element that wraps the anchor element
        position_id: String,
        /// Whether or not the element's display is conditional on the anchor element.
        /// If the anchor's position is not saved, the element will not be rendered,
        /// instead of panicking. This is useful when the anchor element is itself
        /// only sometimes rendered (for example, it's a cursor/selection).
        conditional: bool,
    },
    RelativeToParent,
}

impl PositioningAnchor {
    /// Whether or not the element's display is conditional on the anchor element
    /// having been rendered.
    pub fn is_conditional(&self) -> bool {
        match self {
            PositioningAnchor::RelativeToParent => false,
            PositioningAnchor::RelativeToSavedPosition { conditional, .. } => *conditional,
        }
    }
}

impl<T> PositioningAxis<T>
where
    T: AxisAnchor + Clone,
{
    pub fn relative_to_stack_child(
        position_id: impl Into<String>,
        bounds: PositionedElementOffsetBounds,
        offset: OffsetType,
        anchor_pair: AnchorPair<T>,
    ) -> Self {
        Self {
            anchor: PositioningAnchor::RelativeToSavedPosition {
                position_id: position_id.into(),
                conditional: false,
            },
            bounds: Bounds::PositionedElement(bounds),
            anchor_pair,
            offset,
        }
    }

    pub fn relative_to_parent(
        bounds: ParentOffsetBounds,
        offset: OffsetType,
        anchor_pair: AnchorPair<T>,
    ) -> Self {
        Self {
            anchor: PositioningAnchor::RelativeToParent,
            bounds: Bounds::Parent(bounds),
            anchor_pair,
            offset,
        }
    }

    /// Conditionally position the element along this axis. If the element
    /// cannot be positioned relative to its anchor, it will be skipped, rather
    /// than causing a panic.
    ///
    /// This is only supported for elements positioned relative to a stack child.
    pub fn with_conditional_anchor(mut self) -> Self {
        if let PositioningAnchor::RelativeToSavedPosition { conditional, .. } = &mut self.anchor {
            *conditional = true;
        } else {
            debug_assert!(
                false,
                "Can only use conditional_anchor with child-relative positioning"
            );
        }
        self
    }
}

impl PositioningAxis<XAxisAnchor> {
    // Computes where on the x axis the stack child should be positioned given the element it's
    // anchored to, the child's size, and the parent the stack is rendered into. The anchored
    // element can be another child in the stack or the parent of the stack.
    pub(super) fn compute_child_position(
        &self,
        child_size: Vector2F,
        parent_rect: RectF,
        window_size: Vector2F,
        position_cache: &PositionCache,
    ) -> Result<f32, String> {
        let anchor_element_rect = match &self.anchor {
            PositioningAnchor::RelativeToSavedPosition { position_id, .. } => {
                match position_cache.get_position(position_id) {
                    Some(position) => position,
                    None => {
                        return Err(format!("Position not set for {position_id:?}"));
                    }
                }
            }
            PositioningAnchor::RelativeToParent => parent_rect,
        };

        let anchor_x = match self.anchor_pair.from {
            XAxisAnchor::Left => anchor_element_rect.origin().x(),
            XAxisAnchor::Right => anchor_element_rect.max_x(),
            XAxisAnchor::Middle => {
                anchor_element_rect.origin().x() + anchor_element_rect.width() / 2.
            }
        };

        let pixel_offset = match self.offset {
            OffsetType::Percentage(ratio) => {
                let total_width = match self.bounds {
                    Bounds::PositionedElement(PositionedElementOffsetBounds::AnchoredElement) => {
                        (anchor_element_rect.width() - child_size.x()).max(0.)
                    }
                    _ => anchor_element_rect.width(),
                };
                total_width * ratio
            }
            OffsetType::Pixel(value) => value,
        };

        let child_position_x = match self.anchor_pair.to {
            XAxisAnchor::Left => anchor_x,
            XAxisAnchor::Right => anchor_x - child_size.x(),
            XAxisAnchor::Middle => anchor_x - child_size.x() / 2.,
        } + pixel_offset;

        let bounded_position_x = match self.bounds {
            Bounds::Parent(ParentOffsetBounds::ParentByPosition)
            | Bounds::PositionedElement(PositionedElementOffsetBounds::ParentByPosition) => {
                // our first try: find a position for the element to sit comfortably
                // within the left/right bounds of the parent.
                let mut bounded_position_x = child_position_x.clamp(
                    parent_rect.min_x(),
                    (parent_rect.max_x() - child_size.x()).max(parent_rect.min_x()),
                );

                // now, check if this position will cause the element to bleed offscreen.
                // if so, make it right-align with its parent.
                if bounded_position_x + child_size.x() > window_size.x() {
                    bounded_position_x = parent_rect.max_x() - child_size.x();
                }

                // oops, we now made the left go offscreen.
                // just center the element in the window then.
                if bounded_position_x < 0.0 {
                    bounded_position_x = (window_size.x() - child_size.x()) / 2.0;
                }

                bounded_position_x
            }
            Bounds::Parent(ParentOffsetBounds::ParentBySize) => {
                child_position_x.clamp(parent_rect.min_x(), parent_rect.max_x())
            }
            Bounds::Parent(ParentOffsetBounds::WindowByPosition)
            | Bounds::PositionedElement(PositionedElementOffsetBounds::WindowByPosition) => {
                child_position_x.clamp(0., (window_size.x() - child_size.x()).max(0.))
            }
            Bounds::PositionedElement(PositionedElementOffsetBounds::WindowBySize) => {
                child_position_x.clamp(0., window_size.x())
            }
            Bounds::PositionedElement(PositionedElementOffsetBounds::AnchoredElement) => {
                child_position_x.clamp(
                    anchor_element_rect.min_x(),
                    (anchor_element_rect.max_x() - child_size.x()).max(anchor_element_rect.min_x()),
                )
            }
            _ => child_position_x,
        };
        Ok(bounded_position_x)
    }

    pub(super) fn compute_max_child_width(
        &self,
        max_parent_width: f32,
        window_width: f32,
        position_cache: &PositionCache,
    ) -> Result<Option<f32>, String> {
        match self.bounds {
            Bounds::Parent(ParentOffsetBounds::ParentBySize) => {
                let parent_anchor_x =
                    Self::parent_anchor_x(self.anchor_pair.from, max_parent_width, self.offset);
                let mut max_child_width = match self.anchor_pair.to {
                    XAxisAnchor::Left => max_parent_width - parent_anchor_x,
                    XAxisAnchor::Right => parent_anchor_x,
                    XAxisAnchor::Middle => {
                        (max_parent_width - parent_anchor_x).min(parent_anchor_x) * 2.
                    }
                };
                max_child_width = max_child_width.clamp(0., max_parent_width);
                Ok(Some(max_child_width))
            }
            Bounds::PositionedElement(PositionedElementOffsetBounds::WindowBySize) => {
                match &self.anchor {
                    PositioningAnchor::RelativeToSavedPosition { position_id, .. } => {
                        let anchor_position_x = Self::positioned_element_anchor_x(
                            position_id.as_str(),
                            self.anchor_pair.from,
                            self.offset,
                            position_cache,
                        )?;

                        let max_width = match self.anchor_pair.to {
                            XAxisAnchor::Left => window_width - anchor_position_x,
                            XAxisAnchor::Right => anchor_position_x,
                            XAxisAnchor::Middle => {
                                (window_width - anchor_position_x).min(anchor_position_x) * 2.
                            }
                        };
                        Ok(Some(max_width.clamp(0., window_width)))
                    }
                    PositioningAnchor::RelativeToParent => {
                        debug_assert!(false, "Bounding element size to window is not supported for parent-offset stack children.");
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }

    // Returns the x coordinate within the parent's max size constraint for the anchor point on the
    // Parent element relative to which the stack child is positioned.
    //
    // If there is a `SavePosition` element (and the stack child is positioned relative to it,
    // rather than its parent), then returns `None`.
    fn parent_anchor_x(anchor: XAxisAnchor, width: f32, offset: OffsetType) -> f32 {
        let pixel_offset = match offset {
            OffsetType::Percentage(ratio) => ratio * width,
            OffsetType::Pixel(value) => value,
        };
        let parent_anchor_position_x = match anchor {
            XAxisAnchor::Left => 0.,
            XAxisAnchor::Right => width,
            XAxisAnchor::Middle => width / 2.,
        };
        parent_anchor_position_x + pixel_offset
    }

    // Returns the x coordinate for the anchor point on the `SavePosition` element relative to
    // which the stack child is positioned.
    //
    // If there is no `SavePosition` element (and the stack child is positioned relative to the
    // parent), then returns `None`.
    fn positioned_element_anchor_x(
        position_id: &str,
        anchor: XAxisAnchor,
        offset: OffsetType,
        position_cache: &PositionCache,
    ) -> Result<f32, String> {
        if let Some(anchor_element_position) = position_cache.get_position(position_id) {
            let pixel_offset = match offset {
                OffsetType::Pixel(value) => value,
                OffsetType::Percentage(ratio) => ratio * anchor_element_position.width(),
            };
            let anchor_position_x = match anchor {
                XAxisAnchor::Left => anchor_element_position.min_x(),
                XAxisAnchor::Right => anchor_element_position.max_x(),
                XAxisAnchor::Middle => anchor_element_position.center().x(),
            };
            Ok(anchor_position_x + pixel_offset)
        } else {
            Err(format!(
                "Position not found for element with position_id {position_id}"
            ))
        }
    }
}

impl Default for PositioningAxis<XAxisAnchor> {
    fn default() -> Self {
        Self {
            anchor_pair: AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
            bounds: Bounds::Parent(ParentOffsetBounds::Unbounded),
            offset: OffsetType::Pixel(0.),
            anchor: PositioningAnchor::RelativeToParent,
        }
    }
}

impl PositioningAxis<YAxisAnchor> {
    // Computes where on the y axis the stack child should be positioned given the element it's
    // anchored to, the child's size, and the parent the stack is rendered into. The anchored
    // element can be another child in the stack or the parent of the stack.
    pub(super) fn compute_child_position(
        &self,
        child_size: Vector2F,
        parent_rect: RectF,
        window_size: Vector2F,
        position_cache: &PositionCache,
    ) -> Result<f32, String> {
        let anchor_element_rect = match &self.anchor {
            PositioningAnchor::RelativeToSavedPosition { position_id, .. } => {
                match position_cache.get_position(position_id) {
                    Some(position) => position,
                    None => {
                        return Err(format!("Position not set for {position_id:?}"));
                    }
                }
            }
            PositioningAnchor::RelativeToParent => parent_rect,
        };

        let anchor_y = match self.anchor_pair.from {
            YAxisAnchor::Top => anchor_element_rect.origin().y(),
            YAxisAnchor::Bottom => anchor_element_rect.max_y(),
            YAxisAnchor::Middle => anchor_element_rect.center().y(),
        };

        let pixel_offset = match self.offset {
            OffsetType::Percentage(ratio) => {
                let total_height = match self.bounds {
                    Bounds::PositionedElement(PositionedElementOffsetBounds::AnchoredElement) => {
                        (anchor_element_rect.height() - child_size.y()).max(0.)
                    }
                    _ => anchor_element_rect.height(),
                };
                total_height * ratio
            }
            OffsetType::Pixel(value) => value,
        };

        let child_position_y = match self.anchor_pair.to {
            YAxisAnchor::Top => anchor_y,
            YAxisAnchor::Bottom => anchor_y - child_size.y(),
            YAxisAnchor::Middle => anchor_y - (child_size.y() / 2.),
        } + pixel_offset;

        let bounded_position_y = match self.bounds {
            Bounds::Parent(ParentOffsetBounds::ParentByPosition)
            | Bounds::PositionedElement(PositionedElementOffsetBounds::ParentByPosition) => {
                // our first try: find a position for the element to sit comfortably
                // within the upper/lower bounds of the parent.
                let mut bounded_position_y = child_position_y.clamp(
                    parent_rect.min_y(),
                    (parent_rect.max_y() - child_size.y()).max(parent_rect.min_y()),
                );

                // now, check if this position will cause the element to bleed offscreen.
                // if so, make it bottom-align with its parent.
                if bounded_position_y + child_size.y() > window_size.y() {
                    bounded_position_y = parent_rect.max_y() - child_size.y();
                }

                // oops, we now made the top go offscreen.
                // just center the element in the window then.
                if bounded_position_y < 0.0 {
                    bounded_position_y = (window_size.y() - child_size.y()) / 2.0;
                }

                bounded_position_y
            }
            Bounds::Parent(ParentOffsetBounds::ParentBySize) => {
                child_position_y.clamp(parent_rect.min_y(), parent_rect.max_y())
            }
            Bounds::Parent(ParentOffsetBounds::WindowByPosition)
            | Bounds::PositionedElement(PositionedElementOffsetBounds::WindowByPosition) => {
                child_position_y.clamp(0., (window_size.y() - child_size.y()).max(0.))
            }
            Bounds::PositionedElement(PositionedElementOffsetBounds::WindowBySize) => {
                child_position_y.clamp(0., window_size.y())
            }
            Bounds::PositionedElement(PositionedElementOffsetBounds::AnchoredElement) => {
                child_position_y.clamp(
                    anchor_element_rect.min_y(),
                    (anchor_element_rect.max_y() - child_size.y()).max(anchor_element_rect.min_y()),
                )
            }
            _ => child_position_y,
        };

        Ok(bounded_position_y)
    }

    /// Returns the maximum height of the positioned child based on the window height, maximum
    /// height given by the parent's size constraint, and the [`OffsetPositioning`]'s
    /// bounding behavior.
    pub(super) fn compute_max_child_height(
        &self,
        max_parent_height: f32,
        window_height: f32,
        position_cache: &PositionCache,
    ) -> Result<Option<f32>, String> {
        match self.bounds {
            Bounds::Parent(ParentOffsetBounds::ParentBySize) => {
                let parent_anchor_y =
                    Self::parent_anchor_y(self.anchor_pair.from, max_parent_height, self.offset);
                let max_child_height = match self.anchor_pair.to {
                    YAxisAnchor::Top => max_parent_height - parent_anchor_y,
                    YAxisAnchor::Bottom => parent_anchor_y,
                    YAxisAnchor::Middle => {
                        (max_parent_height - parent_anchor_y).min(parent_anchor_y) * 2.
                    }
                };
                Ok(Some(max_child_height.clamp(0., max_parent_height)))
            }
            Bounds::PositionedElement(PositionedElementOffsetBounds::WindowBySize) => {
                match &self.anchor {
                    PositioningAnchor::RelativeToSavedPosition { position_id, .. } => {
                        let anchor_position_y = Self::positioned_element_anchor_y(
                            position_id.as_str(),
                            self.anchor_pair.from,
                            self.offset,
                            position_cache,
                        )?;
                        let max_height = match self.anchor_pair.to {
                            YAxisAnchor::Top => window_height - anchor_position_y,
                            YAxisAnchor::Bottom => anchor_position_y,
                            YAxisAnchor::Middle => {
                                (window_height - anchor_position_y).min(anchor_position_y) * 2.
                            }
                        };
                        Ok(Some(max_height.clamp(0., window_height)))
                    }
                    PositioningAnchor::RelativeToParent => {
                        debug_assert!(false, "Bounding element size to window is not supported for parent-offset stack children.");
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }

    // Returns the y coordinate within the parent's max size constraint for the anchor point on the
    // Parent element relative to which the stack child is positioned.
    //
    // If there is a `SavePosition` element (and the stack child is positioned relative to it,
    // rather than its parent), then returns `None`.
    fn parent_anchor_y(anchor: YAxisAnchor, height: f32, offset: OffsetType) -> f32 {
        let pixel_offset = match offset {
            OffsetType::Percentage(ratio) => ratio * height,
            OffsetType::Pixel(value) => value,
        };
        let parent_anchor_position_y = match anchor {
            YAxisAnchor::Top => 0.,
            YAxisAnchor::Bottom => height,
            YAxisAnchor::Middle => height / 2.,
        };
        parent_anchor_position_y + pixel_offset
    }

    // Returns the y coordinate for the anchor point on the `SavePosition` element relative to
    // which the stack child is positioned.
    //
    // If there is no `SavePosition` element (and the stack child is positioned relative to the
    // parent), then returns `None`.
    fn positioned_element_anchor_y(
        position_id: &str,
        anchor: YAxisAnchor,
        offset: OffsetType,
        position_cache: &PositionCache,
    ) -> Result<f32, String> {
        if let Some(positioned_element_position) = position_cache.get_position(position_id) {
            let pixel_offset = match offset {
                OffsetType::Pixel(value) => value,
                OffsetType::Percentage(ratio) => ratio * positioned_element_position.height(),
            };
            let anchor_position_y = match anchor {
                YAxisAnchor::Top => positioned_element_position.min_y(),
                YAxisAnchor::Bottom => positioned_element_position.max_y(),
                YAxisAnchor::Middle => positioned_element_position.center().y(),
            };
            Ok(anchor_position_y + pixel_offset)
        } else {
            Err(format!(
                "Position not found for element with position_id {position_id}"
            ))
        }
    }
}

impl Default for PositioningAxis<YAxisAnchor> {
    fn default() -> Self {
        Self {
            anchor_pair: AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
            bounds: Bounds::Parent(ParentOffsetBounds::Unbounded),
            offset: OffsetType::Pixel(0.),
            anchor: PositioningAnchor::RelativeToParent,
        }
    }
}

#[cfg(test)]
#[path = "offset_positioning_tests.rs"]
mod tests;
