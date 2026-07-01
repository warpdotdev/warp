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
//! - [`TuiParentElement`]: a trait for multi-child elements, providing
//!   [`with_child`](TuiParentElement::with_child) /
//!   [`with_children`](TuiParentElement::with_children) /
//!   [`add_child`](TuiParentElement::add_child) /
//!   [`add_children`](TuiParentElement::add_children).

use crate::{AppContext, EntityId, EntityIdMap};

mod buffer;
mod child_view;
mod clipped;
mod color;
mod column;
mod constrained_box;
mod container;
mod event;
mod event_handler;
mod geometry;
mod parent;
mod scrollable;
mod text;
mod viewported_list;

pub use buffer::{Cell, Color, Modifier, TuiBuffer, TuiBufferExt, TuiStyle};
pub use child_view::TuiChildView;
pub use clipped::TuiClipped;
pub use column::TuiColumn;
pub use constrained_box::TuiConstrainedBox;
pub use container::TuiContainer;
pub use event::{
    TuiDispatchEventResult, TuiEvent, TuiEventContext, TuiEventDispatchResult, TuiScrollDelta,
};
pub use event_handler::TuiEventHandler;
pub use geometry::{TuiConstraint, TuiPoint, TuiPointExt, TuiRect, TuiRectExt, TuiSize};
pub use parent::TuiParentElement;
pub use scrollable::{TuiScrollable, TuiScrollableElement};
pub use text::TuiText;
pub use viewported_list::{
    TuiViewportContent, TuiViewportPosition, TuiViewportVerticalAlignment, TuiViewportWindow,
    TuiViewportedElement, TuiViewportedList, TuiViewportedListState, TuiVisibleViewportItem,
};

/// Carries the pre-rendered per-view element map through the layout pass,
/// mirroring the GUI's `LayoutContext`. [`TuiChildView`] uses it to look up
/// its child element (freshly rendered by [`TuiPresenter::invalidate`] if
/// the child was updated, or cached from the previous frame otherwise).
///
/// [`TuiChildView`]: crate::elements::tui::TuiChildView
/// [`TuiPresenter::invalidate`]: crate::presenter::tui::TuiPresenter::invalidate
pub struct TuiLayoutContext<'a> {
    /// Pre-rendered elements keyed by view id, consumed during layout.
    pub rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>,
}

impl<'a> TuiLayoutContext<'a> {
    /// Temporarily removes the element for `view_id` from `rendered_views`,
    /// passes it (along with `self`) to `f`, then returns it. Mirrors the
    /// GUI's `LayoutContext::layout` / `PaintContext::paint` /
    /// `EventContext::dispatch_event_on_view` pattern. Returns the value
    /// produced by `f`, or `None` if no element was registered for `view_id`.
    pub(crate) fn use_view<R>(
        &mut self,
        view_id: EntityId,
        f: impl FnOnce(&mut Box<dyn TuiElement>, &mut Self) -> R,
    ) -> Option<R> {
        let mut element = self.rendered_views.remove(&view_id)?;
        let result = f(&mut element, self);
        self.rendered_views.insert(view_id, element);
        Some(result)
    }
}

/// A node in the renderable tree: it measures itself against a constraint,
/// then paints into a sub-rectangle of the buffer.
///
/// - [`layout`](TuiElement::layout): measure against a [`TuiConstraint`] and
///   [`TuiLayoutContext`], returning a [`TuiSize`] within the constraint (see
///   [`TuiConstraint::clamp`]). The context carries the presenter's
///   pre-rendered view map so [`TuiChildView`](crate::elements::tui::TuiChildView)
///   can retrieve its child element.
/// - [`render`](TuiElement::render): paint into `area` of `buffer`. `area` is
///   the rect the parent allocated (its size is the value `layout` returned,
///   clamped to what was available).
/// - [`cursor_position`](TuiElement::cursor_position): where a text cursor
///   should sit within `area`, if any (default: none).
/// - [`present`](TuiElement::present): participate in the child-view recursion
///   so the presenter can record parent/child view relationships (default:
///   nothing — only container/child-view elements override this).
/// - [`dispatch_event`](TuiElement::dispatch_event): offer an event to this
///   element, returning whether it was handled (default: not handled).
pub trait TuiElement {
    /// Measures this element against `constraint`, returning the size it will
    /// occupy (which must lie within `constraint`). `ctx` carries the
    /// presenter's pre-rendered view map for child-view lookup; `app` provides
    /// shared read access to the core, mirroring the GUI's `Element::layout`, so
    /// an element can push viewport-dependent state into a model during layout.
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize;

    /// Paints this element into `area` of `buffer`. `ctx` carries the
    /// presenter's pre-rendered view map so [`TuiChildView`] can look up and
    /// render its child element without caching it locally.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext);

    /// The `(x, y)` cell, within `area`, where the terminal cursor should be
    /// placed for this element, if it owns the cursor. `ctx` is passed through
    /// so [`TuiChildView`] can delegate to its child without caching it.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        None
    }

    /// Walks this element during the presenter's child-view pass. Container and
    /// child-view elements override this to enter/exit their children on `ctx`;
    /// leaf elements do nothing.
    fn present(&mut self, _ctx: &mut TuiPresentationContext<'_>) {}

    /// Offers `event` to this element within `area`, returning `true` if it was
    /// handled. `event_ctx` collects app updates and typed actions;
    /// `ctx` carries the presenter's pre-rendered view map so [`TuiChildView`]
    /// can look up and dispatch into its child; `app` provides read access to
    /// the shared core during dispatch.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        _area: TuiRect,
        _event_ctx: &mut TuiEventContext,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    /// Boxes this element as a trait object, mirroring the GUI `Element::finish`
    /// convenience so element trees can be terminated with `.finish()` rather
    /// than an explicit `Box::new`.
    fn finish(self) -> Box<dyn TuiElement>
    where
        Self: 'static + Sized,
    {
        Box::new(self)
    }
}

/// A no-op leaf element: occupies no space and paints nothing. Used by tests
/// as a placeholder child where the element's own rendering is irrelevant.
#[cfg(test)]
impl TuiElement for () {
    fn layout(
        &mut self,
        _constraint: TuiConstraint,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiSize {
        TuiSize::ZERO
    }

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiLayoutContext) {}
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
    parent_by_child: &'a mut EntityIdMap<EntityId>,
    pub(crate) rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>,
    view_stack: Vec<EntityId>,
}

impl<'a> TuiPresentationContext<'a> {
    pub(crate) fn new(
        root_view_id: EntityId,
        rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>,
        parent_by_child: &'a mut EntityIdMap<EntityId>,
    ) -> Self {
        Self {
            parent_by_child,
            rendered_views,
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

    /// Temporarily removes the element for `view_id` from `rendered_views`,
    /// passes it (along with `self`) to `f`, then returns it — the same
    /// move-in/move-out pattern the GUI's `EventContext::dispatch_event_on_view`
    /// uses. Returns `None` if no element is registered for `view_id`.
    pub(crate) fn use_view<R>(
        &mut self,
        view_id: EntityId,
        f: impl FnOnce(&mut Box<dyn TuiElement>, &mut Self) -> R,
    ) -> Option<R> {
        let mut element = self.rendered_views.remove(&view_id)?;
        let result = f(&mut element, self);
        self.rendered_views.insert(view_id, element);
        Some(result)
    }
}
