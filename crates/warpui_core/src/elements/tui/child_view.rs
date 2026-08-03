//! [`TuiChildView`]: embeds another [`TuiView`]'s rendered element tree as a
//! node in this view's tree.
//!
//! # Construction
//! Build with [`TuiChildView::new`], passing a [`ViewHandle`] and the current
//! [`AppContext`]; the child view is rendered immediately and stored as the
//! wrapped element.
//!
//! # Tree participation
//! `TuiChildView` is otherwise transparent — layout, render, height, and cursor
//! all delegate to the embedded element. It additionally hooks the two passes
//! that are view-aware:
//! - [`present`](TuiElement::present) enters the child's view id on the
//!   presentation context (recording the parent/child relationship) before
//!   descending, then exits it.
//! - [`dispatch_event`](TuiElement::dispatch_event) marks the child's view id as
//!   the action origin for the duration of the subtree's dispatch.

use super::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize,
    TuiViewMapContext,
};
#[cfg(test)]
use crate::EntityIdMap;
use crate::{AppContext, EntityId, TuiView, ViewHandle};

/// Embeds a registered [`TuiView`] as a node in the element tree, mirroring
/// the GUI's `ChildView` design: the child element is never cached in this
/// struct. Instead, every pass (layout, render, present, dispatch) temporarily
/// removes the child from [`TuiLayoutContext::rendered_views`] (or
/// [`TuiPresentationContext::rendered_views`]), uses it, and returns it — the
/// same move-in/move-out pattern the GUI uses through its `PaintContext` and
/// `EventContext`.
pub struct TuiChildView {
    view_id: EntityId,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl TuiChildView {
    pub fn new<V: TuiView>(handle: &ViewHandle<V>) -> Self {
        Self {
            view_id: handle.id(),
            size: None,
            origin: None,
        }
    }

    /// Inserts a pre-rendered element directly into `rendered_views` for
    /// headless tests that exercise the embedding/recursion contract without
    /// a full presenter. Returns the `TuiChildView` node that will look up the
    /// element from `rendered_views` during each pass.
    #[cfg(test)]
    pub(crate) fn from_rendered(
        view_id: EntityId,
        child: Box<dyn TuiElement>,
        rendered_views: &mut EntityIdMap<Box<dyn TuiElement>>,
    ) -> Self {
        rendered_views.insert(view_id, child);
        Self {
            view_id,
            size: None,
            origin: None,
        }
    }

    /// Constructs a bare child-view node for tests — no element pre-inserted.
    /// The caller must populate `rendered_views` separately before any pass.
    #[cfg(test)]
    pub(crate) fn for_view_id(view_id: EntityId) -> Self {
        Self {
            view_id,
            size: None,
            origin: None,
        }
    }
}

impl TuiElement for TuiChildView {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let size = ctx
            .use_view(self.view_id, |child, ctx| {
                child.layout(constraint, ctx, app)
            })
            .unwrap_or_else(|| {
                log::warn!("TuiChildView: no element found for {:?}", self.view_id);
                TuiSize::ZERO
            });
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        ctx.use_view(self.view_id, |child, ctx| child.after_layout(ctx, app));
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        ctx.use_view(self.view_id, |child, ctx| {
            child.render(origin, surface, ctx)
        });
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        ctx.enter_child(self.view_id);
        ctx.use_view(self.view_id, |child, ctx| child.present(ctx));
        ctx.exit_child();
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        event_ctx
            .use_view(self.view_id, |child, event_ctx| {
                let previous_origin = event_ctx.set_origin_view(Some(self.view_id));
                let handled = child.dispatch_event(event, event_ctx, app);
                event_ctx.set_origin_view(previous_origin);
                handled
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
#[path = "child_view_tests.rs"]
mod tests;
