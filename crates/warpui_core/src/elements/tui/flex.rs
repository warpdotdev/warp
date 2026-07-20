//! [`TuiFlex`]: a stack that lays its children out along a main [`Axis`] —
//! top-to-bottom for [`TuiFlex::column`], left-to-right for [`TuiFlex::row`] —
//! mirroring the GUI's `Flex` element at terminal-cell granularity.
//!
//! # Construction
//! Start from [`TuiFlex::column`] / [`TuiFlex::row`] (or [`TuiFlex::new`] with
//! an explicit [`Axis`]) and append boxed children (see [`TuiElement::finish`])
//! with [`child`](TuiFlex::child) (fixed main-axis extent) or
//! [`flex_child`](TuiFlex::flex_child) (fills leftover main-axis extent). The
//! [`with_spacing`](TuiFlex::with_spacing) builder inserts a fixed number of
//! blank cells between adjacent children.
//! The
//! [`TuiParentElement`](super::TuiParentElement) trait's `with_child` /
//! `with_children` / `add_child` / `add_children` also work and add fixed
//! children.
//!
//! # Layout policy
//! Every child is offered the flex's full cross-axis extent to lay out
//! against, but the flex itself sizes its cross axis to its largest child,
//! clamped to the constraint — so a tight cross-axis constraint still forces
//! the flex to fill it. This is the same content-sized cross-axis policy as
//! the GUI `Flex` (and Flutter).
//!
//! [`with_cross_axis_alignment`](TuiFlex::with_cross_axis_alignment) controls
//! where children land along the cross axis, mirroring the GUI's
//! [`CrossAxisAlignment`]: `Start` (default) anchors children at the cross
//! start, `Center` / `End` position each child's measured cross extent within
//! its slot, and `Stretch` forces children — and the flex itself — to fill the
//! offered cross extent (e.g. a full-width background banner).
//!
//! Along the main axis, each fixed child is laid out against the remaining
//! extent (loose) and takes its natural size; children are packed from the
//! start with the configured spacing, and children that fall past the
//! available extent are clipped.
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
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiRect, TuiRectExt, TuiScreenPoint,
    TuiScreenPosition, TuiSize,
};
use crate::AppContext;
use crate::elements::{Axis, CrossAxisAlignment};

/// A child of a [`TuiFlex`] plus whether it fills leftover main-axis space.
struct FlexChild {
    element: Box<dyn TuiElement>,
    flex: bool,
}

pub struct TuiFlex {
    axis: Axis,
    children: Vec<FlexChild>,
    /// Where children land along the cross axis (see
    /// [`with_cross_axis_alignment`](Self::with_cross_axis_alignment)).
    cross_axis_alignment: CrossAxisAlignment,
    /// Blank cells inserted between adjacent children along the main axis.
    spacing: u16,
    /// Sizes returned by each child's `layout()` call; populated during layout
    /// so `render`, `cursor_position`, and `dispatch_event` have consistent slot
    /// information.
    child_sizes: Vec<TuiSize>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiFlex {
    pub fn new(axis: Axis) -> Self {
        Self {
            axis,
            children: Vec::new(),
            cross_axis_alignment: CrossAxisAlignment::Start,
            spacing: 0,
            child_sizes: Vec::new(),
            size: None,
            origin: None,
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

    /// Inserts `spacing` blank cells between adjacent children.
    pub fn with_spacing(mut self, spacing: u16) -> Self {
        self.spacing = spacing;
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

    /// Sets where children land along the cross axis, mirroring the GUI
    /// `Flex`'s method of the same name. `Stretch` additionally makes the flex
    /// (and its children) fill the offered cross extent instead of sizing to
    /// content.
    pub fn with_cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.cross_axis_alignment = alignment;
        self
    }

    /// The main-axis component of `size`.
    fn main_extent(axis: Axis, size: TuiSize) -> u16 {
        match axis {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }

    /// The cross-axis component of `size`.
    fn cross_extent(axis: Axis, size: TuiSize) -> u16 {
        match axis {
            Axis::Vertical => size.width,
            Axis::Horizontal => size.height,
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

    /// Returns the child slots implied by the last layout pass: the leading
    /// main-axis extent of each laid-out child, packed from the start of
    /// `area`, stopping once the area is exhausted. Shared by `render`,
    /// `cursor_position`, and `dispatch_event` so paint and hit-test geometry
    /// cannot drift.
    fn child_slots(
        axis: Axis,
        area: TuiRect,
        child_sizes: &[TuiSize],
        spacing: u16,
    ) -> impl Iterator<Item = TuiRect> + '_ {
        child_sizes
            .iter()
            .enumerate()
            .scan(area, move |remaining, (index, size)| {
                if remaining.is_empty() {
                    return None;
                }
                let (slot, mut rest) =
                    Self::split_main(axis, *remaining, Self::main_extent(axis, *size));
                if index + 1 < child_sizes.len() {
                    (_, rest) = Self::split_main(axis, rest, spacing);
                }
                *remaining = rest;
                Some(slot)
            })
    }

    /// Clamps a main-axis extent into the constraint's main-axis bounds.
    fn constrain_main(axis: Axis, constraint: TuiConstraint, extent: u16) -> u16 {
        match axis {
            Axis::Vertical => constraint.constrain_height(extent),
            Axis::Horizontal => constraint.constrain_width(extent),
        }
    }

    /// Clamps a cross-axis extent into the constraint's cross-axis bounds.
    fn constrain_cross(axis: Axis, constraint: TuiConstraint, extent: u16) -> u16 {
        match axis {
            Axis::Vertical => constraint.constrain_width(extent),
            Axis::Horizontal => constraint.constrain_height(extent),
        }
    }

    /// The minimum cross extent handed to children: `Stretch` tightens the
    /// cross constraint so children fill it; other alignments leave it loose.
    fn child_cross_min(&self, cross: u16) -> u16 {
        match self.cross_axis_alignment {
            CrossAxisAlignment::Stretch => cross,
            CrossAxisAlignment::Start | CrossAxisAlignment::Center | CrossAxisAlignment::End => 0,
        }
    }

    /// The cross extent the flex reports for itself: `Stretch` fills the
    /// offered extent; otherwise the largest child's, clamped to the
    /// constraint.
    fn reported_cross(&self, constraint: TuiConstraint, cross: u16, cross_max: u16) -> u16 {
        match self.cross_axis_alignment {
            CrossAxisAlignment::Stretch => cross,
            CrossAxisAlignment::Start | CrossAxisAlignment::Center | CrossAxisAlignment::End => {
                Self::constrain_cross(self.axis, constraint, cross_max)
            }
        }
    }

    /// The rect a child occupies within its main-axis `slot`, positioned along
    /// the cross axis per the alignment. `Stretch` keeps the full slot;
    /// other alignments use the child's measured cross extent.
    /// Associated (not `&self`) so `dispatch_event` can call it while
    /// `children` is mutably borrowed.
    fn child_rect_for(
        axis: Axis,
        alignment: CrossAxisAlignment,
        slot: TuiRect,
        child_size: TuiSize,
    ) -> TuiRect {
        // The cross axis is horizontal for a column and vertical for a row.
        let (slot_cross, child_cross) = match axis {
            Axis::Vertical => (slot.width, child_size.width.min(slot.width)),
            Axis::Horizontal => (slot.height, child_size.height.min(slot.height)),
        };
        let offset = match alignment {
            CrossAxisAlignment::Stretch => return slot,
            CrossAxisAlignment::Start => 0,
            CrossAxisAlignment::Center => slot_cross.saturating_sub(child_cross) / 2,
            CrossAxisAlignment::End => slot_cross.saturating_sub(child_cross),
        };
        match axis {
            Axis::Vertical => TuiRect::new(
                slot.x.saturating_add(offset),
                slot.y,
                child_cross,
                slot.height,
            ),
            Axis::Horizontal => TuiRect::new(
                slot.x,
                slot.y.saturating_add(offset),
                slot.width,
                child_cross,
            ),
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
        // Children lay out against the full offered cross-axis extent; the
        // flex itself reports its largest child's cross extent, clamped to the
        // constraint (or the full extent under `Stretch`).
        let cross = match axis {
            Axis::Vertical => constraint.constrain_width(constraint.max.width),
            Axis::Horizontal => constraint.constrain_height(constraint.max.height),
        };
        let cross_min = self.child_cross_min(cross);
        let offered_main = Self::main_extent(axis, constraint.max);
        let has_flex = self.children.iter().any(|c| c.flex);
        self.child_sizes.clear();
        let mut cross_max: u16 = 0;

        if !has_flex {
            // No flex children: give each child the remaining main-axis extent
            // (loose) and sum the actual extents.
            let mut total_main: u16 = 0;
            for (index, child) in self.children.iter_mut().enumerate() {
                if index > 0 {
                    total_main = total_main.saturating_add(self.spacing);
                }
                let remaining = offered_main.saturating_sub(total_main);
                let child_constraint = TuiConstraint::new(
                    Self::size_of(axis, 0, cross_min),
                    Self::size_of(axis, remaining, cross),
                );
                let size = child.element.layout(child_constraint, ctx, app);
                total_main = total_main.saturating_add(Self::main_extent(axis, size));
                cross_max = cross_max.max(Self::cross_extent(axis, size));
                self.child_sizes.push(size);
            }
            let size = Self::size_of(
                axis,
                Self::constrain_main(axis, constraint, total_main),
                self.reported_cross(constraint, cross, cross_max),
            );
            self.size = Some(size);
            return size;
        }

        // Flex children: two passes.
        // Pass 1 — lay out fixed children to measure their total main-axis extent.
        let mut fixed_sizes: Vec<Option<TuiSize>> = Vec::with_capacity(self.children.len());
        let mut total_fixed = self
            .spacing
            .saturating_mul(self.children.len().saturating_sub(1) as u16);
        for child in &mut self.children {
            if child.flex {
                fixed_sizes.push(None);
            } else {
                let remaining = offered_main.saturating_sub(total_fixed);
                let child_constraint = TuiConstraint::new(
                    Self::size_of(axis, 0, cross_min),
                    Self::size_of(axis, remaining, cross),
                );
                let size = child.element.layout(child_constraint, ctx, app);
                total_fixed = total_fixed.saturating_add(Self::main_extent(axis, size));
                cross_max = cross_max.max(Self::cross_extent(axis, size));
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
                // Tight along the main axis so the child fills its slot; the
                // cross axis stays as for fixed children (loose, or tight
                // under `Stretch`).
                let child_constraint = TuiConstraint::new(
                    Self::size_of(axis, slot, cross_min),
                    Self::size_of(axis, slot, cross),
                );
                let child_size = child.element.layout(child_constraint, ctx, app);
                cross_max = cross_max.max(Self::cross_extent(axis, child_size));
                Self::size_of(axis, slot, Self::cross_extent(axis, child_size))
            } else {
                maybe_size.expect("fixed child was measured in pass 1")
            };
            self.child_sizes.push(size);
        }
        let size = Self::size_of(
            axis,
            offered_main,
            self.reported_cross(constraint, cross, cross_max),
        );
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        for child in &mut self.children {
            child.element.after_layout(ctx, app);
        }
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        let area = TuiRect::new(0, 0, size.width, size.height);
        let axis = self.axis;
        let alignment = self.cross_axis_alignment;
        for ((child, size), slot) in
            self.children
                .iter_mut()
                .zip(&self.child_sizes)
                .zip(Self::child_slots(
                    axis,
                    area,
                    &self.child_sizes,
                    self.spacing,
                ))
        {
            let rect = Self::child_rect_for(axis, alignment, slot, *size);
            child.element.render(
                origin.offset(i32::from(rect.x), i32::from(rect.y)),
                surface,
                ctx,
            );
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for child in &mut self.children {
            child.element.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.children.iter_mut().fold(false, |handled, child| {
            child.element.dispatch_event(event, event_ctx, app) || handled
        })
    }
}

#[cfg(test)]
#[path = "flex_tests.rs"]
mod tests;
