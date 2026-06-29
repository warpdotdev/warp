//! A reusable wheel-scroll wrapper for TUI elements that own a scroll position.
//!
//! Mirrors the GUI split between `NewScrollable` and `NewScrollableElement` for
//! child-owned scroll positions: the wrapped element owns its scroll *position*
//! and clamping (e.g. a virtualized list, which is the only thing that knows
//! item heights), while this wrapper owns wheel-event capture and translates
//! wheel deltas into scroll requests. The TUI stack intentionally omits the
//! GUI's clipped-scrollable mode for now; a future clipped adapter can implement
//! [`TuiScrollableElement`] without changing this wrapper.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::AppContext;

/// Logical rows scrolled per wheel notch.
const WHEEL_STEP: isize = 2;

/// A [`TuiElement`] that owns a scroll position and can be driven by
/// [`TuiScrollable`].
///
/// Implementors own the scroll state and its clamping (a virtualized list, for
/// example, is the only thing that knows item heights), so [`TuiScrollable`]
/// only has to capture wheel events and forward them here.
pub trait TuiScrollableElement: TuiElement {
    /// Scrolls by `rows` (negative scrolls toward the top) within a viewport of
    /// `viewport_height` rows. Returns whether the scroll position changed.
    fn scroll_by_rows(&mut self, rows: isize, viewport_height: usize) -> bool;
}

/// Wraps a [`TuiScrollableElement`], capturing wheel events over the child's
/// area and translating them into scroll requests. Layout, render, cursor, and
/// inner event dispatch are transparent — only the wheel is intercepted, and
/// only when the child did not already handle the event.
pub struct TuiScrollable {
    child: Box<dyn TuiScrollableElement>,
    propagate_mousewheel_if_not_handled: bool,
}

impl TuiScrollable {
    /// Wraps `child` so wheel events over its area scroll it.
    pub fn new(child: impl TuiScrollableElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            propagate_mousewheel_if_not_handled: false,
        }
    }

    /// Propagates in-bounds wheel events when they do not change scroll state.
    pub fn with_propagate_mousewheel_if_not_handled(mut self, propagate: bool) -> Self {
        self.propagate_mousewheel_if_not_handled = propagate;
        self
    }
}

impl TuiElement for TuiScrollable {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child.layout(constraint, ctx, app)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        self.child.render(area, buffer, ctx);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
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
        if self.child.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }
        match event {
            TuiEvent::ScrollWheel {
                position, delta, ..
            } if area.contains_point(*position) => {
                let scrolled = self
                    .child
                    .scroll_by_rows(-(delta.1 * WHEEL_STEP), usize::from(area.height));
                if scrolled {
                    event_ctx.notify();
                }
                scrolled || !self.propagate_mousewheel_if_not_handled
            }
            _ => false,
        }
    }
}
