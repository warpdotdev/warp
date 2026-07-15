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
mod scene;
mod scrollable;
mod selectable;
mod shimmering_text;
mod text;
mod viewported_list;

pub use animated::TuiAnimated;
pub use buffer::{Cell, Color, Modifier, TuiBuffer, TuiBufferExt, TuiPaintSurface, TuiStyle};
pub use child_view::TuiChildView;
pub use clipped::TuiClipped;
pub use collapsible::tui_collapsible;
pub use constrained_box::TuiConstrainedBox;
pub use container::TuiContainer;
pub use event::{
    TuiDispatchEventResult, TuiEvent, TuiEventContext, TuiEventDispatchResult, TuiScrollDelta,
};
pub use event_handler::TuiEventHandler;
pub use flex::TuiFlex;
pub use geometry::{
    TuiConstraint, TuiGridPoint, TuiPoint, TuiPointExt, TuiRect, TuiRectExt, TuiSize,
};
pub use hoverable::TuiHoverable;
pub use parent::TuiParentElement;
pub use scene::{
    TuiClipBounds, TuiLocalPoint, TuiScene, TuiScreenPoint, TuiScreenPosition, TuiScreenRect,
    TuiZIndex,
};
pub use scrollable::{TuiScrollable, TuiScrollableElement};
pub use selectable::{
    point_after_col, TuiRowGlyph, TuiRowResize, TuiSelectable, TuiSelectableElement,
    TuiSelectionHandle, TuiSelectionSpan,
};
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
    /// Retained clip and hit geometry produced during paint.
    pub scene: TuiScene,
    /// The earliest repaint deadline requested by any element this frame.
    repaint_at: Option<Instant>,
    /// Hardware terminal cursor submitted during paint.
    terminal_cursor: Option<TuiScreenPoint>,
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
            scene: TuiScene::default(),
            repaint_at: None,
            terminal_cursor: None,
        }
    }
    /// Attaches the active scene layer to an absolute screen position.
    pub fn scene_point(&self, position: TuiScreenPosition) -> TuiScreenPoint {
        TuiScreenPoint::from_position(position, self.scene.z_index())
    }

    /// Submits the hardware cursor for this frame, preferring higher layers.
    pub fn set_terminal_cursor(&mut self, cursor: TuiScreenPoint) {
        if self
            .terminal_cursor
            .is_some_and(|current| current.z_index > cursor.z_index)
        {
            return;
        }
        self.terminal_cursor = Some(cursor);
    }

    /// Returns the hardware cursor submitted during paint.
    pub fn terminal_cursor(&self) -> Option<TuiScreenPoint> {
        self.terminal_cursor
    }

    /// Runs `f` inside a clipped normal scene layer.
    pub fn with_scene_layer<R>(
        &mut self,
        bounds: TuiClipBounds,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.scene.start_layer(bounds);
        let result = f(self);
        self.scene.stop_layer();
        result
    }

    /// Runs `f` inside a clipped overlay scene layer.
    pub fn with_overlay_layer<R>(
        &mut self,
        bounds: TuiClipBounds,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.scene.start_overlay_layer(bounds);
        let result = f(self);
        self.scene.stop_layer();
        result
    }

    /// Makes the active scene layer transparent to hit testing.
    pub fn set_active_layer_click_through(&mut self) {
        self.scene.set_active_layer_click_through();
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

    /// Returns the earliest repaint requested by the current test paint.
    #[cfg(test)]
    pub(crate) fn requested_repaint_at(&self) -> Option<Instant> {
        self.repaint_at
    }

    /// Finishes paint and returns its retained scene and repaint request.
    pub(crate) fn finish(self) -> (TuiScene, Option<Instant>, Option<TuiScreenPoint>) {
        debug_assert!(self.scene.is_at_root_layer());
        (self.scene, self.repaint_at, self.terminal_cursor)
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
/// - [`render`](TuiElement::render): paint at an absolute screen origin through
///   a [`TuiPaintSurface`], retaining scene geometry through [`TuiPaintContext`].
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

    /// Runs once after the whole tree has been laid out, before paint, so an
    /// element can commit a *side effect* that depends on its final size
    /// (mirroring the GUI's `Element::after_layout`). Unlike
    /// [`layout`](TuiElement::layout) — the measurement pass, which may run more
    /// than once and must stay pure — this fires exactly once per frame with the
    /// arranged geometry settled, which is where size-driven effects like a PTY
    /// resize belong. `ctx` carries the presenter's view map so container
    /// elements can propagate the pass into their children (and
    /// [`TuiChildView`](crate::elements::tui::TuiChildView) into its embedded
    /// view). The default is a no-op; only container elements and elements with
    /// a post-layout side effect override it.
    fn after_layout(&mut self, _ctx: &mut TuiLayoutContext, _app: &AppContext) {}

    /// Paints this element at absolute `origin`. `surface` owns the active
    /// ratatui buffer and accepts only absolute-coordinate paint operations.
    /// `ctx` carries the
    /// presenter's pre-rendered view map so [`TuiChildView`] can look up and
    /// render its child element without caching it locally, and collects
    /// repaint requests from animated elements
    /// ([`TuiPaintContext::repaint_after`]).
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    );

    /// Returns the size retained by the most recent layout.
    fn size(&self) -> Option<TuiSize> {
        None
    }

    /// Returns the screen origin retained by the most recent paint.
    fn origin(&self) -> Option<TuiScreenPoint> {
        None
    }

    /// Returns the element's retained screen bounds.
    fn bounds(&self) -> Option<TuiScreenRect> {
        Some(TuiScreenRect::new(self.origin()?, self.size()?))
    }

    /// Walks this element during the presenter's child-view pass. Container and
    /// child-view elements override this to enter/exit their children on `ctx`;
    /// leaf elements do nothing.
    fn present(&mut self, _ctx: &mut TuiPresentationContext<'_>) {}

    /// Offers `event` to this element, returning `true` if it was handled.
    /// Retained geometry and the painted scene are available through
    /// `event_ctx`; `app` provides shared core access.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        _event_ctx: &mut TuiEventContext<'_>,
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

    fn render(
        &mut self,
        _origin: TuiScreenPosition,
        _surface: &mut TuiPaintSurface<'_>,
        _ctx: &mut TuiPaintContext,
    ) {
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
    use std::rc::Rc;

    use super::{
        TuiBuffer, TuiBufferExt, TuiElement, TuiEvent, TuiEventContext, TuiPaintContext,
        TuiPaintSurface, TuiRect, TuiScene, TuiSize,
    };
    use crate::presenter::tui::{TuiFrame, TuiPresenter};
    use crate::{App, AppContext, EntityId, EntityIdMap};

    /// Runs `f` with an identity-mapped surface and fresh paint context.
    pub(crate) fn with_paint_surface<R>(
        buffer: &mut TuiBuffer,
        f: impl FnOnce(&mut TuiPaintSurface<'_>, &mut TuiPaintContext<'_>) -> R,
    ) -> R {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiPaintContext::new(&mut rendered_views);
        let mut surface = TuiPaintSurface::new(buffer);
        f(&mut surface, &mut ctx)
    }
    /// Runs `f` with dispatch state over an empty scene and view map.
    pub(crate) fn with_event_context<R>(f: impl FnOnce(&mut TuiEventContext<'_>) -> R) -> R {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiEventContext::new(Rc::new(TuiScene::default()), &mut rendered_views);
        f(&mut ctx)
    }

    /// Renders `element` into a `size` buffer and returns the rows as strings.
    pub(crate) fn render_to_lines(
        element: impl TuiElement + 'static,
        size: TuiSize,
    ) -> Vec<String> {
        render_to_frame(element, size).buffer.to_lines()
    }

    /// Renders `element` through the presenter and returns the complete frame.
    pub(crate) fn render_to_frame(element: impl TuiElement + 'static, size: TuiSize) -> TuiFrame {
        App::test((), |app| async move {
            app.read(|app_ctx| {
                TuiPresenter::new().present_element(
                    element.finish(),
                    TuiRect::new(0, 0, size.width, size.height),
                    app_ctx,
                )
            })
        })
    }

    /// Dispatches through the element tree and scene retained by `presenter`.
    pub(crate) fn dispatch_presented_event(
        presenter: &mut TuiPresenter,
        event: &TuiEvent,
        app: &AppContext,
    ) -> (bool, usize) {
        let (Some(element), Some(scene)) = (
            presenter.last_element.as_mut(),
            presenter.last_scene.clone(),
        ) else {
            return (false, 0);
        };
        let mut event_ctx = TuiEventContext::new(scene, &mut presenter.rendered_views);
        event_ctx.set_origin_view(Some(EntityId::new()));
        let handled = element.dispatch_event(event, &mut event_ctx, app);
        (handled, event_ctx.take_notified().len())
    }
}
