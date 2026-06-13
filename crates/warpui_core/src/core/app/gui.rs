//! The presenter-backed (GUI window) half of the `App`/`AppContext` API:
//! platform window creation and the event/scene/draw loop.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Debug;
use std::rc::Rc;

use anyhow::Result;
use futures::future::join_all;
use instant::Instant;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::{AddWindowOptions, App, AppContext, ClosedWindowData, EventMunger};
use crate::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use crate::event::KeyState;
use crate::fonts::{self, ExternalFontFamily, FallbackFontModel, RequestedFallbackFontSource};
use crate::keymap::{CustomTag, MatchResult, Trigger};
use crate::platform::keyboard::KeyCode;
use crate::platform::{WindowBounds, WindowContext, WindowOptions};
use crate::presenter::{CursorUpdate, DispatchedActionKind};
use crate::r#async::Timer;
use crate::windowing::{WindowCallbacks, WindowManager};
use crate::{
    rendering, AccessibilityData, CursorInfo, EntityId, Event,
    NextNewWindowsHasThisWindowsBoundsUponClose, Presenter, Scene, SingletonEntity, WindowId,
};

impl App {
    pub fn presenter(&self, window_id: WindowId) -> Option<Rc<RefCell<Presenter>>> {
        self.0.borrow().presenter(window_id)
    }

    pub fn dispatch_custom_action<Action>(&mut self, action: Action, window_id: WindowId)
    where
        Action: Into<CustomTag> + Debug + Copy,
    {
        self.0
            .borrow_mut()
            .dispatch_custom_action(action, window_id);
    }
}

impl AppContext {
    /// Returns the [`AccessibilityData`] of the focused view, or a parent of that view in its
    /// responder chain.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn focused_view_accessibility_data(
        &mut self,
        window_id: WindowId,
    ) -> Option<AccessibilityData> {
        let responder_chain = self.get_responder_chain(window_id);
        for view_id in responder_chain {
            let window = self.windows.get_mut(&window_id)?;
            let view = window.views.remove(&view_id)?;
            let accessibility_data = view.accessibility_data(self, window_id, view_id);

            if let Some(window) = self.windows.get_mut(&window_id) {
                window.views.insert(view_id, view);
            }

            if let Some(accessibility_data) = accessibility_data {
                return Some(accessibility_data);
            }
        }
        None
    }

    /// Snapshot of the window's child-view → parent-view map (for debug tooling).
    fn view_parent_map(&self, window_id: WindowId) -> HashMap<EntityId, EntityId> {
        self.view_parents
            .get(&window_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn open_view_tree_debug_window(&mut self, target_window_id: WindowId) {
        let Some(root_view_id) = self.root_view_id(target_window_id) else {
            return;
        };

        let Some(current_bounds) = self.window_bounds(&target_window_id) else {
            return;
        };
        let size = Vector2F::new(340., 540.);
        let origin = Vector2F::new(
            current_bounds.origin().x() + current_bounds.width() - size.x() - 20.,
            current_bounds.origin().y() + 20.,
        );

        let options = AddWindowOptions {
            window_bounds: WindowBounds::ExactPosition(RectF::new(origin, size)),
            anchor_new_windows_from_closed_position:
                NextNewWindowsHasThisWindowsBoundsUponClose::No,
            window_instance: Some("dev.warp.warpui-debug".to_owned()),
            title: Some("View Tree Debugger".to_owned()),
            ..Default::default()
        };
        let view_parents = self.view_parent_map(target_window_id);
        self.add_window(options, |ctx| {
            crate::debug::DebugRootView::new(target_window_id, view_parents, root_view_id, ctx)
        });
    }
}

/// The GUI half of the window/event/draw loop.
impl AppContext {
    pub fn rendering_config(&self) -> rendering::Config {
        self.presentation.rendering_config
    }

    pub fn update_rendering_config<U>(&mut self, update_fn: U)
    where
        U: FnOnce(&mut rendering::Config),
    {
        update_fn(&mut self.presentation.rendering_config)
    }

    pub fn presenter(&self, window_id: WindowId) -> Option<Rc<RefCell<Presenter>>> {
        // The presenter may not exist if there is a race condition where a window event comes in
        // after the window is closed (for example, if a fullscreen window is closed, a resize event
        // comes in after the window is closed.
        self.presentation.presenters.get(&window_id).cloned()
    }

    /// Drops the GUI presentation state for a closed window.
    pub(super) fn drop_window_presentation(&mut self, window_id: WindowId) {
        self.presentation.presenters.remove(&window_id);
    }

    /// Dispatches a custom action through the focused view's responder chain,
    /// falling back to replaying the bound keystroke when key-binding dispatch
    /// is disabled (i.e. while the user is editing their keybindings).
    ///
    /// Custom actions themselves are backend-neutral (the matcher/registry
    /// machinery lives in `app.rs`); this entry point is GUI-only solely
    /// because the keystroke-replay fallback drives the presenter-backed event
    /// loop, which doesn't exist under the `tui` backend.
    pub fn dispatch_custom_action<Action>(&mut self, action: Action, window_id: WindowId)
    where
        Action: Into<CustomTag> + Debug + Copy,
    {
        if self.key_bindings_enabled(window_id) {
            log::info!("Dispatching custom action {action:?}");
            let responder_chain = self.get_responder_chain(window_id);
            let res =
                self.dispatch_custom_action_internal(action.into(), window_id, &responder_chain);

            if let Ok(true) = res {
                self.dispatch_self_or_child_interacted_with(window_id, &responder_chain);
            }

            if let Err(error) = res {
                log::error!("error dispatching custom action: {error}");
            }
        } else {
            // We hit this case when the user is in the course of editing their keybindings.
            self.dispatch_custom_action_keystroke(action, window_id);
        }
    }

    /// Figures out what keystroke (if any) is bound to a custom action and dispatches it as a key event.
    /// This is used when the user is in the course of editing their keybindings and we need
    /// to get the raw keystroke for something that is currently bound to a custom action.
    ///
    /// This happens commonly with actions that are defined through Mac menu items (e.g. cmd-p for
    /// the command palette). If we want to map another keybinding to cmd-p while cmd-p is
    /// currently mapped to the command palette through a mac menu item, the only way to do it
    /// is by first handling the custom action, and then looking up the keystroke for the
    /// action, and then handling that as a raw key event.
    fn dispatch_custom_action_keystroke<Action>(
        &mut self,
        action: Action,
        window_id: WindowId,
    ) -> bool
    where
        Action: Into<CustomTag> + Debug + Clone + Copy,
    {
        self.contexts_from_responder_chain(window_id, &self.get_responder_chain(window_id))
            .ok()
            .and_then(|contexts| self.binding_for_custom_action(action.into(), contexts))
            .and_then(|binding| match binding.trigger {
                Trigger::Keystrokes(keys) => keys.first().cloned(),
                Trigger::Custom(custom_tag) => self
                    .keystroke_matcher
                    .default_keystroke_trigger_for_custom_action(*custom_tag),
                _ => None,
            })
            .map(|keystroke| Event::KeyDown {
                keystroke: keystroke.clone(),
                chars: keystroke.key,
                details: Default::default(),
                is_composing: false,
            })
            .and_then(|key_event| {
                self.presenter(window_id).map(|presenter| {
                    log::info!("Dispatching key event {key_event:?} for custom action {action:?}");
                    self.handle_non_keybound_event(key_event, window_id, presenter.clone())
                        .handled
                })
            })
            .unwrap_or_default()
    }

    fn dispatch_custom_action_internal(
        &mut self,
        action: CustomTag,
        window_id: WindowId,
        responder_chain: &[EntityId],
    ) -> Result<bool> {
        let mut context_chain = self.contexts_from_responder_chain(window_id, responder_chain)?;
        for (i, ctx) in context_chain.iter_mut().enumerate().rev() {
            let handled = match self.keystroke_matcher.match_custom(action, ctx) {
                MatchResult::Action(action) => self.dispatch_typed_action(
                    window_id,
                    &responder_chain[0..=i],
                    action.as_ref(),
                    log::Level::Info,
                ),
                _ => false,
            };
            if handled {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn dispatch_draw_frame_error_callback(&mut self, window_id: WindowId) {
        let callback = self.on_draw_frame_error_callback.take();
        if let Some(callback) = &callback {
            callback(self, window_id);
        }

        self.on_draw_frame_error_callback = callback;
    }

    fn trigger_on_frame_drawn_callbacks(&mut self, window_id: WindowId) {
        if let Some(callback) = self.first_frame_callback.take() {
            callback(self);
        }

        let frame_drawn_callback = self.frame_drawn_callback.take();
        if let Some(callback) = &frame_drawn_callback {
            callback(self, window_id);
        }
        self.frame_drawn_callback = frame_drawn_callback;

        if let Some(callbacks) = self.next_frame_callbacks.remove(&window_id) {
            for callback in callbacks {
                callback();
            }
        }
    }

    fn active_cursor_position(&mut self, window_id: WindowId) -> Option<CursorInfo> {
        let focused_view_id = self.focused_view_id(window_id)?;
        let view = self
            .windows
            .get_mut(&window_id)?
            .views
            .remove(&focused_view_id)?;
        let position = view.active_cursor_position(self, window_id, focused_view_id);

        if let Some(window) = self.windows.get_mut(&window_id) {
            window.views.insert(focused_view_id, view);
        }

        position
    }

    /// GUI window creation: allocate the window id + bounds bookkeeping, create
    /// the [`Presenter`], open the platform window and wire its
    /// [`WindowCallbacks`] (event/scene/resize loop), build the root view, focus
    /// it, and register the redraw invalidation callback.
    pub(super) fn insert_window_internal<F>(
        &mut self,
        window_id: Option<WindowId>,
        add_window_options: AddWindowOptions,
        build_window_data: F,
    ) -> (WindowId, EntityId)
    where
        F: FnOnce(WindowId, &mut AppContext) -> EntityId,
    {
        let AddWindowOptions {
            window_style,
            window_bounds,
            title,
            fullscreen_state,
            background_blur_radius_pixels,
            background_blur_texture,
            anchor_new_windows_from_closed_position,
            on_gpu_driver_selected: on_gpu_driver_reported,
            window_instance,
        } = add_window_options;

        let window_id = window_id.unwrap_or_else(WindowId::new);

        // Make sure we store the window bounds before we create the root view,
        // in case it uses this value.
        self.window_bounds.insert(window_id, window_bounds.bounds());
        self.next_window_bounds_map
            .insert(window_id, anchor_new_windows_from_closed_position);

        // Clear the next window bounds if they were set - we don't want to start
        // from the last closed position after a new window has been created.
        self.next_window_bounds = None;

        self.presentation
            .presenters
            .insert(window_id, Rc::new(RefCell::new(Presenter::new(window_id))));

        let window_options = WindowOptions {
            bounds: window_bounds,
            fullscreen_state,
            hide_title_bar: true,
            title,
            style: window_style,
            background_blur_radius_pixels,
            background_blur_texture,
            gpu_power_preference: self.presentation.rendering_config.gpu_power_preference,
            backend_preference: self.presentation.rendering_config.backend_preference,
            on_gpu_device_info_reported: on_gpu_driver_reported.unwrap_or(Box::new(|_| {})),
            window_instance,
        };

        let callbacks = WindowCallbacks {
            standard_action_callback: Box::new(move |action, ctx| {
                log::info!("Dispatching standard action {action:?}");
                let responder_chain = ctx.get_responder_chain(window_id);
                let res = ctx.dispatch_standard_action(action, window_id, &responder_chain);
                if let Err(error) = res {
                    log::error!("error dispatching standard action: {error}");
                }
            }),
            event_callback: Box::new(move |event, ctx| {
                let last_mouse_moved_event: Rc<RefCell<Option<Event>>> =
                    ctx.get_last_mouse_moved_event(window_id);
                match event {
                    Event::MouseMoved { .. } => {
                        *last_mouse_moved_event.borrow_mut() = Some(event.clone())
                    }
                    // Update the last mouse moved event with the current state of the modifiers
                    // so we don't dispatch a synthetic moused moved event with an incorrect modifier state.
                    Event::ModifierStateChanged {
                        modifiers,
                        key_code,
                        ..
                    } => {
                        if let Some(Event::MouseMoved { cmd, shift, .. }) =
                            &mut (*last_mouse_moved_event.borrow_mut())
                        {
                            *cmd = modifiers.cmd;
                            *shift = modifiers.shift;
                        }

                        if let Some(presenter) = ctx.presenter(window_id) {
                            if let Some(key_code) = key_code {
                                // Based on the key code in question and the new state of the modifier key,
                                // we can infer whether it was pressed or released.
                                let key_pressed = match key_code {
                                    KeyCode::ShiftLeft | KeyCode::ShiftRight => {
                                        Some(modifiers.shift)
                                    }
                                    KeyCode::ControlLeft | KeyCode::ControlRight => {
                                        Some(modifiers.ctrl)
                                    }
                                    KeyCode::AltLeft | KeyCode::AltRight => Some(modifiers.alt),
                                    KeyCode::SuperLeft | KeyCode::SuperRight => Some(modifiers.cmd),
                                    KeyCode::Fn => Some(modifiers.func),
                                    _ => None,
                                };
                                if let Some(key_pressed) = key_pressed {
                                    // Note: this can be slightly incorrect in a particular edge case where the user
                                    // uses 2 physical keys corresponding to the same logical modifer. For example:
                                    // 1. The user holds down right-alt - we fire the right-alt pressed event
                                    // 2. The user then holds down left-alt - we fire the left-alt pressed event
                                    // 3. The user lets go of left-alt - we would incorrectly fire the left-alt pressed event (since the logical state is still true)
                                    // 4. The user lets go of right-alt - we correctly fire the right-alt released event
                                    // This is a known limitation due to the underlying APIs being limited (we must use lower-level Apple
                                    // APIs to get the exact physical key states, which we currently don't do).
                                    let key_state = if key_pressed {
                                        KeyState::Pressed
                                    } else {
                                        KeyState::Released
                                    };

                                    ctx.handle_window_event(
                                        Event::ModifierKeyChanged {
                                            key_code,
                                            state: key_state,
                                        },
                                        window_id,
                                        presenter.clone(),
                                    );
                                }
                            }
                        }
                    }
                    // Update the last mouse moved event on mouse up with the new position.
                    // This makes sure that hoverables are updated to the proper mouse
                    // position after a click-and-drag, as we don't update the cached event
                    // on a LeftMouseDragged event.
                    Event::LeftMouseUp {
                        position: new_position,
                        modifiers,
                    } => {
                        if let Some(Event::MouseMoved {
                            cmd,
                            shift,
                            position,
                            is_synthetic: _,
                        }) = &mut (*last_mouse_moved_event.borrow_mut())
                        {
                            *position = new_position;
                            *cmd = modifiers.cmd;
                            *shift = modifiers.shift;
                        }
                    }
                    _ => (),
                };

                if let Some(presenter) = ctx.presenter(window_id) {
                    ctx.handle_window_event(event, window_id, presenter)
                } else {
                    crate::windowing::EventDispatchResult::default()
                }
            }),
            resize_callback: Box::new(move |window, ctx| {
                let origin = window.origin();
                let size = window.size();
                ctx.window_bounds
                    .insert(window_id, Some(RectF::new(origin, size)));

                window.request_redraw();

                // On Linux and Windows, we don't have a direct way to react to
                // window fullscreen state changes, so instead we're using a
                // resize event as a signal that the fullscreen state _may_ have
                // changed.
                #[cfg(any(target_os = "linux", target_os = "freebsd", windows))]
                crate::windowing::WindowManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.update_is_active_window_fullscreen(ctx);
                });

                ctx.report_active_cursor_position_update();
            }),
            build_scene_callback: Box::new(move |window, ctx| ctx.build_scene(window_id, window)),
            frame_callback: Box::new(move |ctx| {
                ctx.trigger_on_frame_drawn_callbacks(window_id);
            }),
            draw_frame_error_callback: Box::new(move |ctx| {
                ctx.dispatch_draw_frame_error_callback(window_id);
            }),
            move_callback: Box::new(move |bound, ctx| {
                ctx.window_bounds.insert(window_id, Some(bound));
                ctx.report_active_cursor_position_update();
            }),
            active_cursor_position_callback: Box::new(move |ctx| {
                ctx.active_cursor_position(window_id)
            }),
        };

        let window_result = WindowManager::handle(self).update(self, |windowing_state, _ctx| {
            windowing_state.open_window(window_id, window_options, callbacks)
        });

        // Create the root view after adding the window but before registering an invalidation callback.
        // This ensures that a platform window is always available to user view code.
        let root_view_id = build_window_data(window_id, self);
        self.focus(window_id, root_view_id);

        match window_result {
            Err(err) => {
                log::error!("error opening window: {err}");
            }
            Ok(_) => {
                self.on_window_invalidated(window_id, move |window_id, ctx| {
                    if let Some(window) = ctx.windows().platform_window(window_id) {
                        // All we need to do when a window is invalidated is
                        // request from the OS that it be redrawn.  We'll do all
                        // of the actual work later.
                        window.request_redraw();

                        // In tests, however, there's no real event loop, so we need to do the work now.
                        // While this _shouldn't_ be necessary in integration tests, it currently is.
                        if ctx.is_unit_test || cfg!(feature = "integration_tests") {
                            ctx.build_scene(window_id, window.as_ctx());
                        }
                    }
                });
            }
        };

        (window_id, root_view_id)
    }

    pub fn reopen_closed_window(&mut self, data: ClosedWindowData) {
        let ClosedWindowData {
            window_id,
            window,
            subscriptions,
            observations,
            view_to_window,
            bounds,
            fullscreen_state,
        } = data;

        let Some(bounds) = bounds else {
            log::error!("Had no bounds for cached closed window!");
            return;
        };

        for (entity_id, subs) in subscriptions {
            self.subscriptions.insert(entity_id, subs);
        }
        for (entity_id, obs) in observations {
            self.observations.insert(entity_id, obs);
        }
        for (entity_id, win_id) in view_to_window {
            self.view_to_window.insert(entity_id, win_id);
        }

        let add_window_options = AddWindowOptions {
            // TODO(vorporeal): what's the right value here?
            background_blur_radius_pixels: None,
            background_blur_texture: false,
            window_bounds: WindowBounds::ExactPosition(bounds),
            // TODO(alokedesai): Determine if, and how, we want to pass the on_gpu_driver_reported
            // callback from the original window back to this window.
            on_gpu_driver_selected: None,
            fullscreen_state,
            ..Default::default()
        };
        let root_view_id = window
            .root_view
            .as_ref()
            .expect("should have root view")
            .id();
        self.insert_window_internal(
            Some(window_id),
            add_window_options,
            move |window_id, ctx| {
                ctx.windows.insert(window_id, window);
                root_view_id
            },
        );

        self.invalidate_all_views_for_window(window_id);
    }

    /// Builds a new scene for the given window.
    fn build_scene(&mut self, window_id: WindowId, window: &dyn WindowContext) -> Rc<Scene> {
        let mut scene = Rc::new(Scene::new(
            window.backing_scale_factor(),
            self.rendering_config(),
        ));
        let Some(presenter) = self.presenter(window_id) else {
            return scene;
        };

        // This outer loop exists because after redrawing a scene, we will sometimes emit an
        // artificial `MouseMoved` event to ensure that `Hoverable` elements are properly updated
        // if the layout changes. However, to prevent an infinite loop hanging the app, we limit
        // to a maximum of three iterations. If more invalidations are created, then those will be
        // handled on the next call to `update_windows`.
        for iter in 1..=3 {
            let invalidation = self.take_all_invalidations_for_window(window_id);

            // Always build the scene at least once, even if there
            // are no updated views.
            if invalidation.updated.is_empty() && !invalidation.redraw_requested && iter > 1 {
                break;
            }

            {
                let mut presenter = presenter.borrow_mut();
                presenter.invalidate(invalidation, self);

                // Skip rendering if a dimension is 0. This must happen after
                // invalidation to ensure the proper views are still invalidated.
                let size = window.size();
                if size.x() == 0. || size.y() == 0. {
                    log::debug!(
                        "Received window_id={window_id:?} with window size {size:?}. Skipping render"
                    );
                    break;
                }

                // Build the scene.  As part of this process, we compute element
                // origins and bounding boxes.
                //
                // In the future, we should separate out position computation from
                // scene building, as we don't need to do the latter on each
                // iteration of this loop.
                scene = presenter.build_scene(
                    size,
                    window.backing_scale_factor(),
                    window.max_texture_dimension_2d(),
                    self,
                );

                // Cache the last position cache after rendering.
                self.presentation
                    .last_frame_position_cache
                    .insert(window_id, presenter.position_cache().clone());
            }

            // Synthesize a MouseMoved event in case any elements
            // are now hovered due to being in a different location.
            let last_mouse_moved_event: Rc<RefCell<Option<Event>>> =
                self.get_last_mouse_moved_event(window_id);
            let event = last_mouse_moved_event.borrow();
            if let Some(event) = event
                .as_ref()
                .and_then(Event::to_synthetic_mouse_move_event)
            {
                self.handle_window_event(event, window_id, presenter.clone());
            }
        }

        scene
    }

    /// Simulates rendering a frame for the given window.
    #[cfg(test)]
    pub fn simulate_render_frame(&mut self, window_id: WindowId) {
        let Some(window) = self.windows().platform_window(window_id) else {
            return;
        };
        self.build_scene(window_id, window.as_ctx());
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn simulate_window_event(
        &mut self,
        event: Event,
        window_id: WindowId,
        presenter: Rc<RefCell<Presenter>>,
    ) -> bool {
        self.handle_window_event(event, window_id, presenter)
            .handled
    }

    fn handle_window_event(
        &mut self,
        mut event: Event,
        window_id: WindowId,
        presenter: Rc<RefCell<Presenter>>,
    ) -> crate::windowing::EventDispatchResult {
        let mut event_munger: Box<EventMunger> = Box::new(|_, _| {});
        std::mem::swap(&mut event_munger, &mut self.event_munger);
        // Give the app a chance to modify the event first. The closure had to be moved out of
        // `self.event_munger` to avoid a double-borrow error, so swapped it into `event_munger`
        // local variable, then swap it back after.
        event_munger(&mut event, self);
        std::mem::swap(&mut event_munger, &mut self.event_munger);

        let mut keystroke_handled = false;
        if self.key_bindings_enabled(window_id) {
            // Checks (and possibly dispatches) for actions with a matching keybinding
            if let Event::KeyDown {
                keystroke,
                is_composing,
                ..
            } = &event
            {
                if let Some(focused_view_id) = self.focused_view_id(window_id) {
                    let responder_chain = self.view_ancestors(window_id, focused_view_id);
                    match self.dispatch_keystroke(
                        window_id,
                        &responder_chain,
                        keystroke,
                        *is_composing,
                    ) {
                        Ok(handled) => {
                            keystroke_handled = handled;
                        }
                        Err(error) => {
                            log::error!("error dispatching keystroke: {error}");
                        }
                    }
                }
            }
        }

        let mut result = crate::windowing::EventDispatchResult::default();
        if !keystroke_handled {
            result = self.handle_non_keybound_event(event.clone(), window_id, presenter.clone());
        }

        let handled = keystroke_handled || result.handled;

        // Only dispatch `self_or_child_interacted_with` if:
        // (1) the event was handled by a view in the responder chain, and
        // (2) the event is a valid interaction (we exclude mouse and scroll movements to reduce noise)
        if handled && !matches!(event, Event::MouseMoved { .. } | Event::ScrollWheel { .. }) {
            if let Some(focused_view_id) = self.focused_view_id(window_id) {
                let responder_chain = self.view_ancestors(window_id, focused_view_id);
                self.dispatch_self_or_child_interacted_with(window_id, &responder_chain);
            }
        }

        crate::windowing::EventDispatchResult {
            handled,
            soft_keyboard_requested: result.soft_keyboard_requested,
        }
    }

    fn handle_non_keybound_event(
        &mut self,
        event: Event,
        window_id: WindowId,
        presenter: Rc<RefCell<Presenter>>,
    ) -> crate::windowing::EventDispatchResult {
        let log_level = match &event {
            // If the action comes from the MouseMoved or ScrollWheel events,
            // dispatch it at the `trace` log level so it doesn't clutter the
            // logs by default.
            Event::MouseMoved { .. } | Event::ScrollWheel { .. } => log::Level::Trace,
            _ => {
                // Update last user action timestamp for non-hover events
                App::record_last_active_timestamp();
                log::Level::Info
            }
        };
        let dispatch_result = presenter.borrow_mut().dispatch_event(event, self);

        // Iterate through all timers, and put a task onto the main thread for each
        // to call back and notify the view when the timer triggers.
        for (timer_id, view_to_notify) in dispatch_result.notify_timers_to_set.iter() {
            let (timer_id, view_to_notify) = (*timer_id, *view_to_notify);
            let weak_app = self.weak_self.clone();
            let task = self.foreground.spawn(async move {
                Timer::after(view_to_notify.notify_at - Instant::now()).await;
                if let Some(app) = weak_app.upgrade() {
                    let mut app = app.borrow_mut();
                    if app.notify_tasks.remove(&timer_id).is_some() {
                        log::info!(
                            "notifying view observers and updating windows for timer id {timer_id}"
                        );
                        app.notify_view_observers(window_id, view_to_notify.view_id);
                        // Note that for the hoverable delay this triggers the appropriate
                        // behavior because the window stores the last mouse moved event and
                        // dispatches it on every window redraw.
                        // See the on_window_invalidated callback
                        app.update_windows();
                    }
                }
            });
            self.notify_tasks.insert(timer_id, task);
        }

        for timer_id in dispatch_result.notify_timers_to_clear {
            // Dropping the task should be sufficient to cancel it.
            self.notify_tasks.remove(&timer_id);
        }

        for view_id in dispatch_result.notified {
            self.notify_view_observers(window_id, view_id)
        }

        for action in dispatch_result.actions.into_iter().rev() {
            let responder_chain = self.view_ancestors(window_id, action.view_id);
            match action.kind {
                DispatchedActionKind::Legacy { name, arg } => {
                    self.dispatch_action(
                        window_id,
                        &responder_chain,
                        name,
                        arg.as_ref(),
                        log_level,
                    );
                }
                DispatchedActionKind::Typed(action) => {
                    self.dispatch_typed_action(
                        window_id,
                        &responder_chain,
                        action.as_ref(),
                        log_level,
                    );
                }
            }
        }

        match dispatch_result.cursor_update {
            Some(CursorUpdate::Set {
                cursor, view_id, ..
            }) => self.set_cursor_shape(cursor, window_id, view_id),
            Some(CursorUpdate::Reset) => self.reset_cursor(),
            _ => {}
        }

        crate::windowing::EventDispatchResult {
            handled: dispatch_result.handled,
            soft_keyboard_requested: dispatch_result.soft_keyboard_requested,
        }
    }

    fn load_fallback_family_and_redraw(
        &mut self,
        window_id: WindowId,
        fallback_family: ExternalFontFamily,
        request_sources: Vec<RequestedFallbackFontSource>,
        asset_sources: Vec<AssetSource>,
    ) {
        let asset_cache = AssetCache::as_ref(self);

        let mut font_family_bytes: Vec<Vec<u8>> = Vec::new();
        // Get the raw font bytes from the loaded assets.
        for asset in asset_sources
            .into_iter()
            .map(|source| asset_cache.load_asset::<fonts::FontBytes>(source))
        {
            match asset {
                AssetState::Loaded { data } => {
                    // TODO(PLAT-746): Update API for loading a font family to
                    // take an `Rc`, so we don't have to clone the font data.
                    font_family_bytes.push(data.0.clone());
                }
                AssetState::Evicted => {
                    log::warn!("Unable to load requested fallback font because it was evicted");
                }
                AssetState::FailedToLoad(e) => {
                    log::warn!("Unable to load requested fallback font: {e:?}");
                }
                AssetState::Loading { .. } => {
                    log::error!("Fallback font asset should not be in a loading state");
                }
            }
        }

        // Early return if we were not able to load any of the font assets for
        // this family.
        // TODO(PLAT-760): Implement a retry mechanism if we load some but not
        // all of the font assets for the family. Currently we will load the
        // partial font family into the cache and will not try loading again.
        if font_family_bytes.is_empty() {
            log::warn!(
                "Failed to load any fonts for the family {}",
                &fallback_family.name
            );
            return;
        }

        // Insert the family into the font cache.
        if !self
            .font_cache()
            .is_fallback_family_loaded(fallback_family.name)
        {
            if let Err(e) = fonts::Cache::handle(self).update(self, |cache, _| {
                cache.load_fallback_family_from_bytes(fallback_family, font_family_bytes)
            }) {
                log::warn!("Unable to load fallback family from bytes: {e:?}");
                return;
            }

            // TODO(PLAT-747): Ideally the font cache itself is a model and can
            // handle emitting these events.
            FallbackFontModel::handle(self).update(self, |model, ctx| {
                model.loaded_fallback_font(ctx);
            });
        }

        // Clear the glyph/layout caches before redrawing. These caches must
        // be cleared even if the fallback family has previously been loaded.
        for source in request_sources {
            match source {
                RequestedFallbackFontSource::GlyphForChar(key) => {
                    fonts::Cache::handle(self).update(self, |cache, _| {
                        cache.remove_glyphs_by_char_entry(key);
                    });
                }
                RequestedFallbackFontSource::Line(key) => {
                    if let Some(presenter) = self.presenter(window_id) {
                        presenter.borrow().text_layout_cache().remove_line(&key);
                    }
                }
                RequestedFallbackFontSource::TextFrame(key) => {
                    if let Some(presenter) = self.presenter(window_id) {
                        presenter
                            .borrow()
                            .text_layout_cache()
                            .remove_text_frame(&key);
                    }
                }
            }
        }

        // Trigger a redraw on the window.
        self.window_invalidations
            .entry(window_id)
            .or_default()
            .redraw_requested = true;
        self.update_windows();
    }

    pub(crate) fn load_requested_fallback_families(&mut self, window_id: WindowId) {
        let Some(fallback_font_source_provider) = self.fallback_font_source_provider.as_ref()
        else {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                log::warn!(
                    "No fallback_font_source_provider registered; cannot load fallback fonts"
                );
            });

            return;
        };

        let requested_fallback_families =
            fonts::Cache::as_ref(self).take_requested_fallback_families();
        let asset_cache = AssetCache::as_ref(self);

        for (fallback_family, request_sources) in requested_fallback_families {
            let mut asset_sources = Vec::with_capacity(fallback_family.font_urls.len());
            let mut futures = Vec::with_capacity(fallback_family.font_urls.len());
            for fallback_font in fallback_family.font_urls.as_ref() {
                let asset_source = fallback_font_source_provider(fallback_font);
                let asset = asset_cache.load_asset::<fonts::FontBytes>(asset_source.clone());

                // If the font is loading, collect the future so we can wait
                // for it to resolve.
                if let AssetState::Loading { ref handle } = asset {
                    if let Some(future) = handle.when_loaded(asset_cache) {
                        futures.push(future);
                    }
                }
                // We need to load the asset again once the future has resolved,
                // so collect the asset source.
                asset_sources.push(asset_source);
            }

            let weak_self_clone = self.weak_self.clone();
            self.foreground
                .spawn(async move {
                    join_all(futures).await;

                    let Some(app) = weak_self_clone.upgrade() else {
                        return;
                    };
                    let mut app = app.borrow_mut();

                    app.load_fallback_family_and_redraw(
                        window_id,
                        fallback_family,
                        request_sources,
                        asset_sources,
                    );
                })
                .detach();
        }
    }

    /// Returns the cached element position from the last rendered frame, if there is one.
    pub fn element_position_by_id_at_last_frame<S>(
        &self,
        window_id: WindowId,
        id: S,
    ) -> Option<RectF>
    where
        S: AsRef<str>,
    {
        self.presentation
            .last_frame_position_cache
            .get(&window_id)
            .and_then(|position_cache| position_cache.get_position(id))
    }
}

#[cfg(test)]
#[path = "mod_gui_tests.rs"]
mod mod_gui_tests;
