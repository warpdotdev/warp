//! [`TuiEventHandler`]: wraps a child element and runs callbacks for keys the
//! child itself did not handle.
//!
//! # Construction
//! Wrap a child with [`TuiEventHandler::new`] and register handlers with
//! [`on_key`](TuiEventHandler::on_key), matching against the
//! [`Keystroke::key`](crate::keymap::Keystroke) string (e.g. `"enter"`,
//! `"a"`). Layout, render, height, and cursor are transparent — they delegate to
//! the wrapped child.
//!
//! # Dispatch policy
//! On [`dispatch_event`](TuiElement::dispatch_event) the event is offered to the
//! child first. If the child consumes it, dispatch stops. Otherwise, for a
//! `KeyDown` event, the first registered binding whose key matches is invoked
//! (with the event, the [`TuiEventContext`], and the [`AppContext`]) and the
//! event is reported handled. Events matching no binding are left unhandled so
//! ancestors can react.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiPresentationContext, TuiRect, TuiSize,
};
use crate::{AppContext, Event};

type KeyCallback = Box<dyn FnMut(&Event, &mut TuiEventContext, &AppContext)>;

struct KeyBinding {
    key: String,
    callback: KeyCallback,
}

pub struct TuiEventHandler {
    child: Box<dyn TuiElement>,
    bindings: Vec<KeyBinding>,
}

impl TuiEventHandler {
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            bindings: Vec::new(),
        }
    }

    /// Registers `callback` to run when a `KeyDown` whose key equals `key`
    /// reaches this element unhandled by the child.
    pub fn on_key(
        mut self,
        key: impl Into<String>,
        callback: impl FnMut(&Event, &mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.bindings.push(KeyBinding {
            key: key.into(),
            callback: Box::new(callback),
        });
        self
    }
}

impl TuiElement for TuiEventHandler {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        self.child.layout(constraint)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        self.child.render(area, buffer);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.child.desired_height(width)
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        self.child.cursor_position(area)
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
        if self.child.dispatch_event(event, area, ctx, app) {
            return true;
        }

        if let Event::KeyDown { keystroke, .. } = event {
            for binding in &mut self.bindings {
                if binding.key == keystroke.key {
                    (binding.callback)(event, ctx, app);
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
#[path = "event_handler_tests.rs"]
mod tests;
