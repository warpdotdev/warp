//! The TUI-backend half of the `AppContext` API: TUI view/window creation and
//! TUI view rendering, available when compiled with `tui` feature.

use anyhow::{Result, anyhow};

use super::{
    AddWindowOptions, AppContext, StoredView, TypedActionView, ViewContext, ViewHandle, Window,
    autotracking,
};
use crate::keymap::{BindingLens, IsBindingValid};
use crate::{EntityId, WindowId};

impl AppContext {
    /// Registers a validator that validates every binding that matches the
    /// given TUI view's default [`Context`](crate::keymap::Context). The TUI
    /// counterpart of [`Self::register_binding_validator`]: after the app is
    /// initialized, the validator runs for every binding matching the view's
    /// default context, and the app panics on invalid bindings when
    /// `debug_assertions` are enabled.
    #[cfg(debug_assertions)]
    pub fn register_tui_binding_validator<V: crate::TuiView>(
        &mut self,
        binding_validator: impl Fn(BindingLens) -> IsBindingValid + 'static,
    ) {
        let context = V::default_keymap_context();
        self.keystroke_matcher
            .register_binding_validator(context, binding_validator)
    }

    /// Registers a validator that validates every binding that matches the
    /// given TUI view's default [`Context`](crate::keymap::Context). Noops if
    /// `debug_assertions` are disabled.
    #[cfg(not(debug_assertions))]
    pub fn register_tui_binding_validator<V: crate::TuiView>(
        &mut self,
        binding_validator: impl Fn(BindingLens) -> IsBindingValid + 'static,
    ) {
    }

    /// Adds a TUI view to the given window.
    pub fn add_tui_view<T, F>(&mut self, window_id: WindowId, build_view: F) -> ViewHandle<T>
    where
        T: crate::TuiView,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        let view_id = EntityId::new();
        self.pending_flushes += 1;
        let mut ctx = ViewContext::new(self, window_id, view_id);
        let view = build_view(&mut ctx);
        let window = self
            .windows
            .get_mut(&window_id)
            .expect("Window does not exist");
        window
            .views
            .insert(view_id, StoredView::Tui(Box::new(view)));
        self.view_to_window.insert(view_id, window_id);
        self.window_invalidations
            .entry(window_id)
            .or_default()
            .updated
            .insert(view_id);
        let handle = ViewHandle::new(window_id, view_id, &self.ref_counts);
        self.flush_effects();
        handle
    }

    /// Adds a TUI view that handles typed actions.
    pub fn add_typed_action_tui_view<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + crate::TuiView,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.add_typed_action_tui_view_internal(window_id, build_view, None)
    }

    /// [`Self::add_typed_action_tui_view`] with creation-time structural
    /// parentage, mirroring [`Self::add_typed_action_view_with_parent`].
    pub(crate) fn add_typed_action_tui_view_with_parent<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
        parent_view_id: EntityId,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + crate::TuiView,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.add_typed_action_tui_view_internal(window_id, build_view, Some(parent_view_id))
    }

    fn add_typed_action_tui_view_internal<V, F>(
        &mut self,
        window_id: WindowId,
        build_view: F,
        parent_view_id: Option<EntityId>,
    ) -> ViewHandle<V>
    where
        V: TypedActionView + crate::TuiView,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        self.pending_flushes += 1;

        let view_id = EntityId::new();
        let mut ctx = ViewContext::new(self, window_id, view_id);
        let view = build_view(&mut ctx);
        let window = self
            .windows
            .get_mut(&window_id)
            .expect("Window does not exist");
        window
            .views
            .insert(view_id, StoredView::Tui(Box::new(view)));

        self.register_typed_action_view_internal::<V>(window_id, view_id, parent_view_id)
    }

    /// Creates a new TUI window with the view returned by `build_root_view` as
    /// its root view. The TUI counterpart of [`Self::add_window`], reduced to
    /// the backend-neutral subset: window-id + bounds bookkeeping, root-view
    /// construction, and focus. No presenter, platform window, or
    /// scene/event callbacks — the [`runtime::TuiRuntime`](crate::runtime::TuiRuntime)
    /// owns the draw + input loop for the window.
    pub fn add_tui_window<T, F>(
        &mut self,
        options: AddWindowOptions,
        build_root_view: F,
    ) -> (WindowId, ViewHandle<T>)
    where
        T: crate::TuiView + TypedActionView,
        F: FnOnce(&mut ViewContext<T>) -> T,
    {
        let AddWindowOptions {
            window_bounds,
            anchor_new_windows_from_closed_position,
            ..
        } = options;

        let window_id = WindowId::new();

        // Store the window bounds before creating the root view, in case it
        // uses this value.
        self.window_bounds.insert(window_id, window_bounds.bounds());
        self.next_window_bounds_map
            .insert(window_id, anchor_new_windows_from_closed_position);
        // Clear the next window bounds if they were set - we don't want to
        // start from the last closed position after a new window has been
        // created.
        self.next_window_bounds = None;

        self.windows.insert(window_id, Window::default());
        let root_handle = self.add_typed_action_tui_view(window_id, build_root_view);
        let root_view_id = root_handle.id();
        self.windows
            .get_mut(&window_id)
            .expect("this window was just inserted and should still exist")
            .root_view = Some((&root_handle).into());
        self.focus(window_id, root_view_id);

        (window_id, root_handle)
    }

    /// Renders the given TUI view to a [`TuiElement`](crate::elements::tui::TuiElement),
    /// tracking any `Tracked` reads as rendering dependencies.
    pub fn render_tui_view(
        &self,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Result<Box<dyn crate::elements::tui::TuiElement>> {
        let window = self
            .windows
            .get(&window_id)
            .ok_or_else(|| anyhow!("window not found"))?;
        match window.views.get(&view_id) {
            Some(StoredView::Tui(view)) => {
                Ok(autotracking::render_view(window_id, view_id, || {
                    view.render(self)
                }))
            }
            Some(StoredView::Gui(_)) => Err(anyhow!("view is not a TUI view")),
            None => Err(anyhow!("view not found")),
        }
    }
}
