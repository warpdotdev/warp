//! Publishes a subtree's laid-out cell size for PTY sizing.
//!
//! The TUI mirror of the GUI's `TerminalSizeElement`
//! (`app/src/terminal/terminal_size_element.rs`): a transparent wrapper around
//! the element currently displaying PTY content — the block-list content
//! column or the full-screen alt-screen grid. Once layout settles each frame,
//! it publishes the child's size on a channel; the session view consumes it
//! with a `ViewContext` and commits the model + PTY resize (see
//! `TuiTerminalSessionView::handle_terminal_resize`), which layout and paint
//! passes cannot do themselves.

use async_channel::Sender;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
};
use warpui_core::AppContext;

/// Wraps the element displaying PTY content and reports its laid-out size.
pub(crate) struct TuiTerminalSizeElement {
    child: Box<dyn TuiElement>,
    resize_tx: Sender<TuiSize>,
}

impl TuiTerminalSizeElement {
    /// Wraps `child`, publishing its laid-out size on `resize_tx`.
    pub(crate) fn new(resize_tx: Sender<TuiSize>, child: Box<dyn TuiElement>) -> Self {
        Self { child, resize_tx }
    }
}

impl TuiElement for TuiTerminalSizeElement {
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
        self.child.dispatch_event(event, event_ctx, app)
    }
}

#[cfg(test)]
#[path = "terminal_size_element_tests.rs"]
mod tests;
