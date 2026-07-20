//! [`TuiSizeConstraintSwitch`] selects one prebuilt child from the current
//! [`TuiConstraint`].
//!
//! This mirrors the GUI `SizeConstraintSwitch`: conditions are checked in
//! order during layout, and every later lifecycle pass delegates to the child
//! selected by that layout.

use super::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
};
use crate::AppContext;

/// A condition that selects a child from a [`TuiSizeConstraintSwitch`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiSizeConstraintCondition {
    WidthLessThan(u16),
    HeightLessThan(u16),
    SizeSmallerThan(TuiSize),
}

impl TuiSizeConstraintCondition {
    fn matches(self, constraint: TuiConstraint) -> bool {
        match self {
            Self::WidthLessThan(width) => constraint.max.width < width,
            Self::HeightLessThan(height) => constraint.max.height < height,
            Self::SizeSmallerThan(size) => {
                constraint.max.width < size.width && constraint.max.height < size.height
            }
        }
    }
}

/// Selects a child according to the size constraint supplied during layout.
pub struct TuiSizeConstraintSwitch {
    default_child: Box<dyn TuiElement>,
    children: Vec<(TuiSizeConstraintCondition, Box<dyn TuiElement>)>,
    active_child_index: Option<usize>,
    cached_constraint: Option<TuiConstraint>,
}

impl TuiSizeConstraintSwitch {
    /// Creates a switch whose first matching conditional child wins.
    pub fn new(
        default_child: Box<dyn TuiElement>,
        children: impl Into<Vec<(TuiSizeConstraintCondition, Box<dyn TuiElement>)>>,
    ) -> Self {
        Self {
            default_child,
            children: children.into(),
            active_child_index: None,
            cached_constraint: None,
        }
    }

    fn active_child(&self) -> &dyn TuiElement {
        self.active_child_index
            .and_then(|index| self.children.get(index))
            .map(|(_, child)| child.as_ref())
            .unwrap_or(self.default_child.as_ref())
    }

    fn active_child_mut(&mut self) -> &mut dyn TuiElement {
        self.active_child_index
            .and_then(|index| self.children.get_mut(index))
            .map(|(_, child)| child.as_mut())
            .unwrap_or(self.default_child.as_mut())
    }
}

impl TuiElement for TuiSizeConstraintSwitch {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        if self.cached_constraint != Some(constraint) {
            self.active_child_index = self
                .children
                .iter()
                .position(|(condition, _)| condition.matches(constraint));
            self.cached_constraint = Some(constraint);
        }
        self.active_child_mut().layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.active_child_mut().after_layout(ctx, app);
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.active_child_mut().render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.active_child().size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.active_child().origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.active_child_mut().present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.active_child_mut()
            .dispatch_event(event, event_ctx, app)
    }
}

#[cfg(test)]
#[path = "size_constraint_switch_tests.rs"]
mod tests;
