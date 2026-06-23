//! [`TuiColumn`]: a vertical stack that lays its children out top-to-bottom.
//!
//! # Construction
//! Start from [`TuiColumn::new`] (empty) and append children via the
//! [`TuiParentElement`](super::TuiParentElement) trait:
//! [`with_child`](super::TuiParentElement::with_child),
//! [`with_children`](super::TuiParentElement::with_children),
//! [`add_child`](super::TuiParentElement::add_child),
//! [`add_children`](super::TuiParentElement::add_children).
//!
//! # Layout policy
//! The column fills the width it is offered and gives every child that same
//! width. Each child is allocated exactly its
//! [`desired_height`](TuiElement::desired_height) at that width; children are
//! stacked without gaps from the top, and the column's own height is the sum of
//! those heights clamped to the constraint. Children that fall past the
//! available height are clipped.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::{AppContext, Event};

#[derive(Default)]
pub struct TuiColumn {
    children: Vec<Box<dyn TuiElement>>,
    /// Sizes returned by each child's `layout()` call; populated during layout
    /// so `render`, `cursor_position`, and `dispatch_event` don't need to
    /// re-invoke `desired_height` (which has no context).
    child_sizes: Vec<TuiSize>,
}

impl TuiColumn {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Extend<Box<dyn TuiElement>> for TuiColumn {
    fn extend<I: IntoIterator<Item = Box<dyn TuiElement>>>(&mut self, iter: I) {
        self.children.extend(iter);
    }
}

impl TuiElement for TuiColumn {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        let mut total_height: u16 = 0;
        self.child_sizes.clear();
        for child in &mut self.children {
            // Use the remaining available height rather than desired_height so
            // child views (which have no locally-cached size) get a valid
            // budget. The child's layout clamps to its actual content height.
            let remaining_height = constraint.max.height.saturating_sub(total_height);
            let child_constraint = TuiConstraint::loose(TuiSize::new(width, remaining_height));
            let size = child.layout(child_constraint, ctx);
            total_height = total_height.saturating_add(size.height);
            self.child_sizes.push(size);
        }
        TuiSize::new(width, constraint.constrain_height(total_height))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            child.render(slot, buffer, ctx);
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
            if let Some((cx, cy)) = child.cursor_position(slot, ctx) {
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
            child.present(ctx);
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
        // Offer the event to each child in its rendered slot (mirrors render's
        // stacking); the first child to handle it consumes it. Children clipped
        // past the available height see no events.
        let mut remaining = area;
        for (child, size) in self.children.iter_mut().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = remaining.split_top(size.height);
            if child.dispatch_event(event, slot, event_ctx, ctx, app) {
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
