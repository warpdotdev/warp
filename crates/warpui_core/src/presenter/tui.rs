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
    TuiBuffer, TuiConstraint, TuiElement, TuiPresentationContext, TuiRect, TuiSize,
};
use crate::{AppContext, TuiView, ViewHandle};

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
            buffer: TuiBuffer::new(buffer_size_for(area)),
            cursor: None,
        }
    }
}

/// Lays out and paints a [`TuiView`]'s element tree into a [`TuiFrame`].
///
/// The view ancestry discovered while presenting is reported into the core's
/// neutral `view_parents` hierarchy (the single source of truth shared with the
/// GUI presenter), so the presenter itself retains no parent map.
#[derive(Default)]
pub struct TuiPresenter;

impl TuiPresenter {
    pub fn new() -> Self {
        Self
    }

    /// Renders the root view through the app, then lays it out and paints it
    /// into `area`, returning the composited [`TuiFrame`].
    ///
    /// The root is resolved via [`AppContext::render_tui_view`] as a typed
    /// `Box<dyn TuiElement>`; a view that is not a TUI view yields a blank
    /// frame. Child-view embeddings discovered during the present pass are
    /// reported via [`AppContext::report_view_embeddings`] — as a batch,
    /// because the present pass borrows the rendered tree mutably (the same
    /// constraint the GUI presenter's `build_scene` has).
    pub fn present<V: TuiView>(
        &mut self,
        ctx: &mut AppContext,
        root: &ViewHandle<V>,
        area: TuiRect,
    ) -> TuiFrame {
        let window_id = root.window_id(ctx);
        let root_view_id = root.id();
        let Ok(mut element) = ctx.render_tui_view(window_id, root_view_id) else {
            return TuiFrame::blank(area);
        };

        let arranged = arrange(element.as_mut(), area);

        let mut embeddings = HashMap::new();
        {
            let mut present_ctx = TuiPresentationContext::new(root_view_id, &mut embeddings);
            element.present(&mut present_ctx);
        }
        ctx.report_view_embeddings(window_id, embeddings);

        paint(element.as_ref(), arranged, area)
    }

    /// Lays out and paints an already-rendered element tree into `area`.
    ///
    /// This is the backend-agnostic core of [`present`](Self::present), exposed
    /// so the runtime (and tests) can drive layout/paint for an element tree
    /// that was produced outside the app's view registry. No view-ancestry is
    /// recorded.
    pub fn present_element(&mut self, mut root: Box<dyn TuiElement>, area: TuiRect) -> TuiFrame {
        let arranged = arrange(root.as_mut(), area);
        paint(root.as_ref(), arranged, area)
    }
}

/// Measure the root against `area` and anchor the measured size at the area's
/// origin (the size is already within the area, but clamp defensively so
/// writes stay in bounds).
fn arrange(root: &mut dyn TuiElement, area: TuiRect) -> TuiRect {
    let measured = root.layout(TuiConstraint::loose(area.size()));
    TuiRect::new(
        area.x,
        area.y,
        measured.width.min(area.width),
        measured.height.min(area.height),
    )
}

/// Composite the tree into a fresh buffer and lift the root-relative cursor
/// offset to absolute coordinates.
fn paint(root: &dyn TuiElement, arranged: TuiRect, area: TuiRect) -> TuiFrame {
    let mut buffer = TuiBuffer::new(buffer_size_for(area));
    root.render(arranged, &mut buffer);

    let cursor = root
        .cursor_position(arranged)
        .map(|(x, y)| (arranged.x.saturating_add(x), arranged.y.saturating_add(y)));

    TuiFrame { buffer, cursor }
}

/// The buffer size needed to hold everything painted within `area`: it spans
/// from the origin to the area's right/bottom edge, so absolute coordinates
/// (including any area offset) index correctly.
fn buffer_size_for(area: TuiRect) -> TuiSize {
    TuiSize::new(area.right(), area.bottom())
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
