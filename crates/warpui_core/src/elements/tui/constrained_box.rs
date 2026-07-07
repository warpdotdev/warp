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
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
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

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.child.render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.child.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.child.origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, event_ctx, app)
    }
}

#[cfg(test)]
#[path = "constrained_box_tests.rs"]
mod tests;
