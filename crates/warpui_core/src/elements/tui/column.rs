//! [`TuiColumn`]: a vertical stack that lays its children out top-to-bottom.
//!
//! # Construction
//! Start from [`TuiColumn::new`] (empty) and append children with
//! [`child`](TuiColumn::child), or build from an iterator with
//! [`with_children`](TuiColumn::with_children).
//!
//! # Layout policy
//! The column fills the width it is offered and gives every child that same
//! width. By default each child is allocated exactly its
//! [`desired_height`](TuiElement::desired_height) at that width; children are
//! stacked without gaps from the top, and the column's own height is the sum of
//! those heights clamped to the constraint. Children that fall past the
//! available height are clipped.
//!
//! A child added with [`flex_child`](TuiColumn::flex_child) instead *fills* the
//! height left over after the fixed children, so a scrollable body can sit under
//! a fixed header. When at least one flex child is present the column fills the
//! height it is offered, and the leftover (available height minus the fixed
//! children's desired heights) is split evenly across the flex children (any
//! remainder going to the earlier ones). With no flex children the column
//! behaves exactly as the default sum-and-clip policy above.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::{AppContext, Event};

/// A child of a [`TuiColumn`] plus whether it fills leftover vertical space.
struct ColumnChild {
    element: Box<dyn TuiElement>,
    flex: bool,
}

#[derive(Default)]
pub struct TuiColumn {
    children: Vec<ColumnChild>,
    /// Sizes returned by each child's `layout()` call; populated during layout
    /// so `render`, `cursor_position`, and `dispatch_event` have consistent
    /// slot information without needing a `desired_height` context.
    child_sizes: Vec<TuiSize>,
}

impl TuiColumn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a fixed-height child, allocated its `desired_height`.
    pub fn child(mut self, child: impl TuiElement + 'static) -> Self {
        self.children.push(ColumnChild {
            element: Box::new(child),
            flex: false,
        });
        self
    }

    /// Appends a child that fills the height left over after the fixed children
    /// (shared evenly when there are several flex children).
    pub fn flex_child(mut self, child: impl TuiElement + 'static) -> Self {
        self.children.push(ColumnChild {
            element: Box::new(child),
            flex: true,
        });
        self
    }

    pub fn with_children(children: impl IntoIterator<Item = Box<dyn TuiElement>>) -> Self {
        Self {
            children: children
                .into_iter()
                .map(|element| ColumnChild {
                    element,
                    flex: false,
                })
                .collect(),
            child_sizes: Vec::new(),
        }
    }
}

/// Allows [`TuiParentElement`](super::TuiParentElement) (`with_child`,
/// `with_children`) to work on `TuiColumn` for backward compatibility.
impl Extend<Box<dyn TuiElement>> for TuiColumn {
    fn extend<I: IntoIterator<Item = Box<dyn TuiElement>>>(&mut self, iter: I) {
        self.children
            .extend(iter.into_iter().map(|element| ColumnChild {
                element,
                flex: false,
            }));
    }
}

impl TuiElement for TuiColumn {
    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            child.element.render(slot, buffer, ctx);
            remaining = rest;
        }
    }

    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        let has_flex = self.children.iter().any(|c| c.flex);
        self.child_sizes.clear();

        if !has_flex {
            // No flex children: give each child remaining height (loose), sum
            // actual sizes.
            let mut total_height: u16 = 0;
            for child in &mut self.children {
                let remaining = constraint.max.height.saturating_sub(total_height);
                let child_constraint = TuiConstraint::loose(TuiSize::new(width, remaining));
                let size = child.element.layout(child_constraint, ctx);
                total_height = total_height.saturating_add(size.height);
                self.child_sizes.push(size);
            }
            TuiSize::new(width, constraint.constrain_height(total_height))
        } else {
            // Flex children: two passes.
            // Pass 1 — layout fixed children to measure their total height.
            let mut fixed_sizes: Vec<Option<TuiSize>> = Vec::with_capacity(self.children.len());
            let mut total_fixed: u16 = 0;
            for child in &mut self.children {
                if child.flex {
                    fixed_sizes.push(None);
                } else {
                    let remaining = constraint.max.height.saturating_sub(total_fixed);
                    let child_constraint = TuiConstraint::loose(TuiSize::new(width, remaining));
                    let size = child.element.layout(child_constraint, ctx);
                    total_fixed = total_fixed.saturating_add(size.height);
                    fixed_sizes.push(Some(size));
                }
            }
            // Pass 2 — distribute leftover evenly among flex children.
            let flex_count = self.children.iter().filter(|c| c.flex).count() as u16;
            let leftover = constraint.max.height.saturating_sub(total_fixed);
            let base = leftover / flex_count;
            let remainder = leftover % flex_count;
            let mut flex_rank = 0u16;
            for (child, maybe_size) in self.children.iter_mut().zip(fixed_sizes) {
                let size = if child.flex {
                    let slot = base + if flex_rank < remainder { 1 } else { 0 };
                    flex_rank += 1;
                    // Lay out with a tight height so the child fills its slot.
                    child.element.layout(TuiConstraint::tight(TuiSize::new(width, slot)), ctx);
                    TuiSize::new(width, slot)
                } else {
                    maybe_size.unwrap()
                };
                self.child_sizes.push(size);
            }
            TuiSize::new(width, constraint.max.height)
        }
    }


    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            if let Some((cx, cy)) = child.element.cursor_position(slot, ctx) {
                return Some((
                    slot.x.saturating_sub(area.x) + cx,
                    slot.y.saturating_sub(area.y) + cy,
                ));
            }
            remaining = rest;
        }
        None
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for child in &mut self.children {
            child.element.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        let mut remaining = area;
        for (child, size) in self.children.iter_mut().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            if child.element.dispatch_event(event, slot, event_ctx, ctx, app) {
                return true;
            }
            remaining = rest;
        }
        false
    }
}

#[cfg(test)]
#[path = "column_tests.rs"]
mod tests;
