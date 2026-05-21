use super::Point;
use crate::{
    event::DispatchedEvent, AfterLayoutContext, AppContext, Element, EventContext, LayoutContext,
    PaintContext, SizeConstraint,
};
use pathfinder_geometry::vector::Vector2F;

#[derive(Clone, Copy, Debug)]
pub struct Size {
    width: f32,
    height: f32,
}

/// Conditions on the element's [`SizeConstraint`].
#[derive(Clone, Copy, Debug)]
pub enum SizeConstraintCondition {
    /// A condition in which the [`SizeConstraint`] is valid if its max width is less than the
    /// contained f32.
    WidthLessThan(f32),

    /// A condition in which the [`SizeConstraint`] is valid if its max height is less than the
    /// contained f32.
    HeightLessThan(f32),

    /// A condition in which the [`SizeConstraint`] is valid if its max width and height are _both_
    /// less than width and height in the contained [`Size`].
    SizeSmallerThan(Size),
}

impl SizeConstraintCondition {
    fn is_valid_size_constraint(&self, constraint: &SizeConstraint) -> bool {
        match self {
            SizeConstraintCondition::WidthLessThan(max_width) => constraint.max.x() < *max_width,
            SizeConstraintCondition::HeightLessThan(max_height) => constraint.max.y() < *max_height,
            SizeConstraintCondition::SizeSmallerThan(Size {
                width: max_width,
                height: max_height,
            }) => constraint.max.x() < *max_width && constraint.max.y() < *max_height,
        }
    }
}

/// Element that determines which child element to render based on its [`SizeConstraint`]. This
/// element may be used to implement responsive layouts for different window sizes.
///
/// Example:
///
/// ```ignore
/// let switch = SizeConstraintSwitch::new(default_element, [
///                 (SizeConstraintCondition::WidthLessThan(400.), narrow_width_element)
///                 (SizeConstraintCondition::WidthLessThan(800.), medium_width_element)
///             ]);
/// ```
pub struct SizeConstraintSwitch {
    default_child: Box<dyn Element>,
    children: Vec<(SizeConstraintCondition, Box<dyn Element>)>,
    active_child_index: Option<usize>,

    /// A cached copy of the [`SizeConstraint`] passed to the element's most recent `layout()`
    /// call. `None` if `layout()` has not yet been called on this element.
    ///
    /// This is used to ensure that the correct child is `paint()`-ed when the element's
    /// [`SizeConstraint`] changes during the element's lifetime.
    cached_size_constraint: Option<SizeConstraint>,
}

impl SizeConstraintSwitch {
    /// Children's [`SizeConstraintCondition`]s are checked in the order that they are passed into
    /// this constructor. If more than one child's condition is satisfied, the child that appeared
    /// earlier in the `children` argument will be rendered.
    pub fn new(
        default_child: Box<dyn Element>,
        children: impl Into<Vec<(SizeConstraintCondition, Box<dyn Element>)>>,
    ) -> Self {
        Self {
            default_child,
            children: children.into(),
            active_child_index: None,
            cached_size_constraint: None,
        }
    }

    /// Returns the child that should be rendered.
    fn active_child(&self) -> &dyn Element {
        self.active_child_index
            .and_then(|index| self.children.get(index).map(|child| &child.1))
            .unwrap_or(&self.default_child)
            .as_ref()
    }

    /// Returns a mutable reference to the child that should be rendered.
    fn active_child_mut(&mut self) -> &mut dyn Element {
        self.active_child_index
            .and_then(|index| self.children.get_mut(index))
            .map(|(_, child)| child.as_mut())
            .unwrap_or_else(|| self.default_child.as_mut())
    }
}

impl Element for SizeConstraintSwitch {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        if self.cached_size_constraint.map(|constraint| constraint.max) != Some(constraint.max)
            || self.cached_size_constraint.map(|constraint| constraint.min) != Some(constraint.min)
        {
            self.active_child_index = self.children.iter().position(|(constraint_condition, _)| {
                constraint_condition.is_valid_size_constraint(&constraint)
            });
        }
        self.cached_size_constraint = Some(constraint);
        self.active_child_mut().layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.active_child_mut().after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.active_child_mut().paint(origin, ctx, app)
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.active_child_mut().dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.active_child().size()
    }

    fn origin(&self) -> Option<Point> {
        self.active_child().origin()
    }

    #[cfg(any(test, feature = "test-util"))]
    fn debug_text_content(&self) -> Option<String> {
        self.active_child().debug_text_content()
    }
}

#[cfg(test)]
#[path = "size_constraint_switch_tests.rs"]
mod tests;
