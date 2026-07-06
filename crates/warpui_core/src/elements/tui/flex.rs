//! [`TuiFlex`]: a stack that lays its children out along a main [`Axis`] —
//! top-to-bottom for [`TuiFlex::column`], left-to-right for [`TuiFlex::row`] —
//! mirroring the GUI's `Flex` element at terminal-cell granularity.
//!
//! # Construction
//! Start from [`TuiFlex::column`] / [`TuiFlex::row`] (or [`TuiFlex::new`] with
//! an explicit [`Axis`]) and append boxed children (see [`TuiElement::finish`])
//! with [`child`](TuiFlex::child) (fixed main-axis extent) or
//! [`flex_child`](TuiFlex::flex_child) (fills leftover main-axis extent). The
//! [`TuiParentElement`](super::TuiParentElement) trait's `with_child` /
//! `with_children` / `add_child` / `add_children` also work and add fixed
//! children.
//!
//! # Layout policy
//! The flex fills the cross-axis extent it is offered (a column spans the
//! offered width, a row the offered height) and gives every child that same
//! cross-axis extent to lay out against. Wrap a flex in a
//! [`TuiConstrainedBox`](super::TuiConstrainedBox) to cap the cross axis, e.g.
//! a one-row status line inside a column.
//!
//! Along the main axis, each fixed child is laid out against the remaining
//! extent (loose) and takes its natural size; children are packed without gaps
//! from the start, and children that fall past the available extent are
//! clipped.
//!
//! A child added with [`flex_child`](TuiFlex::flex_child) instead *fills* the
//! main-axis extent left over after the fixed children, so content can be
//! docked at the far edge behind a flex spacer (a body above a bottom-docked
//! input, or a right-aligned status line). With at least one flex child the
//! flex fills the main-axis extent it is offered and splits the leftover
//! evenly across the flex children (any remainder going to the earlier ones).
//! With no flex children the flex's main-axis extent is the sum of its
//! children's, clamped to the constraint.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::elements::Axis;
use crate::AppContext;

/// A child of a [`TuiFlex`] plus whether it fills leftover main-axis space.
struct FlexChild {
    element: Box<dyn TuiElement>,
    flex: bool,
}

pub struct TuiFlex {
    axis: Axis,
    children: Vec<FlexChild>,
    /// Sizes returned by each child's `layout()` call; populated during layout
    /// so `render`, `cursor_position`, and `dispatch_event` have consistent slot
    /// information.
    child_sizes: Vec<TuiSize>,
}

impl TuiFlex {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            children: Vec::new(),
            child_sizes: Vec::new(),
        }
    }

    /// A flex stacking its children top-to-bottom.
    pub fn column() -> Self {
        Self::new(Axis::Vertical)
    }

    /// A flex packing its children left-to-right.
    pub fn row() -> Self {
        Self::new(Axis::Horizontal)
    }

    /// Appends a fixed child (boxed via [`TuiElement::finish`]), laid out
    /// against the remaining main-axis extent.
    pub fn child(mut self, child: Box<dyn TuiElement>) -> Self {
        self.children.push(FlexChild {
            element: child,
            flex: false,
        });
        self
    }

    /// Appends a child (boxed via [`TuiElement::finish`]) that fills the
    /// main-axis extent left over after the fixed children (shared evenly when
    /// there are several flex children).
    pub fn flex_child(mut self, child: Box<dyn TuiElement>) -> Self {
        self.children.push(FlexChild {
            element: child,
            flex: true,
        });
        self
    }

    /// The main-axis component of `size`.
    fn main_extent(axis: Axis, size: TuiSize) -> u16 {
        match axis {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }

    /// A size from main- and cross-axis extents.
    fn size_of(axis: Axis, main: u16, cross: u16) -> TuiSize {
        match axis {
            Axis::Vertical => TuiSize::new(cross, main),
            Axis::Horizontal => TuiSize::new(main, cross),
        }
    }

    /// Splits off the leading `extent` of `rect` along the main axis,
    /// returning `(slot, remainder)`.
    fn split_main(axis: Axis, rect: TuiRect, extent: u16) -> (TuiRect, TuiRect) {
        match axis {
            Axis::Vertical => rect.split_top(extent),
            Axis::Horizontal => rect.split_left(extent),
        }
    }

    /// Clamps a main-axis extent into the constraint's main-axis bounds.
    fn constrain_main(axis: Axis, constraint: TuiConstraint, extent: u16) -> u16 {
        match axis {
            Axis::Vertical => constraint.constrain_height(extent),
            Axis::Horizontal => constraint.constrain_width(extent),
        }
    }
}

/// Allows [`TuiParentElement`](super::TuiParentElement) (`with_child`,
/// `with_children`) to work on `TuiFlex`, adding fixed children.
impl Extend<Box<dyn TuiElement>> for TuiFlex {
    fn extend<I: IntoIterator<Item = Box<dyn TuiElement>>>(&mut self, iter: I) {
        self.children
            .extend(iter.into_iter().map(|element| FlexChild {
                element,
                flex: false,
            }));
    }
}

impl TuiElement for TuiFlex {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let axis = self.axis;
        // The flex fills the cross-axis extent it is offered.
        let cross = match axis {
            Axis::Vertical => constraint.constrain_width(constraint.max.width),
            Axis::Horizontal => constraint.constrain_height(constraint.max.height),
        };
        let offered_main = Self::main_extent(axis, constraint.max);
        let has_flex = self.children.iter().any(|c| c.flex);
        self.child_sizes.clear();

        if !has_flex {
            // No flex children: give each child the remaining main-axis extent
            // (loose) and sum the actual extents.
            let mut total_main: u16 = 0;
            for child in &mut self.children {
                let remaining = offered_main.saturating_sub(total_main);
                let child_constraint = TuiConstraint::loose(Self::size_of(axis, remaining, cross));
                let size = child.element.layout(child_constraint, ctx, app);
                total_main = total_main.saturating_add(Self::main_extent(axis, size));
                self.child_sizes.push(size);
            }
            return Self::size_of(
                axis,
                Self::constrain_main(axis, constraint, total_main),
                cross,
            );
        }

        // Flex children: two passes.
        // Pass 1 — lay out fixed children to measure their total main-axis extent.
        let mut fixed_sizes: Vec<Option<TuiSize>> = Vec::with_capacity(self.children.len());
        let mut total_fixed: u16 = 0;
        for child in &mut self.children {
            if child.flex {
                fixed_sizes.push(None);
            } else {
                let remaining = offered_main.saturating_sub(total_fixed);
                let child_constraint = TuiConstraint::loose(Self::size_of(axis, remaining, cross));
                let size = child.element.layout(child_constraint, ctx, app);
                total_fixed = total_fixed.saturating_add(Self::main_extent(axis, size));
                fixed_sizes.push(Some(size));
            }
        }
        // Pass 2 — distribute leftover evenly among the flex children (guaranteed
        // non-zero count because `has_flex` is true).
        let flex_count = self.children.iter().filter(|c| c.flex).count() as u16;
        let leftover = offered_main.saturating_sub(total_fixed);
        let base = leftover / flex_count;
        let remainder = leftover % flex_count;
        let mut flex_rank = 0u16;
        for (child, maybe_size) in self.children.iter_mut().zip(fixed_sizes) {
            let size = if child.flex {
                let slot = base + u16::from(flex_rank < remainder);
                flex_rank += 1;
                let slot_size = Self::size_of(axis, slot, cross);
                // Lay out with a tight extent so the child fills its slot.
                child
                    .element
                    .layout(TuiConstraint::tight(slot_size), ctx, app);
                slot_size
            } else {
                maybe_size.expect("fixed child was measured in pass 1")
            };
            self.child_sizes.push(size);
        }
        Self::size_of(axis, offered_main, cross)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        let mut remaining = area;
        for (child, size) in self.children.iter().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) =
                Self::split_main(self.axis, remaining, Self::main_extent(self.axis, *size));
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
            let (slot, rest) =
                Self::split_main(self.axis, remaining, Self::main_extent(self.axis, *size));
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
        // packing); the first child to handle it consumes it. Children clipped
        // past the available extent see no events.
        let axis = self.axis;
        let mut remaining = area;
        for (child, size) in self.children.iter_mut().zip(&self.child_sizes) {
            if remaining.is_empty() {
                break;
            }
            let (slot, rest) = Self::split_main(axis, remaining, Self::main_extent(axis, *size));
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
#[path = "flex_tests.rs"]
mod tests;
