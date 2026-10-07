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
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPoint, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
    TuiZIndex,
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

    /// Boxes this element as a scrollable trait object, mirroring the GUI's
    /// `NewScrollableElement::finish_scrollable`. [`TuiElement::finish`] can't
    /// be used to build a [`TuiScrollable`] child because it erases to
    /// `dyn TuiElement`, losing the scroll interface.
    fn finish_scrollable(self) -> Box<dyn TuiScrollableElement>
    where
        Self: 'static + Sized,
    {
        Box::new(self)
    }
}

/// Wraps a [`TuiScrollableElement`], capturing wheel events over the child's
/// area and translating them into scroll requests. Layout, render, cursor, and
/// inner event dispatch are transparent — only the wheel is intercepted, and
/// only when the child did not already handle the event.
pub struct TuiScrollable {
    child: Box<dyn TuiScrollableElement>,
    propagate_mousewheel_if_not_handled: bool,
    child_max_z_index: Option<TuiZIndex>,
}

impl TuiScrollable {
    /// Wraps `child` so wheel events over its area scroll it.
    pub fn new(child: Box<dyn TuiScrollableElement>) -> Self {
        Self {
            child,
            propagate_mousewheel_if_not_handled: false,
            child_max_z_index: None,
        }
    }

    /// Propagates in-bounds wheel events when they do not change scroll state.
    pub fn with_propagate_mousewheel_if_not_handled(mut self, propagate: bool) -> Self {
        self.propagate_mousewheel_if_not_handled = propagate;
        self
    }

    /// Returns whether `position` is inside the visible child bounds without
    /// being covered by a layer painted above the child's own descendants.
    fn is_mouse_over_child(&self, position: TuiPoint, event_ctx: &TuiEventContext<'_>) -> bool {
        let Some((origin, size, z_index)) = self
            .origin()
            .zip(self.size())
            .zip(self.child_max_z_index)
            .map(|((origin, size), z_index)| (origin, size, z_index))
        else {
            return false;
        };
        event_ctx.hit_test_with_z_index(origin, size, z_index, position)
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

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.child.render(origin, surface, ctx);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
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
        if self.child.dispatch_event(event, event_ctx, app) {
            return true;
        }
        let Some(size) = self.size() else {
            return false;
        };
        match event {
            TuiEvent::ScrollWheel {
                position, delta, ..
            } if self.is_mouse_over_child(*position, event_ctx) => {
                let scrolled = self
                    .child
                    .scroll_by_rows(-(delta.1 * WHEEL_STEP), usize::from(size.height));
                if scrolled {
                    event_ctx.notify();
                }
                scrolled || !self.propagate_mousewheel_if_not_handled
            }
            _ => false,
        }
    }
}
