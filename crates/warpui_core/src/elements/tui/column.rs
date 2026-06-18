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
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiPresentationContext, TuiRect,
    TuiRectExt, TuiSize,
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
        }
    }

    /// The height allocated to each child, in order, for a column laid out at
    /// `width` with `available_height` rows. Fixed children get their
    /// `desired_height`; the leftover is split evenly across the flex children
    /// (remainder to the earlier ones). With no flex children this is just each
    /// child's `desired_height`.
    fn slot_heights(&self, width: u16, available_height: u16) -> Vec<u16> {
        let mut heights: Vec<u16> = self
            .children
            .iter()
            .map(|child| {
                if child.flex {
                    0
                } else {
                    child.element.desired_height(width)
                }
            })
            .collect();

        let flex_count = self.children.iter().filter(|child| child.flex).count() as u16;
        if flex_count == 0 {
            return heights;
        }

        let fixed_total = heights.iter().fold(0u16, |acc, h| acc.saturating_add(*h));
        let leftover = available_height.saturating_sub(fixed_total);
        let base = leftover / flex_count;
        let remainder = leftover % flex_count;

        let mut flex_rank = 0u16;
        for (slot, child) in heights.iter_mut().zip(&self.children) {
            if child.flex {
                let extra = u16::from(flex_rank < remainder);
                *slot = base + extra;
                flex_rank += 1;
            }
        }
        heights
    }
}

impl TuiElement for TuiColumn {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        let has_flex = self.children.iter().any(|child| child.flex);
        let slot_heights = self.slot_heights(width, constraint.max.height);

        let mut total_height: u16 = 0;
        for (child, &slot) in self.children.iter_mut().zip(&slot_heights) {
            let child_constraint =
                TuiConstraint::new(TuiSize::new(width, 0), TuiSize::new(width, slot));
            let size = child.element.layout(child_constraint);
            // A flex child reserves its whole slot even if its content is
            // shorter, so later children sit below the reserved space.
            total_height = total_height.saturating_add(if child.flex { slot } else { size.height });
        }

        let height = if has_flex {
            constraint.max.height
        } else {
            total_height
        };
        TuiSize::new(width, constraint.constrain_height(height))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        let slot_heights = self.slot_heights(area.width, area.height);
        let mut remaining = area;
        for (child, &slot) in self.children.iter().zip(&slot_heights) {
            if remaining.is_empty() {
                break;
            }
            let (rect, rest) = remaining.split_top(slot);
            child.element.render(rect, buffer);
            remaining = rest;
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.children.iter().fold(0, |total, child| {
            total.saturating_add(child.element.desired_height(width))
        })
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        // Mirror `render`'s slot walk: ask each child for the cursor within its
        // rendered slot, then lift the slot's vertical offset back into `area`.
        let slot_heights = self.slot_heights(area.width, area.height);
        let mut remaining = area;
        for (child, &slot) in self.children.iter().zip(&slot_heights) {
            if remaining.is_empty() {
                break;
            }
            let (rect, rest) = remaining.split_top(slot);
            if let Some((x, y)) = child.element.cursor_position(rect) {
                return Some((x, y.saturating_add(rect.y - area.y)));
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
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        // Offer the event to each child in its rendered slot (mirroring
        // `render`'s stacking); the first child to handle it consumes it.
        // Children clipped past the available height see no events.
        let slot_heights = self.slot_heights(area.width, area.height);
        let mut remaining = area;
        for (child, &slot) in self.children.iter_mut().zip(&slot_heights) {
            if remaining.is_empty() {
                break;
            }
            let (rect, rest) = remaining.split_top(slot);
            if child.element.dispatch_event(event, rect, ctx, app) {
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
