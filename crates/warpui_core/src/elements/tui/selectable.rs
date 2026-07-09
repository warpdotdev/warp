//! Thin interaction plumbing for TUI elements that own selection behavior.

use std::rc::Rc;

use super::{
    TuiBuffer, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext,
    TuiPaintContext, TuiPresentationContext, TuiRect, TuiScrollableElement, TuiSize,
};
use crate::AppContext;

mod cells;
mod state;

pub(crate) use cells::{cell_span, point_after_col, row_glyphs, scrape_row};
pub use cells::{TuiContentPoint, TuiRowGlyph, TuiSelectionSpan};
pub(crate) use state::TuiSelectionHandle;

type SelectionCallback = Box<dyn FnMut(&mut TuiEventContext, &AppContext)>;
type CopyCallback = Box<dyn FnMut(String, &mut TuiEventContext, &AppContext)>;

/// Resolves a semantic word unit from rendered row glyphs.
pub type TuiWordSelectionResolver =
    Rc<dyn Fn(TuiContentPoint, u16, &[TuiRowGlyph], &AppContext) -> Option<TuiSelectionSpan>>;

/// Semantic-word policy for a selectable viewport.
#[derive(Clone)]
pub struct TuiSelectionConfig {
    pub(crate) word_resolver: TuiWordSelectionResolver,
}

impl TuiSelectionConfig {
    /// Creates selection configuration with custom word semantics.
    pub fn new(word_resolver: TuiWordSelectionResolver) -> Self {
        Self { word_resolver }
    }
}

/// Result of delegating one selection-related event to a child element.
pub enum TuiSelectionEventResult {
    Unhandled,
    Started,
    Changed,
    Completed(Option<String>),
}

/// Semantic selection behavior implemented by the child element.
pub trait TuiSelectableElement: TuiElement {
    /// Returns whether the child owns an active drag gesture.
    fn selection_gesture_active(&self) -> bool;

    /// Handles one selection-related mouse event.
    fn dispatch_selection_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSelectionEventResult;

    /// Paints persistent selection state over normal child rendering.
    fn render_selection(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext);
}

/// Delegates selection interaction to a child and owns only external callbacks.
pub struct TuiSelectable<Child> {
    child: Child,
    on_selection_start: Option<SelectionCallback>,
    on_copy: Option<CopyCallback>,
}

impl<Child> TuiSelectable<Child>
where
    Child: TuiSelectableElement,
{
    /// Wraps a child that owns its selection behavior.
    pub fn new(child: Child) -> Self {
        Self {
            child,
            on_selection_start: None,
            on_copy: None,
        }
    }

    /// Runs `callback` when the child starts a selection.
    pub fn on_selection_start(
        mut self,
        callback: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.on_selection_start = Some(Box::new(callback));
        self
    }

    /// Runs `callback` when the child completes a non-empty selection.
    pub fn on_copy(
        mut self,
        callback: impl FnMut(String, &mut TuiEventContext, &AppContext) + 'static,
    ) -> Self {
        self.on_copy = Some(Box::new(callback));
        self
    }
}

impl<Child> TuiElement for TuiSelectable<Child>
where
    Child: TuiSelectableElement,
{
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
        self.child.render_selection(area, buffer, ctx);
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
        let captures_drag = self.child.selection_gesture_active()
            && matches!(
                event,
                TuiEvent::LeftMouseDragged { .. } | TuiEvent::LeftMouseUp { .. }
            );
        if !captures_drag && self.child.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        match self.child.dispatch_selection_event(event, area, ctx, app) {
            TuiSelectionEventResult::Unhandled => false,
            TuiSelectionEventResult::Started => {
                if let Some(callback) = self.on_selection_start.as_mut() {
                    callback(event_ctx, app);
                }
                event_ctx.notify();
                true
            }
            TuiSelectionEventResult::Changed => {
                event_ctx.notify();
                true
            }
            TuiSelectionEventResult::Completed(text) => {
                if let (Some(text), Some(callback)) = (text, self.on_copy.as_mut()) {
                    callback(text, event_ctx, app);
                }
                event_ctx.notify();
                true
            }
        }
    }
}

impl<Child> TuiScrollableElement for TuiSelectable<Child>
where
    Child: TuiSelectableElement + TuiScrollableElement,
{
    fn scroll_by_rows(&mut self, rows: isize, viewport_height: usize) -> bool {
        self.child.scroll_by_rows(rows, viewport_height)
    }
}
