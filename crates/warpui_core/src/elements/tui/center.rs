//! [`TuiCenter`]: centers a single child within the area it is given.
//!
//! # Layout policy
//! `TuiCenter` claims the whole area it is offered, measures its child loosely
//! within that area (remembering the child's size), and paints the child into a
//! sub-rectangle centered on both axes. Pair it with a full-width child whose
//! own content is centered (e.g. a [`TuiColumn`](super::TuiColumn) of
//! [`TuiText::centered`](super::TuiText::centered) rows) to center a block of
//! text on screen: `TuiCenter` centers the block vertically, and the child
//! widths fill the area so the text alignment centers it horizontally.
//!
//! The child's measured size is cached during [`layout`](TuiElement::layout) so
//! [`render`](TuiElement::render) and [`dispatch_event`](TuiElement::dispatch_event)
//! target the same centered rectangle; the layout pass always runs before either
//! (the presenter arranges before painting; the runtime lays out before
//! dispatching).

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiPresentationContext, TuiRect, TuiSize,
};
use crate::{AppContext, Event};

pub struct TuiCenter {
    child: Box<dyn TuiElement>,
    /// The child's size as measured by the most recent [`layout`](TuiElement::layout).
    child_size: TuiSize,
}

impl TuiCenter {
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            child_size: TuiSize::ZERO,
        }
    }

    /// The sub-rectangle of `area` the measured child is centered into, clamped
    /// so it never exceeds `area`.
    fn centered_rect(&self, area: TuiRect) -> TuiRect {
        let width = self.child_size.width.min(area.width);
        let height = self.child_size.height.min(area.height);
        let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
        let y = area
            .y
            .saturating_add(area.height.saturating_sub(height) / 2);
        TuiRect::new(x, y, width, height)
    }
}

impl TuiElement for TuiCenter {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        // Measure the child loosely within the available area and remember its
        // size so `render`/`dispatch_event` center the same rectangle, then
        // claim the whole area.
        self.child_size = self.child.layout(TuiConstraint::loose(constraint.max));
        constraint.max
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        if area.is_empty() {
            return;
        }
        self.child.render(self.centered_rect(area), buffer);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.child.desired_height(width)
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        if area.is_empty() {
            return false;
        }
        let rect = self.centered_rect(area);
        self.child.dispatch_event(event, rect, ctx, app)
    }
}

#[cfg(test)]
#[path = "center_tests.rs"]
mod tests;
