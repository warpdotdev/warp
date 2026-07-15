//! The TUI view layer, additive behind the `tui` feature.
//!
//! [`TuiView`] is the TUI sibling of [`View`](super::View): it shares all of
//! the neutral entity machinery (entity IDs, ref counts, handles,
//! subscriptions/observations, focus, the responder chain, typed actions, and
//! the unified [`ViewContext`]) and differs only in its render output
//! ([`TuiElement`] instead of `Element`).

use std::any::Any;

use super::{BlurContext, FocusContext, ViewContext};
use crate::elements::tui::TuiElement;
use crate::{keymap, AppContext, Entity, EntityId, WindowId};

/// An interactive, renderable TUI component. The TUI counterpart of
/// [`View`](crate::View); registered with
/// [`AppContext::add_tui_view`](crate::AppContext::add_tui_view) or
/// [`AppContext::add_typed_action_tui_view`](crate::AppContext::add_typed_action_tui_view).
pub trait TuiView: Entity {
    /// Returns a unique name for this implementation of TuiView.
    fn ui_name() -> &'static str;

    /// Produces the [`TuiElement`] representation of this view.
    ///
    /// Terminal resizes flow through the layout pass: the presenter lays out
    /// against the current terminal size every frame, and each
    /// [`TuiElement::layout`] receives the [`AppContext`], so width-dependent
    /// read-only state (e.g. a char-cell editor's terminal width) is refreshed
    /// there. A size-driven *side effect* that must run once with the settled
    /// geometry — e.g. committing a PTY resize — belongs in
    /// [`TuiElement::after_layout`], the post-layout pass the presenter runs
    /// after arranging the tree and before paint (mirroring the GUI's
    /// `Element::after_layout`).
    fn render(&self, app: &AppContext) -> Box<dyn TuiElement>;

    /// Handles the view or its descendent receiving focus.
    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {}

    /// Handles the view or its descendent losing focus.
    fn on_blur(&mut self, _blur_ctx: &BlurContext, _ctx: &mut ViewContext<Self>) {}

    /// Returns a representation of the current UI context for use in computing
    /// the set of valid actions/keyboard shortcuts.
    fn keymap_context(&self, _: &AppContext) -> keymap::Context {
        Self::default_keymap_context()
    }

    /// Returns the default context for a view.
    fn default_keymap_context() -> keymap::Context {
        let mut ctx = keymap::Context::default();
        ctx.set.insert(Self::ui_name());
        ctx
    }

    /// Returns the ids of child views this view directly owns via
    /// [`ViewHandle`]s that are not registered in the structural parent/child
    /// graph, regardless of whether they are currently being rendered.
    ///
    /// See [`View::child_view_ids`](crate::View::child_view_ids) for the full
    /// contract. The semantics are identical for TUI views.
    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        Vec::new()
    }
}

/// The object-safe, type-erased TUI view object stored per window: the TUI
/// counterpart of [`AnyView`](crate::AnyView), with hook signatures that match
/// it so the shared dispatch paths treat both uniformly.
pub trait AnyTuiView {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn ui_name(&self) -> &'static str;
    fn render(&self, app: &AppContext) -> Box<dyn TuiElement>;
    fn on_focus(
        &mut self,
        focus_ctx: &FocusContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    );
    fn on_blur(
        &mut self,
        blur_ctx: &BlurContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    );
    fn keymap_context(&self, app: &AppContext) -> keymap::Context;
    fn child_view_ids(&self, app: &AppContext) -> Vec<EntityId>;
}

impl<T> AnyTuiView for T
where
    T: TuiView,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn ui_name(&self) -> &'static str {
        T::ui_name()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        TuiView::render(self, app)
    }

    fn on_focus(
        &mut self,
        focus_ctx: &FocusContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        TuiView::on_focus(self, focus_ctx, &mut ctx);
    }

    fn on_blur(
        &mut self,
        blur_ctx: &BlurContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        let mut ctx = ViewContext::new(app, window_id, view_id);
        TuiView::on_blur(self, blur_ctx, &mut ctx);
    }

    fn keymap_context(&self, app: &AppContext) -> keymap::Context {
        TuiView::keymap_context(self, app)
    }

    fn child_view_ids(&self, app: &AppContext) -> Vec<EntityId> {
        TuiView::child_view_ids(self, app)
    }
}
