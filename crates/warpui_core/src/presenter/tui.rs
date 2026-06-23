//! The TUI presenter: turns a root [`TuiView`]'s render output into a painted
//! [`TuiFrame`] (a cell [`TuiBuffer`] plus the absolute cursor cell).
//!
//! Placement: this is the additive `tui` submodule of `presenter` — the TUI
//! sibling of the GUI [`Presenter`](crate::Presenter) that owns the rest of
//! this module. Both backends genuinely have a presenter (layout + paint of a
//! window's view tree); keeping them under one module mirrors that symmetry
//! without gating any GUI path.
//!
//! # Layout pass ordering: measure → arrange → present → paint
//!
//! 1. **measure** — the root element is measured against a loose
//!    [`TuiConstraint`] bounded by the target area, returning the size it wants
//!    (within that box).
//! 2. **arrange** — that size is anchored at the area's origin, producing the
//!    absolute rectangle the root occupies. Container elements recurse this
//!    measure/arrange internally for their children (the presenter only drives
//!    the root; the element tree composes itself).
//! 3. **present** — the tree is walked via [`TuiElement::present`] to record
//!    parent/child *view* embeddings (from [`TuiChildView`]-style elements that
//!    embed a sub-view), which are reported to the core's neutral view
//!    hierarchy via [`AppContext::report_view_embeddings`]. That hierarchy is
//!    what the responder chain and focus ancestor propagation walk — for TUI
//!    views exactly as for GUI views.
//! 4. **paint** — the root paints into its arranged rectangle of a fresh
//!    buffer. Each container paints its children into their sub-rectangles, so
//!    the whole tree composites into one buffer.
//!
//! # Child views
//!
//! A child view is a full [`TuiView`] registered in the app. It is embedded by
//! resolving it through the app — [`AppContext::render_tui_view`] renders it to
//! its typed boxed element tree — and wrapping that output in a child-view
//! element during the *parent* view's render (which has app access). The
//! presenter then lays out and paints the composed tree, so the child's output
//! lands at exactly the area the layout allocated to it, and the present pass
//! records the embedded view as a child of its parent.
//!
//! [`TuiChildView`]: crate::elements::tui::TuiChildView

use std::collections::HashMap;

use crate::elements::tui::{
    TuiBuffer, TuiConstraint, TuiElement, TuiLayoutContext, TuiPresentationContext, TuiRect,
};
use crate::{AppContext, EntityId, TuiView, ViewHandle, WindowId, WindowInvalidation};

/// A painted frame: the composited cell [`TuiBuffer`] plus the absolute cursor
/// position (in buffer cell coordinates), if a focused element owns the cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiFrame {
    /// The composited cell grid, covering the columns/rows up to the painted
    /// area's right/bottom edge.
    pub buffer: TuiBuffer,
    /// The absolute `(x, y)` cell the terminal cursor should occupy, if any.
    pub cursor: Option<(u16, u16)>,
}

impl TuiFrame {
    /// A blank frame sized to cover `area`, with no cursor.
    fn blank(area: TuiRect) -> Self {
        Self {
            buffer: TuiBuffer::empty(buffer_rect_for(area)),
            cursor: None,
        }
    }
}

/// Lays out and paints a [`TuiView`]'s element tree into a [`TuiFrame`].
///
/// Mirrors the GUI [`Presenter`](crate::Presenter) pattern:
/// - [`invalidate`](Self::invalidate) re-renders only the views that changed
///   into `rendered_views`, leaving unchanged views' cached elements in place.
/// - [`present`](Self::present) performs layout (using the `rendered_views` map
///   via [`TuiLayoutContext`] so [`TuiChildView`] can find its child without a
///   nested render) and paint, then caches the root element in `last_element`
///   for event dispatch.
///
/// [`TuiChildView`]: crate::elements::tui::TuiChildView
#[derive(Default)]
pub struct TuiPresenter {
    /// Pre-rendered elements keyed by view id. Populated by [`invalidate`](Self::invalidate)
    /// for each view that changed; consumed by [`TuiChildView`] during layout.
    pub(crate) rendered_views: HashMap<EntityId, Box<dyn TuiElement>>,
    /// The root element tree from the last [`present`](Self::present) call,
    /// with all child views already laid out inside it. Reused as the starting
    /// point for the next frame's layout (for unchanged child subtrees) and for
    /// event dispatch between frames.
    pub(crate) last_element: Option<Box<dyn TuiElement>>,
}

impl TuiPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-renders the views listed in `invalidation.updated` into `rendered_views`,
    /// mirroring [`Presenter::invalidate`](crate::Presenter::invalidate).
    ///
    /// Called by the runtime before each draw so that [`present`] finds
    /// fresh elements for every changed view and stale-but-valid cached elements
    /// for everything else.
    ///
    /// [`present`]: Self::present
    pub fn invalidate(
        &mut self,
        invalidation: &WindowInvalidation,
        ctx: &AppContext,
        window_id: WindowId,
    ) {
        for &view_id in invalidation.updated.difference(&invalidation.removed) {
            match ctx.render_tui_view(window_id, view_id) {
                Ok(element) => {
                    self.rendered_views.insert(view_id, element);
                }
                Err(e) => log::warn!("TUI view {view_id:?} was not rendered: {e:?}"),
            }
        }
        for &view_id in &invalidation.removed {
            self.rendered_views.remove(&view_id);
        }
    }

    /// Lays out and paints the root view's element tree into `area`.
    ///
    /// The root element is taken from `rendered_views` (if the root was
    /// re-rendered this frame by [`invalidate`](Self::invalidate)) or from
    /// `last_element` (the previous frame's root). During layout, a
    /// [`TuiLayoutContext`] carrying `rendered_views` is threaded through the
    /// tree so [`TuiChildView`] can retrieve its child element without a nested
    /// render call. The laid-out root is stored as `last_element` for the next
    /// frame and for event dispatch.
    ///
    /// [`TuiChildView`]: crate::elements::tui::TuiChildView
    pub fn present<V: TuiView>(
        &mut self,
        ctx: &mut AppContext,
        root: &ViewHandle<V>,
        area: TuiRect,
    ) -> TuiFrame {
        let window_id = root.window_id(ctx);
        let root_view_id = root.id();

        // Element resolution order:
        //   1. Fresh from rendered_views (populated by invalidate() this frame).
        //   2. Cached last_element — ONLY when rendered_views is non-empty,
        //      meaning invalidate() was called and this view was not changed.
        //      If rendered_views is empty (no invalidate() was called), skip
        //      last_element: the root may be stale (e.g. view called notify()
        //      but the caller drives the presenter standalone without the
        //      runtime's invalidate() step).
        //   3. Direct render fallback for callers that skip invalidate().
        let Some(mut element) = self
            .rendered_views
            .remove(&root_view_id)
            .or_else(|| {
                if !self.rendered_views.is_empty() {
                    self.last_element.take()
                } else {
                    None
                }
            })
            .or_else(|| ctx.render_tui_view(window_id, root_view_id).ok())
        else {
            return TuiFrame::blank(area);
        };

        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut self.rendered_views,
        };
        let arranged = arrange(element.as_mut(), area, &mut layout_ctx);

        let mut embeddings = HashMap::new();
        {
            let mut present_ctx = TuiPresentationContext::new(
                root_view_id,
                &mut self.rendered_views,
                &mut embeddings,
            );
            element.present(&mut present_ctx);
        }
        ctx.report_view_embeddings(window_id, embeddings);

        let frame = paint(element.as_ref(), arranged, area, &mut self.rendered_views);
        self.last_element = Some(element);
        frame
    }

    /// Lays out and paints an already-rendered element tree into `area`.
    ///
    /// Exposed for the runtime and tests that drive layout/paint for an element
    /// tree produced outside the app's view registry. No view-ancestry is
    /// recorded and no `rendered_views` state is consulted or updated.
    pub fn present_element(&mut self, mut root: Box<dyn TuiElement>, area: TuiRect) -> TuiFrame {
        let mut empty_views = HashMap::new();
        let mut layout_ctx = TuiLayoutContext {
            rendered_views: &mut empty_views,
        };
        let arranged = arrange(root.as_mut(), area, &mut layout_ctx);
        paint(root.as_ref(), arranged, area, &mut empty_views)
    }

    /// Returns a mutable reference to the root element from the last
    /// [`present`](Self::present) call, for use by event dispatch.
    pub fn last_element_mut(&mut self) -> Option<&mut Box<dyn TuiElement>> {
        self.last_element.as_mut()
    }
}

/// Measure the root against `area` and anchor the measured size at the area's
/// origin (the size is already within the area, but clamp defensively so
/// writes stay in bounds).
fn arrange(root: &mut dyn TuiElement, area: TuiRect, ctx: &mut TuiLayoutContext) -> TuiRect {
    let measured = root.layout(TuiConstraint::loose(area.as_size()), ctx);
    TuiRect::new(
        area.x,
        area.y,
        measured.width.min(area.width),
        measured.height.min(area.height),
    )
}

/// Composite the tree into a fresh buffer and lift the root-relative cursor
/// offset to absolute coordinates. `rendered_views` is threaded through so
/// [`TuiChildView`] can look up its child during render and cursor passes.
///
/// [`TuiChildView`]: crate::elements::tui::TuiChildView
fn paint(
    root: &dyn TuiElement,
    arranged: TuiRect,
    area: TuiRect,
    rendered_views: &mut HashMap<EntityId, Box<dyn TuiElement>>,
) -> TuiFrame {
    let mut buffer = TuiBuffer::empty(buffer_rect_for(area));
    let mut ctx = TuiLayoutContext { rendered_views };
    root.render(arranged, &mut buffer, &mut ctx);

    let cursor = root
        .cursor_position(arranged, &mut ctx)
        .map(|(x, y)| (arranged.x.saturating_add(x), arranged.y.saturating_add(y)));

    TuiFrame { buffer, cursor }
}

/// The buffer rect needed to hold everything painted within `area`: it spans
/// from the origin to the area's right/bottom edge, so absolute coordinates
/// (including any area offset) index correctly.
fn buffer_rect_for(area: TuiRect) -> TuiRect {
    TuiRect::new(0, 0, area.right(), area.bottom())
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
