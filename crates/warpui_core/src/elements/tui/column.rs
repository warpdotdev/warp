//! [`TuiColumn`]: a vertical stack that lays its children out top-to-bottom.
//!
//! # Construction
//! Start from [`TuiColumn::new`] (empty) and append children with
//! [`child`](TuiColumn::child) (fixed height) or
//! [`flex_child`](TuiColumn::flex_child) (fills leftover height). The
//! [`TuiParentElement`](super::TuiParentElement) trait's `with_child` /
//! `with_children` / `add_child` / `add_children` also work and add fixed
//! children.
//!
//! # Layout policy
//! The column fills the width it is offered and gives every child that same
//! width. Each fixed child is laid out against the remaining height (loose) and
//! takes its natural height; children are stacked without gaps from the top, and
//! children that fall past the available height are clipped.
//!
//! A child added with [`flex_child`](TuiColumn::flex_child) instead *fills* the
//! height left over after the fixed children, so a body can sit above or below a
//! fixed element (used here to dock the input at the bottom beneath a flex
//! spacer). With at least one flex child the column fills the height it is
//! offered and splits the leftover evenly across the flex children (any
//! remainder going to the earlier ones). With no flex children the column's
//! height is the sum of its children's, clamped to the constraint.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::AppContext;

/// A child of a [`TuiColumn`] plus whether it fills leftover vertical space.
struct ColumnChild {
    element: Box<dyn TuiElement>,
    flex: bool,
}

#[derive(Default)]
pub struct TuiColumn {
    children: Vec<ColumnChild>,
    /// Sizes returned by each child's `layout()` call; populated during layout
    /// so `render`, `cursor_position`, and `dispatch_event` have consistent slot
    /// information.
    child_sizes: Vec<TuiSize>,
}

impl TuiColumn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a fixed-height child, laid out against the remaining height.
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
}

/// Allows [`TuiParentElement`](super::TuiParentElement) (`with_child`,
/// `with_children`) to work on `TuiColumn`, adding fixed children.
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
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        let has_flex = self.children.iter().any(|c| c.flex);
        self.child_sizes.clear();

        if !has_flex {
            // No flex children: give each child the remaining height (loose) and
            // sum the actual sizes.
            let mut total_height: u16 = 0;
            for child in &mut self.children {
                let remaining_height = constraint.max.height.saturating_sub(total_height);
                let child_constraint = TuiConstraint::loose(TuiSize::new(width, remaining_height));
                let size = child.element.layout(child_constraint, ctx, app);
                total_height = total_height.saturating_add(size.height);
                self.child_sizes.push(size);
            }
            return TuiSize::new(width, constraint.constrain_height(total_height));
        }

        // Flex children: two passes.
        // Pass 1 — lay out fixed children to measure their total height.
        let mut fixed_sizes: Vec<Option<TuiSize>> = Vec::with_capacity(self.children.len());
        let mut total_fixed: u16 = 0;
        for child in &mut self.children {
            if child.flex {
                fixed_sizes.push(None);
            } else {
                let remaining_height = constraint.max.height.saturating_sub(total_fixed);
                let child_constraint = TuiConstraint::loose(TuiSize::new(width, remaining_height));
                let size = child.element.layout(child_constraint, ctx, app);
                total_fixed = total_fixed.saturating_add(size.height);
                fixed_sizes.push(Some(size));
            }
        }
        // Pass 2 — distribute leftover evenly among the flex children (guaranteed
        // non-zero count because `has_flex` is true).
        let flex_count = self.children.iter().filter(|c| c.flex).count() as u16;
        let leftover = constraint.max.height.saturating_sub(total_fixed);
        let base = leftover / flex_count;
        let remainder = leftover % flex_count;
        let mut flex_rank = 0u16;
        for (child, maybe_size) in self.children.iter_mut().zip(fixed_sizes) {
            let size = if child.flex {
                let slot = base + u16::from(flex_rank < remainder);
                flex_rank += 1;
                // Lay out with a tight height so the child fills its slot.
                child
                    .element
                    .layout(TuiConstraint::tight(TuiSize::new(width, slot)), ctx, app);
                TuiSize::new(width, slot)
            } else {
                maybe_size.expect("fixed child was measured in pass 1")
            };
            self.child_sizes.push(size);
        }
        TuiSize::new(width, constraint.max.height)
    }

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

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            if let Some((cx, cy)) = child.element.cursor_position(slot, ctx) {
                // Offset is relative to the slot, not the full area.
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
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        // Offer the event to each child in its rendered slot (mirrors render's
        // stacking); the first child to handle it consumes it. Children clipped
        // past the available height see no events.
        let mut remaining = area;
        for (child, size) in self.children.iter_mut().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            if child
                .element
                .dispatch_event(event, slot, event_ctx, ctx, app)
            {
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
