//! [`TuiEventHandler`]: wraps a child element and runs callbacks for events the
//! child itself did not handle.
//!
//! # Construction
//! Wrap a child with [`TuiEventHandler::new`] and register handlers with
//! [`on_key`](TuiEventHandler::on_key), matching against the
//! [`Keystroke::key`](crate::keymap::Keystroke) string (e.g. `"enter"`,
//! `"a"`), and/or [`on_click`](TuiEventHandler::on_click) for left clicks
//! inside the element's area. Layout, render, height, and cursor are
//! transparent — they delegate to the wrapped child.
//!
//! # Dispatch policy
//! On [`dispatch_event`](TuiElement::dispatch_event) the event is offered to the
//! child first. If the child consumes it, dispatch stops. Otherwise, for a
//! `KeyDown` event, the first registered binding whose key matches is invoked
//! (with the event, the [`TuiEventContext`], and the [`AppContext`]) and the
//! event is reported handled; for a
//! [`LeftMouseDown`](TuiEvent::LeftMouseDown) whose position falls within this
//! element's area, the click handler is invoked and the event is reported
//! handled. Events matching no handler are left unhandled so ancestors can
//! react.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiRectExt, TuiSize,
};
use crate::AppContext;

type KeyCallback = Box<dyn FnMut(&TuiEvent, &mut TuiEventContext, &AppContext)>;
type ClickCallback = Box<dyn FnMut(&mut TuiEventContext, &AppContext)>;

struct KeyBinding {
    key: String,
    callback: KeyCallback,
}

pub struct TuiEventHandler {
    child: Box<dyn TuiElement>,
    bindings: Vec<KeyBinding>,
    on_click: Option<ClickCallback>,
}

impl TuiEventHandler {
    pub fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            bindings: Vec::new(),
            on_click: None,
        }
    }

    /// Registers `callback` to run when a `KeyDown` whose key equals `key`
    /// reaches this element unhandled by the child.
    pub fn on_key(
        mut self,
        key: impl Into<String>,
        callback: impl FnMut(&TuiEvent, &mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.bindings.push(KeyBinding {
            key: key.into(),
            callback: Box::new(callback),
        });
        self
    }

    /// Registers `callback` to run when a `LeftMouseDown` within this element's
    /// area reaches this element unhandled by the child.
    pub fn on_click(
        mut self,
        callback: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(callback));
        self
    }
}

impl TuiElement for TuiEventHandler {
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

        if let TuiEvent::KeyDown { keystroke, .. } = event {
            for binding in &mut self.bindings {
                if binding.key == keystroke.key {
                    (binding.callback)(event, event_ctx, app);
                    return true;
                }
            }
        }

        if let (TuiEvent::LeftMouseDown { position, .. }, Some(on_click)) =
            (event, self.on_click.as_mut())
        {
            if area.contains_point(*position) {
                on_click(event_ctx, app);
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
#[path = "event_handler_tests.rs"]
mod tests;
