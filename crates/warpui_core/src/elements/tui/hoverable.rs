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
//! Hover and click hit-test against the child's laid-out footprint (the size
//! returned by the most recent `layout`, anchored at the area's origin), not
//! the whole slot the parent assigned — so trailing blank space in a flex row
//! is not part of the target. On [`MouseMoved`](TuiEvent::MouseMoved) the
//! pointer position is compared against that footprint; a hover transition is
//! recorded on the handle and queues a notification so the owning view
//! re-renders. Mouse moves are never consumed, so sibling hoverables observe
//! their own transitions from the same event. Other events are offered to the
//! child first; clicks use the GUI's press-then-release pairing: an unconsumed
//! [`LeftMouseDown`](TuiEvent::LeftMouseDown) inside the footprint arms a
//! pending click (recorded on the shared state, so [`MouseState::is_clicked`]
//! styling works) and is consumed; the following
//! [`LeftMouseUp`](TuiEvent::LeftMouseUp) disarms it, running the click
//! handler only when released inside the footprint. (Hover delays and the
//! other [`MouseState`] fields are unused.)

use std::mem;
use std::sync::MutexGuard;

use super::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPoint, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
    TuiZIndex,
};
use crate::AppContext;
use crate::elements::{MouseState, MouseStateHandle};

type ClickCallback = Box<dyn for<'a> FnMut(&mut TuiEventContext<'a>, &AppContext)>;

pub struct TuiHoverable {
    child: Box<dyn TuiElement>,
    state: MouseStateHandle,
    on_click: Option<ClickCallback>,
    origin: Option<TuiScreenPoint>,
    child_max_z_index: Option<TuiZIndex>,
}

impl TuiHoverable {
    /// Wraps `child`, recording hover transitions on `state`.
    pub fn new(state: MouseStateHandle, child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            state,
            on_click: None,
            origin: None,
            child_max_z_index: None,
        }
    }

    /// Registers `callback` to run on a left click — a `LeftMouseDown` that
    /// reaches this element unhandled by the child, followed by a
    /// `LeftMouseUp`, both within this element's area.
    pub fn on_click(
        mut self,
        callback: impl for<'a> FnMut(&mut TuiEventContext<'a>, &AppContext) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(callback));
        self
    }

    /// Locks and returns the shared mouse state.
    fn state(&self) -> MutexGuard<'_, MouseState> {
        self.state.lock().unwrap()
    }

    /// Returns whether `position` is inside visible, uncovered child bounds.
    fn is_mouse_over_element(&self, position: TuiPoint, event_ctx: &TuiEventContext<'_>) -> bool {
        let Some((origin, size, z_index)) = self
            .origin
            .zip(self.size())
            .zip(self.child_max_z_index)
            .map(|((origin, size), z_index)| (origin, size, z_index))
        else {
            return false;
        };
        event_ctx.hit_test_with_z_index(origin, size, z_index, position)
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

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        self.child.render(origin, surface, ctx);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<TuiSize> {
        self.child.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
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
        let child_handled = self.child.dispatch_event(event, event_ctx, app);

        if let TuiEvent::MouseMoved {
            position,
            is_synthetic,
            ..
        } = event
        {
            let is_hovered = self.is_mouse_over_element(*position, event_ctx);
            let mut state = self.state();
            if is_hovered != state.is_hovered() {
                // Guard against layout<->hover feedback loops (mirrors the GUI
                // Hoverable): a synthetic move replayed after a redraw may flip
                // hover, which relayouts and replays another synthetic move; two
                // consecutive synthetic flips are suppressed to break the cycle.
                let was_synthetic =
                    mem::replace(&mut state.last_event_is_synthetic_hover, *is_synthetic);
                if *is_synthetic && was_synthetic {
                    return false;
                }

                state.is_hovered = is_hovered;
                drop(state);
                event_ctx.notify();
            }
            // Mouse moves are never consumed so sibling hoverables can track
            // their own transitions from the same event.
            return false;
        }
        // Any non-move event re-arms the synthetic-flip guard, mirroring the GUI.
        self.state().last_event_is_synthetic_hover = false;

        if child_handled {
            return true;
        }

        match event {
            // Press inside the footprint: arm the pending click.
            TuiEvent::LeftMouseDown {
                position,
                click_count,
                ..
            } if self.on_click.is_some() && self.is_mouse_over_element(*position, event_ctx) => {
                self.state().set_click_count(Some(*click_count));
                event_ctx.notify();
                true
            }
            // Release while armed: disarm, and fire only when released inside
            // the footprint (a release elsewhere cancels the click, as in the GUI).
            TuiEvent::LeftMouseUp { position, .. } if self.state().is_clicked() => {
                self.state().set_click_count(None);
                event_ctx.notify();
                if self.is_mouse_over_element(*position, event_ctx) {
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
