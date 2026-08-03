//! [`TuiEventHandler`]: wraps a child element and runs callbacks for keys the
//! child itself did not handle. (Mouse gestures — clicks and hover — live on
//! [`TuiHoverable`](super::TuiHoverable), mirroring the GUI split between
//! `EventHandler` and `Hoverable`.)
//!
//! # Construction
//! Wrap a child with [`TuiEventHandler::new`]. [`on_key`](TuiEventHandler::on_key)
//! matches against the
//! [`Keystroke::key`](crate::keymap::Keystroke) string (e.g. `"enter"`,
//! `"a"`), while
//! [`on_modifier_key_changed`](TuiEventHandler::on_modifier_key_changed)
//! observes physical modifier press/release events. Layout, render, height, and
//! cursor are transparent — they delegate to the wrapped child.
//!
//! # Dispatch policy
//! On [`dispatch_event`](TuiElement::dispatch_event) the event is offered to the
//! child first. If the child consumes it, dispatch stops. Otherwise, for a
//! `KeyDown` event, the first registered binding whose key matches is invoked
//! (with the event, the [`TuiEventContext`], and the [`AppContext`]) and the
//! event is reported handled. A registered modifier callback can explicitly
//! choose whether its event propagates to ancestors. Events matching no handler
//! are left unhandled so ancestors can react.

use super::{
    TuiConstraint, TuiDispatchEventResult, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition,
    TuiSize,
};
use crate::AppContext;
use crate::event::KeyState;
use crate::platform::keyboard::KeyCode;

type KeyCallback = Box<dyn for<'a> FnMut(&TuiEvent, &mut TuiEventContext<'a>, &AppContext)>;
type ModifierKeyChangedCallback = Box<
    dyn for<'a> FnMut(
        KeyCode,
        KeyState,
        &mut TuiEventContext<'a>,
        &AppContext,
    ) -> TuiDispatchEventResult,
>;

struct KeyBinding {
    key: String,
    callback: KeyCallback,
}

pub struct TuiEventHandler {
    child: Box<dyn TuiElement>,
    bindings: Vec<KeyBinding>,
    modifier_key_changed_callback: Option<ModifierKeyChangedCallback>,
}

impl TuiEventHandler {
    pub fn new(child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            bindings: Vec::new(),
            modifier_key_changed_callback: None,
        }
    }

    /// Registers `callback` to run when a `KeyDown` whose key equals `key`
    /// reaches this element unhandled by the child.
    pub fn on_key(
        mut self,
        key: impl Into<String>,
        callback: impl for<'a> FnMut(&TuiEvent, &mut TuiEventContext<'a>, &AppContext) + 'static,
    ) -> Self {
        self.bindings.push(KeyBinding {
            key: key.into(),
            callback: Box::new(callback),
        });
        self
    }

    /// Registers a child-first callback for physical modifier press/release
    /// events. The callback explicitly chooses whether ancestors may continue
    /// handling the event.
    pub fn on_modifier_key_changed(
        mut self,
        callback: impl for<'a> FnMut(
            KeyCode,
            KeyState,
            &mut TuiEventContext<'a>,
            &AppContext,
        ) -> TuiDispatchEventResult
        + 'static,
    ) -> Self {
        self.modifier_key_changed_callback = Some(Box::new(callback));
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

        if let TuiEvent::KeyDown { keystroke, .. } = event {
            for binding in &mut self.bindings {
                if binding.key == keystroke.key {
                    (binding.callback)(event, event_ctx, app);
                    return true;
                }
            }
        }
        if let TuiEvent::ModifierKeyChanged { key_code, state } = event
            && let Some(callback) = &mut self.modifier_key_changed_callback
        {
            return match callback(*key_code, *state, event_ctx, app) {
                TuiDispatchEventResult::PropagateToParent => false,
                TuiDispatchEventResult::StopPropagation => true,
            };
        }

        false
    }
}

#[cfg(test)]
#[path = "event_handler_tests.rs"]
mod tests;
