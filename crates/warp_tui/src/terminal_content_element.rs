//! Publishes terminal content size and optionally forwards foreground PTY input.
//!
//! The TUI mirror of the GUI's `TerminalSizeElement`
//! (`app/src/terminal/terminal_size_element.rs`): a transparent wrapper around
//! the element currently displaying PTY content — the block-list content
//! column or the full-screen alt-screen grid. Once layout settles each frame,
//! it publishes the child's size on a channel; the session view consumes it
//! with a `ViewContext` and commits the model + PTY resize (see
//! `TuiTerminalSessionView::handle_terminal_resize`), which layout and paint
//! passes cannot do themselves.
//!
//! When configured with [`TuiTerminalContentElement::with_pty_input`], the same
//! wrapper also gives the foreground process first refusal on key and paste
//! events. Keeping both responsibilities here ensures the subtree measured for
//! the PTY is also the subtree that owns its input.

use std::ops::Deref as _;
use std::sync::Arc;

use async_channel::Sender;
use parking_lot::FairMutex;
use warp::tui_export::{KeystrokeWithDetails, TerminalModel};
use warp_terminal::model::escape_sequences::{BRACKETED_PASTE_END, BRACKETED_PASTE_START};
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
};
use warpui_core::AppContext;

use crate::terminal_session_view::TuiTerminalSessionAction;

/// Wraps the element displaying PTY content, reports its laid-out size, and
/// optionally forwards input to the foreground process.
pub(crate) struct TuiTerminalContentElement {
    child: Box<dyn TuiElement>,
    resize_tx: Sender<TuiSize>,
    pty_input_model: Option<Arc<FairMutex<TerminalModel>>>,
}

impl TuiTerminalContentElement {
    /// Wraps `child`, publishing its laid-out size on `resize_tx`.
    pub(crate) fn new(resize_tx: Sender<TuiSize>, child: Box<dyn TuiElement>) -> Self {
        Self {
            child,
            resize_tx,
            pty_input_model: None,
        }
    }

    /// Gives the foreground process first refusal on key and paste events for
    /// this terminal-content subtree.
    pub(crate) fn with_pty_input(mut self, model: Arc<FairMutex<TerminalModel>>) -> Self {
        self.pty_input_model = Some(model);
        self
    }
}

impl TuiElement for TuiTerminalContentElement {
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
        // `after_layout` fires once per frame with the arranged geometry
        // settled (unlike `layout`, which may measure speculatively), so this
        // is the size the PTY should adopt. A closed channel just means the
        // consumer is gone; dropping the send is fine.
        if let Some(size) = self.child.size() {
            let _ = self.resize_tx.try_send(size);
        }
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
        if let Some(model) = self.pty_input_model.as_ref() {
            match event {
                TuiEvent::KeyDown {
                    is_composing: false,
                    ..
                }
                | TuiEvent::Paste { .. } => {
                    if let Some(bytes) = pty_bytes_for_event(event, model) {
                        event_ctx.dispatch_typed_action(
                            TuiTerminalSessionAction::ForwardUserPtyBytes(bytes),
                        );
                    }
                    return true;
                }
                TuiEvent::KeyDown {
                    is_composing: true, ..
                } => return false,
                TuiEvent::ScrollWheel { .. }
                | TuiEvent::LeftMouseDown { .. }
                | TuiEvent::LeftMouseUp { .. }
                | TuiEvent::LeftMouseDragged { .. }
                | TuiEvent::MiddleMouseDown { .. }
                | TuiEvent::RightMouseDown { .. }
                | TuiEvent::MouseMoved { .. } => {}
            }
        }
        self.child.dispatch_event(event, event_ctx, app)
    }
}

/// Converts one semantic TUI input event to bytes for the foreground process.
/// Pointer events and composing key events are left for the child subtree.
fn pty_bytes_for_event(event: &TuiEvent, model: &Arc<FairMutex<TerminalModel>>) -> Option<Vec<u8>> {
    match event {
        TuiEvent::KeyDown {
            keystroke,
            chars,
            details,
            is_composing: false,
        } => {
            let model = model.lock();
            KeystrokeWithDetails {
                keystroke,
                key_without_modifiers: details.key_without_modifiers.as_deref(),
                chars: Some(chars.as_str()),
            }
            .to_pty_bytes(model.deref())
        }
        TuiEvent::Paste { text } => {
            let needs_bracketed_paste = model.lock().needs_bracketed_paste();
            Some(paste_bytes(text, needs_bracketed_paste))
        }
        TuiEvent::KeyDown {
            is_composing: true, ..
        }
        | TuiEvent::ScrollWheel { .. }
        | TuiEvent::LeftMouseDown { .. }
        | TuiEvent::LeftMouseUp { .. }
        | TuiEvent::LeftMouseDragged { .. }
        | TuiEvent::MiddleMouseDown { .. }
        | TuiEvent::RightMouseDown { .. }
        | TuiEvent::MouseMoved { .. } => None,
    }
}

fn paste_bytes(text: &str, needs_bracketed_paste: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if !needs_bracketed_paste {
        return normalized.into_bytes();
    }

    let mut bytes = Vec::with_capacity(
        BRACKETED_PASTE_START.len() + normalized.len() + BRACKETED_PASTE_END.len(),
    );
    bytes.extend_from_slice(BRACKETED_PASTE_START);
    bytes.extend_from_slice(normalized.as_bytes());
    bytes.extend_from_slice(BRACKETED_PASTE_END);
    bytes
}

#[cfg(test)]
#[path = "terminal_content_element_tests.rs"]
mod tests;
