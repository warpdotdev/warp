//! [`TuiConstrainedBox`]: caps a single child's size on either axis.
//!
//! # Construction
//! Wrap a child with [`TuiConstrainedBox::new`] and cap either axis with
//! [`with_max_rows`](TuiConstrainedBox::with_max_rows) (height) and
//! [`with_max_cols`](TuiConstrainedBox::with_max_cols) (width). Either cap may
//! be left unset, in which case that axis passes through unchanged.
//!
//! # Layout policy
//! The box is otherwise transparent: it measures and paints its child within the
//! area it is given, but it shrinks the available `max` on each capped axis
//! first and clips the paint area to the cap. This is the TUI analog of the GUI
//! `ConstrainedBox`, letting a caller size a child (for example, pinning the
//! bottom input to at most six rows) without a bespoke layout element.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiSize,
};
use crate::AppContext;

pub struct TuiConstrainedBox {
    child: Box<dyn TuiElement>,
    max_rows: Option<u16>,
    max_cols: Option<u16>,
}

impl TuiConstrainedBox {
    pub fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            max_rows: None,
            max_cols: None,
        }
    }

    /// Caps the child's height to `rows` cells.
    pub fn with_max_rows(mut self, rows: u16) -> Self {
        self.max_rows = Some(rows);
        self
    }

    /// Caps the child's width to `cols` cells.
    pub fn with_max_cols(mut self, cols: u16) -> Self {
        self.max_cols = Some(cols);
        self
    }

    /// `constraint` with its `max` (and, where necessary, `min`) reduced so each
    /// capped axis honors the configured limit.
    fn cap_constraint(&self, constraint: TuiConstraint) -> TuiConstraint {
        let max_width = self
            .max_cols
            .map_or(constraint.max.width, |cols| constraint.max.width.min(cols));
        let max_height = self.max_rows.map_or(constraint.max.height, |rows| {
            constraint.max.height.min(rows)
        });
        let min = TuiSize::new(
            constraint.min.width.min(max_width),
            constraint.min.height.min(max_height),
        );
        TuiConstraint::new(min, TuiSize::new(max_width, max_height))
    }

    /// `area` clipped to the configured caps, anchored at the area's origin.
    fn capped_area(&self, area: TuiRect) -> TuiRect {
        let width = self
            .max_cols
            .map_or(area.width, |cols| area.width.min(cols));
        let height = self
            .max_rows
            .map_or(area.height, |rows| area.height.min(rows));
        TuiRect::new(area.x, area.y, width, height)
    }
}

impl TuiElement for TuiConstrainedBox {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child.layout(self.cap_constraint(constraint), ctx, app)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        self.child.render(self.capped_area(area), buffer, ctx);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        self.child.cursor_position(self.capped_area(area), ctx)
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        self.child
            .dispatch_event(event, self.capped_area(area), event_ctx, ctx, app)
    }
}

#[cfg(test)]
#[path = "constrained_box_tests.rs"]
mod tests;
