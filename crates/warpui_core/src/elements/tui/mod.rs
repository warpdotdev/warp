//! The TUI element library, additive behind the `tui` feature.
//!
//! [`TuiElement`] is the unit of layout and paint: what a
//! [`TuiView`](crate::TuiView) renders to. The library mirrors the GUI element
//! vocabulary at terminal-cell granularity:
//!
//! - [`geometry`]: integer cell-grid geometry ([`TuiSize`], [`TuiRect`],
//!   [`TuiConstraint`]).
//! - [`buffer`]: the in-memory styled cell grid ([`TuiBuffer`], [`Cell`],
//!   [`TuiStyle`]) elements paint into and the renderer flushes to the
//!   terminal.
//! - [`event`]: the dispatch-side event types ([`TuiEventContext`],
//!   [`TuiDispatchEventResult`], [`TuiEventDispatchResult`]) threaded through
//!   [`TuiElement::dispatch_event`]. (The crossterm → warp event *conversion*
//!   lives with the runtime, in `crate::runtime`.)
//! - The concrete elements: [`TuiText`], [`TuiColumn`], [`TuiContainer`],
//!   [`TuiChildView`], and [`TuiEventHandler`].

use std::collections::HashMap;

use crate::{AppContext, EntityId, Event};

mod buffer;
mod child_view;
mod column;
mod container;
mod event;
mod event_handler;
mod geometry;
mod text;

pub use buffer::{Cell, TuiBuffer, TuiStyle};
pub use child_view::TuiChildView;
pub use column::TuiColumn;
pub use container::TuiContainer;
pub use event::{TuiDispatchEventResult, TuiEventContext, TuiEventDispatchResult};
pub use event_handler::TuiEventHandler;
pub use geometry::{TuiConstraint, TuiRect, TuiSize};
pub use text::TuiText;

/// What a [`TuiView`](crate::TuiView) renders to: a boxed element tree.
///
/// Unlike the legacy out-of-crate TUI backend, the core stores and renders
/// this output fully typed — there is no `Any` erasure or downcasting anywhere
/// in the TUI render path.
pub type TuiRenderOutput = Box<dyn TuiElement>;

/// A node in the renderable tree: it measures itself against a constraint,
/// then paints into a sub-rectangle of the buffer.
///
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

/// An empty element, useful as a placeholder render output.
#[derive(Default)]
pub struct TuiEmpty;

impl TuiElement for TuiEmpty {
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
/// Constructed by the TUI presenter; mutated by container/child-view elements
/// from their [`present`](TuiElement::present) impls. The recorded embeddings
/// are reported to the neutral view hierarchy via
/// [`AppContext::report_view_embeddings`].
pub struct TuiPresentationContext<'a> {
    parent_by_child: &'a mut HashMap<EntityId, EntityId>,
    view_stack: Vec<EntityId>,
}

impl<'a> TuiPresentationContext<'a> {
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
    pub fn enter_child(&mut self, child_view_id: EntityId) {
        let parent_view_id = *self
            .view_stack
            .last()
            .expect("the TUI presentation stack always contains a root view");
        self.parent_by_child.insert(child_view_id, parent_view_id);
        self.view_stack.push(child_view_id);
    }

    /// Ascends back out of the most recently entered child view.
    pub fn exit_child(&mut self) {
        self.view_stack
            .pop()
            .expect("a child view is entered before it is exited");
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
