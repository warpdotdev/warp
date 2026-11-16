//! TUI-backend extensions to [`ViewContext`].

use super::{TypedActionView, ViewContext, ViewHandle};
use crate::Entity;

impl<'a, T: Entity> ViewContext<'a, T> {
    /// The TUI counterpart of [`Self::add_view`].
    pub fn add_tui_view<S, F>(&mut self, build_view: F) -> ViewHandle<S>
    where
        S: crate::TuiView,
        F: FnOnce(&mut ViewContext<S>) -> S,
    {
        self.app.add_tui_view(self.window_id, build_view)
    }

    /// The TUI counterpart of [`Self::add_typed_action_view`]: the new view is
    /// recorded as a structural child of this context's view, so it joins the
    /// shared responder chain.
    pub fn add_typed_action_tui_view<V, F>(&mut self, build_view: F) -> ViewHandle<V>
    where
        V: TypedActionView + crate::TuiView,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.app
            .add_typed_action_tui_view_with_parent(self.window_id, build_view, self.view_id)
    }
}
