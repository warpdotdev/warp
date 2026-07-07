//! [`TuiLiveElement`]: a wrapper that requests a repaint at a fixed interval,
//! the TUI mirror of the GUI's [`LiveElement`](crate::elements::LiveElement).
//!
//! Wrap content that changes over time (spinners, elapsed-duration counters)
//! so it repaints on its own cadence. The repaint cycle is self-sustaining:
//! each `render` requests the next repaint, and it stops as soon as the
//! element is no longer part of the painted tree.

use std::time::Duration;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiSize,
};
use crate::AppContext;

pub struct TuiLiveElement {
    child: Box<dyn TuiElement>,
    repaint_interval: Duration,
}

impl TuiLiveElement {
    /// Wraps `child` so every paint requests another repaint after
    /// `repaint_interval`.
    pub fn new(child: Box<dyn TuiElement>, repaint_interval: Duration) -> Self {
        Self {
            child,
            repaint_interval,
        }
    }
}

impl TuiElement for TuiLiveElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child.layout(constraint, ctx, app)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        self.child.render(area, buffer, ctx);
        ctx.repaint_after(self.repaint_interval);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        self.child.cursor_position(area, ctx)
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
        self.child.dispatch_event(event, area, event_ctx, ctx, app)
    }
}
