//! [`TuiAnimated`]: a timed animation — a repaint cadence plus a closure that
//! builds the current frame.

use std::time::Duration;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiSize,
};
use crate::AppContext;

/// An element that animates: every render requests a repaint after
/// `repaint_interval`, and every layout pass re-invokes `build_frame` to
/// produce the child shown by that pass.
///
/// Repaints re-run layout and paint over the cached element tree without
/// re-rendering any view, so `build_frame` is what keeps the content current.
///
/// The repaint cycle is self-sustaining and self-terminating: each render
/// requests the next repaint, and requests stop as soon as the element leaves
/// the painted tree. Multiple animated elements coalesce to the earliest
/// deadline per frame ([`TuiPaintContext::repaint_after`]).
///
/// `build_frame` should be cheap — it runs on every pass, including passes
/// caused by other elements' repaints — and should only build and return the
/// child.
///
/// # When not to use it
/// - Content that changes on *events* rather than time: re-render the view
///   (`ctx.notify()`).
/// - Fixed content whose *paint output* derives from the clock (e.g.
///   [`TuiShimmeringText`](super::TuiShimmeringText)'s colors): implement
///   [`TuiElement::render`] to read the clock and call
///   [`TuiPaintContext::repaint_after`] directly — no rebuild needed.
pub struct TuiAnimated {
    /// How long after each render the next repaint is requested.
    repaint_interval: Duration,
    /// Builds the current frame; run at the start of every layout pass.
    build_frame: Box<dyn Fn() -> Box<dyn TuiElement>>,
    /// The frame built by the most recent layout pass.
    child: Option<Box<dyn TuiElement>>,
}

impl TuiAnimated {
    /// An animated element repainting every `repaint_interval`, with
    /// `build_frame` producing the frame shown by each pass.
    pub fn new(
        repaint_interval: Duration,
        build_frame: impl Fn() -> Box<dyn TuiElement> + 'static,
    ) -> Self {
        Self {
            repaint_interval,
            build_frame: Box::new(build_frame),
            child: None,
        }
    }
}

impl TuiElement for TuiAnimated {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child
            .insert((self.build_frame)())
            .layout(constraint, ctx, app)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        if let Some(child) = &self.child {
            child.render(area, buffer, ctx);
        }
        ctx.repaint_after(self.repaint_interval);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
        self.child
            .as_ref()
            .and_then(|child| child.cursor_position(area, ctx))
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        if let Some(child) = &mut self.child {
            child.present(ctx);
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
        self.child
            .as_mut()
            .is_some_and(|child| child.dispatch_event(event, area, event_ctx, ctx, app))
    }
}

#[cfg(test)]
#[path = "animated_tests.rs"]
mod tests;
