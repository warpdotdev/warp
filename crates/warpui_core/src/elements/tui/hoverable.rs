//! [`TuiHoverable`]: wraps a child, tracks pointer-over state on a caller-owned
//! handle, and runs a click callback — the TUI mirror of the GUI's `Hoverable`,
//! reusing the same [`MouseStateHandle`]/[`MouseState`] so hover *and* click
//! gestures live on the state-owning element (the TUI's `TuiEventHandler` only
//! exposes raw key events).
//!
//! # Construction
//! The composing view owns a [`MouseStateHandle`] (created once and reused
//! across renders, since the element tree is rebuilt every frame), reads
//! [`MouseState::is_hovered`] at composition time to pick styles, and wraps
//! the element with [`TuiHoverable::new`], registering a click handler via
//! [`on_click`](TuiHoverable::on_click). Layout, render, height, and cursor
//! are transparent — they delegate to the wrapped child.
//!
//! # Dispatch policy
//! On [`MouseMoved`](TuiEvent::MouseMoved) the pointer position is compared
//! against this element's area; a hover transition is recorded on the handle
//! and queues a notification so the owning view re-renders. Mouse moves are
//! never consumed, so sibling hoverables observe their own transitions from
//! the same event. Other events are offered to the child first; clicks use the
//! GUI's press-then-release pairing: an unconsumed
//! [`LeftMouseDown`](TuiEvent::LeftMouseDown) inside the area arms a pending
//! click (recorded on the shared state, so [`MouseState::is_clicked`] styling
//! works) and is consumed; the following [`LeftMouseUp`](TuiEvent::LeftMouseUp)
//! disarms it, running the click handler only when released inside the area.
//! (Hover delays and the other [`MouseState`] fields are unused.)

use std::sync::MutexGuard;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::elements::{MouseState, MouseStateHandle};
use crate::AppContext;

type ClickCallback = Box<dyn FnMut(&mut TuiEventContext, &AppContext)>;

pub struct TuiHoverable {
    child: Box<dyn TuiElement>,
    state: MouseStateHandle,
    on_click: Option<ClickCallback>,
}

impl TuiHoverable {
    /// Wraps `child`, recording hover transitions on `state`.
    pub fn new(state: MouseStateHandle, child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            state,
            on_click: None,
        }
    }

    /// Registers `callback` to run on a left click — a `LeftMouseDown` that
    /// reaches this element unhandled by the child, followed by a
    /// `LeftMouseUp`, both within this element's area.
    pub fn on_click(
        mut self,
        callback: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(callback));
        self
    }

    /// Locks and returns the shared mouse state.
    fn state(&self) -> MutexGuard<'_, MouseState> {
        self.state.lock().unwrap()
    }
}

impl TuiElement for TuiHoverable {
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
        let child_handled = self.child.dispatch_event(event, area, event_ctx, ctx, app);

        if let TuiEvent::MouseMoved { position, .. } = event {
            let is_hovered = area.contains_point(*position);
            let mut state = self.state();
            if is_hovered != state.is_hovered() {
                state.is_hovered = is_hovered;
                drop(state);
                event_ctx.notify();
            }
            // Mouse moves are never consumed so sibling hoverables can track
            // their own transitions from the same event.
            return false;
        }

        if child_handled {
            return true;
        }

        match event {
            // Press inside the area: arm the pending click.
            TuiEvent::LeftMouseDown {
                position,
                click_count,
                ..
            } if self.on_click.is_some() && area.contains_point(*position) => {
                self.state().set_click_count(Some(*click_count));
                event_ctx.notify();
                true
            }
            // Release while armed: disarm, and fire only when released inside
            // the area (a release elsewhere cancels the click, as in the GUI).
            TuiEvent::LeftMouseUp { position, .. } if self.state().is_clicked() => {
                self.state().set_click_count(None);
                event_ctx.notify();
                if area.contains_point(*position) {
                    if let Some(on_click) = self.on_click.as_mut() {
                        on_click(event_ctx, app);
                    }
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "hoverable_tests.rs"]
mod tests;
