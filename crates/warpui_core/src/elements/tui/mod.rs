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
//! - The concrete elements: [`TuiText`], [`TuiFlex`], [`TuiContainer`],
//!   [`TuiChildView`], and [`TuiEventHandler`].
//! - [`TuiParentElement`]: a trait for multi-child elements, providing
//!   [`with_child`](TuiParentElement::with_child) /
//!   [`with_children`](TuiParentElement::with_children) /
//!   [`add_child`](TuiParentElement::add_child) /
//!   [`add_children`](TuiParentElement::add_children).

use std::time::Duration;

use instant::Instant;

use crate::{AppContext, EntityId, EntityIdMap};

mod animated;
mod buffer;
mod child_view;
mod clipped;
mod collapsible;
mod color;
mod constrained_box;
mod container;
mod event;
mod event_handler;
mod flex;
mod geometry;
mod hoverable;
mod parent;
mod scrollable;
mod shimmering_text;
mod text;
mod viewported_list;

pub use animated::TuiAnimated;
pub use buffer::{Cell, Color, Modifier, TuiBuffer, TuiBufferExt, TuiStyle};
pub use child_view::TuiChildView;
pub use clipped::TuiClipped;
pub use collapsible::{tui_collapsible, tui_disclosure_chevron};
pub use constrained_box::TuiConstrainedBox;
pub use container::TuiContainer;
pub use event::{
    TuiDispatchEventResult, TuiEvent, TuiEventContext, TuiEventDispatchResult, TuiScrollDelta,
};
pub use event_handler::TuiEventHandler;
pub use flex::TuiFlex;
pub use geometry::{TuiConstraint, TuiPoint, TuiPointExt, TuiRect, TuiRectExt, TuiSize};
pub use hoverable::TuiHoverable;
pub use parent::TuiParentElement;
pub use scrollable::{TuiScrollable, TuiScrollableElement};
pub use shimmering_text::TuiShimmeringText;
pub use text::TuiText;
pub use viewported_list::{
    TuiViewportContent, TuiViewportPosition, TuiViewportVerticalAlignment, TuiViewportWindow,
    TuiViewportedElement, TuiViewportedList, TuiViewportedListState, TuiVisibleViewportItem,
};

/// Shared access to the presenter's pre-rendered view map for the context
/// types threaded through the element tree, providing the one
/// [`use_view`](Self::use_view) implementation they all share.
pub(crate) trait TuiViewMapContext: Sized {
    /// The presenter's pre-rendered elements keyed by view id.
    fn rendered_views_mut(&mut self) -> &mut EntityIdMap<Box<dyn TuiElement>>;

    /// Temporarily removes the element for `view_id` from the view map,
    /// passes it (along with `self`) to `f`, then returns it. Mirrors the
    /// GUI's `LayoutContext::layout` / `PaintContext::paint` /
    /// `EventContext::dispatch_event_on_view` pattern. Returns the value
    /// produced by `f`, or `None` if no element was registered for `view_id`.
    fn use_view<R>(
        &mut self,
        view_id: EntityId,
        f: impl FnOnce(&mut Box<dyn TuiElement>, &mut Self) -> R,
    ) -> Option<R> {
        let mut element = self.rendered_views_mut().remove(&view_id)?;
        let result = f(&mut element, self);
        self.rendered_views_mut().insert(view_id, element);
        Some(result)
    }
}

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

impl TuiViewMapContext for TuiLayoutContext<'_> {
    fn rendered_views_mut(&mut self) -> &mut EntityIdMap<Box<dyn TuiElement>> {
        self.rendered_views
    }
}

/// Carries the pre-rendered per-view element map through the paint pass and
/// accumulates repaint requests, mirroring the GUI's `PaintContext`.
///
/// [`TuiChildView`] uses the view map to look up its child element; animated
/// elements call [`repaint_after`](Self::repaint_after) during
/// [`TuiElement::render`] to request a timed redraw. Requests coalesce with
/// earliest-deadline-wins, and the presenter surfaces the winning deadline on
/// the painted frame so the runtime can schedule exactly one repaint timer.
pub struct TuiPaintContext<'a> {
    /// Pre-rendered elements keyed by view id, consumed during paint.
    pub rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>,
    /// The earliest repaint deadline requested by any element this frame.
    repaint_at: Option<Instant>,
}

/// The soonest an element may request a repaint after the current paint.
/// Floors zero/tiny [`TuiPaintContext::repaint_after`] delays so a
/// misbehaving element can't busy-loop the repaint scheduler.
const MIN_REPAINT_DELAY: Duration = Duration::from_millis(10);

impl<'a> TuiPaintContext<'a> {
    /// Creates a paint context over the presenter's pre-rendered view map with
    /// no repaint requested.
    pub fn new(rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>) -> Self {
        Self {
            rendered_views,
            repaint_at: None,
        }
    }

    /// Requests a repaint after `delay` (floored to [`MIN_REPAINT_DELAY`]),
    /// keeping the earliest pending deadline.
    pub fn repaint_after(&mut self, delay: Duration) {
        self.repaint_at(Instant::now() + delay.max(MIN_REPAINT_DELAY));
    }

    /// Requests a repaint at `new_repaint_at`, keeping the earlier deadline if
    /// one is already pending.
    fn repaint_at(&mut self, new_repaint_at: Instant) {
        if self
            .repaint_at
            .is_some_and(|repaint_at| repaint_at <= new_repaint_at)
        {
            return;
        }
        self.repaint_at = Some(new_repaint_at);
    }

    /// The earliest repaint deadline requested during this paint, if any.
    pub(crate) fn requested_repaint_at(&self) -> Option<Instant> {
        self.repaint_at
    }
}

impl TuiViewMapContext for TuiPaintContext<'_> {
    fn rendered_views_mut(&mut self) -> &mut EntityIdMap<Box<dyn TuiElement>> {
        self.rendered_views
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
    /// render its child element without caching it locally, and collects
    /// repaint requests from animated elements
    /// ([`TuiPaintContext::repaint_after`]).
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext);

    /// The `(x, y)` cell, within `area`, where the terminal cursor should be
    /// placed for this element, if it owns the cursor. `ctx` is passed through
    /// so [`TuiChildView`] can delegate to its child without caching it.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn cursor_position(&self, _area: TuiRect, _ctx: &mut TuiPaintContext) -> Option<(u16, u16)> {
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

    fn render(&self, _area: TuiRect, _buffer: &mut TuiBuffer, _ctx: &mut TuiPaintContext) {}
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
}

impl TuiViewMapContext for TuiPresentationContext<'_> {
    fn rendered_views_mut(&mut self) -> &mut EntityIdMap<Box<dyn TuiElement>> {
        self.rendered_views
    }
}

/// Shared harnesses for TUI element tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{TuiBuffer, TuiBufferExt, TuiElement, TuiPaintContext, TuiRect, TuiSize};
    use crate::EntityIdMap;

    /// Runs `f` with a paint context over a fresh, empty view map — the
    /// common harness for leaf-element paint tests.
    pub(crate) fn with_paint_context<R>(f: impl FnOnce(&mut TuiPaintContext) -> R) -> R {
        let mut rendered_views = EntityIdMap::default();
        f(&mut TuiPaintContext::new(&mut rendered_views))
    }

    /// Renders `element` into a `size` buffer and returns the rows as strings.
    pub(crate) fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
        let area = TuiRect::new(0, 0, size.width, size.height);
        let mut buffer = TuiBuffer::empty(area);
        with_paint_context(|ctx| element.render(area, &mut buffer, ctx));
        buffer.to_lines()
    }
}
