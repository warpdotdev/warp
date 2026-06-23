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
//! event is reported handled. If no exact binding matches, a key-down fallback
//! (registered with [`on_key_fallback`](TuiEventHandler::on_key_fallback)) is
//! offered the event and decides whether to consume it — this lets a view
//! consume arbitrary printable input (inspecting the keystroke's modifiers)
//! without enumerating every key. Events left unconsumed propagate to ancestors.

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiPresentationContext, TuiRect, TuiSize,
};
use crate::{AppContext, Event};

type KeyCallback = Box<dyn FnMut(&Event, &mut TuiEventContext, &AppContext)>;

/// A fallback invoked for `KeyDown` events that matched no exact binding.
/// Returns `true` to consume the event, `false` to let it propagate.
type FallbackCallback = Box<dyn FnMut(&Event, &mut TuiEventContext, &AppContext) -> bool>;

struct KeyBinding {
    key: String,
    callback: KeyCallback,
}

pub struct TuiEventHandler {
    child: Box<dyn TuiElement>,
    bindings: Vec<KeyBinding>,
    fallback: Option<FallbackCallback>,
}

impl TuiEventHandler {
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            bindings: Vec::new(),
            fallback: None,
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

    /// Registers a fallback for `KeyDown` events that matched no exact binding.
    /// The callback inspects the event (including modifier state and produced
    /// `chars`) and returns whether it consumed the event.
    pub fn on_key_fallback(
        mut self,
        callback: impl FnMut(&Event, &mut TuiEventContext, &AppContext) -> bool + 'static,
    ) -> Self {
        self.fallback = Some(Box::new(callback));
        self
    }
}

impl TuiElement for TuiEventHandler {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        self.child.layout(constraint, ctx)
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
        event: &Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if self.child.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        if let Event::KeyDown { keystroke, .. } = event {
            for binding in &mut self.bindings {
                if binding.key == keystroke.key {
                    (binding.callback)(event, event_ctx, app);
                    return true;
                }
            }
            if let Some(fallback) = &mut self.fallback {
                return fallback(event, event_ctx, app);
            }
        }

        false
    }
}

#[cfg(test)]
#[path = "event_handler_tests.rs"]
mod tests;
