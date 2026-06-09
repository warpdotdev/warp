//! The `TuiElement` trait — the unit of layout and paint — and the bridge that
//! lets a [`TuiView`](warpui_core::TuiView) render to a boxed element while the
//! shared core stores that output fully type-erased.
//!
//! The concrete elements (text, columns, styled containers, child-view,
//! key-event handler) are added by task 3.2; this module owns the *trait* they
//! implement and the *presentation context* they thread through during the
//! child-view recursion that the presenter (task 3.3) drives.

use std::collections::HashMap;

use warpui_core::{AppContext, EntityId, Event};

use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};

/// What a [`TuiView`](warpui_core::TuiView) renders to.
///
/// A view sets `type RenderOutput = TuiRenderOutput` and returns a boxed
/// element from `render_tui`. The shared core never names this type: it boxes
/// the value again into `Box<dyn Any>` (the abstract `TuiBackend::RenderOutput`),
/// which the presenter recovers by downcasting back to `Box<dyn TuiElement>`.
/// Because `TuiRenderOutput` is `'static`, it satisfies the core's
/// `TuiView::RenderOutput: 'static` bound, and that erasure is exactly what
/// keeps `warpui_core` free of any dependency on this crate.
pub type TuiRenderOutput = Box<dyn TuiElement>;

/// A node in the renderable tree: it measures itself against a constraint, then
/// paints into a sub-rectangle of the buffer.
///
/// FROZEN METHOD SET. Sibling tasks (3.2 elements, 3.3 presenter, 3.4 runtime)
/// build against exactly these methods:
/// - [`layout`](TuiElement::layout): measure against a [`TuiConstraint`],
///   returning a [`TuiSize`] within it (see [`TuiConstraint::clamp`]).
/// - [`render`](TuiElement::render): paint into `area` of `buffer`. `area` is
///   the rect the parent allocated (its size is the value `layout` returned,
///   clamped to what was available).
/// - [`desired_height`](TuiElement::desired_height): the height this element
///   wants at a given width, used by stacking containers before they have a
///   final height budget.
/// - [`cursor_position`](TuiElement::cursor_position): where a text cursor
///   should sit within `area`, if any (default: none).
/// - [`present`](TuiElement::present): participate in the child-view recursion
///   so the presenter can record parent/child view relationships (default:
///   nothing — only container/child-view elements override this).
/// - [`dispatch_event`](TuiElement::dispatch_event): offer an event to this
///   element, returning whether it was handled (default: not handled).
pub trait TuiElement {
    /// Measures this element against `constraint`, returning the size it will
    /// occupy (which must lie within `constraint`).
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize;

    /// Paints this element into `area` of `buffer`. Implementations must confine
    /// their writes to `area`; the buffer clips anything outside its own bounds
    /// but does not clip to `area`.
    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer);

    /// The height this element wants when laid out at `width` columns.
    fn desired_height(&self, width: u16) -> u16;

    /// The `(x, y)` cell, within `area`, where the terminal cursor should be
    /// placed for this element, if it owns the cursor.
    fn cursor_position(&self, _area: TuiRect) -> Option<(u16, u16)> {
        None
    }

    /// Walks this element during the presenter's child-view pass. Container and
    /// child-view elements override this to enter/exit their children on `ctx`;
    /// leaf elements do nothing.
    fn present(&mut self, _ctx: &mut TuiPresentationContext<'_>) {}

    /// Offers `event` to this element within `area`, returning `true` if it was
    /// handled. `ctx` collects deferred app updates and typed actions; `app`
    /// provides read access to the shared core during dispatch.
    fn dispatch_event(
        &mut self,
        _event: &Event,
        _area: TuiRect,
        _ctx: &mut TuiEventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }
}

impl TuiElement for () {
    fn layout(&mut self, _constraint: TuiConstraint) -> TuiSize {
        TuiSize::ZERO
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer) {}

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

/// Threads the current view ancestry through the element tree during the
/// presenter's child-view recursion, recording each child view's parent so the
/// shared core can attribute events and actions to the right view.
///
/// Constructed by the presenter (task 3.3); mutated by container/child-view
/// elements (task 3.2) from their [`present`](TuiElement::present) impls. Its
/// internals are inert until those consumers land.
#[allow(dead_code)]
pub struct TuiPresentationContext<'a> {
    parent_by_child: &'a mut HashMap<EntityId, EntityId>,
    view_stack: Vec<EntityId>,
}

impl<'a> TuiPresentationContext<'a> {
    #[allow(dead_code)]
    pub(crate) fn new(
        root_view_id: EntityId,
        parent_by_child: &'a mut HashMap<EntityId, EntityId>,
    ) -> Self {
        Self {
            parent_by_child,
            view_stack: vec![root_view_id],
        }
    }

    /// Records `child_view_id` as a child of the current view and descends into
    /// it. Each call must be paired with [`exit_child`](Self::exit_child).
    #[allow(dead_code)]
    pub(crate) fn enter_child(&mut self, child_view_id: EntityId) {
        let parent_view_id = *self
            .view_stack
            .last()
            .expect("the TUI presentation stack always contains a root view");
        self.parent_by_child.insert(child_view_id, parent_view_id);
        self.view_stack.push(child_view_id);
    }

    /// Ascends back out of the most recently entered child view.
    #[allow(dead_code)]
    pub(crate) fn exit_child(&mut self) {
        self.view_stack
            .pop()
            .expect("a child view is entered before it is exited");
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
