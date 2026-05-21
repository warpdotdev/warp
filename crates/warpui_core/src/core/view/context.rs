use std::{any::Any, marker::PhantomData, rc::Rc, sync::Arc};

use futures::future::{AbortHandle, Abortable};
use futures::{Future, FutureExt};
use pathfinder_geometry::rect::RectF;
use thiserror::Error;

use crate::modals::{AlertDialogWithCallbacks, ModalButton, ViewModalCallback};
use crate::platform::{
    file_picker::{FilePickerConfiguration, FilePickerError},
    Cursor, SaveFilePickerConfiguration, TerminationMode,
};
use crate::r#async::SpawnableOutput;
use crate::windowing::WindowManager;
use crate::{
    accessibility::AccessibilityContent,
    core::{Observation, Subscription, SubscriptionKey, TaskCallback},
    fonts::Cache as FontCache,
    notification::{NotificationSendError, RequestPermissionsOutcome, UserNotification},
    r#async::{
        executor::{Background, Foreground},
        SpawnedFutureHandle, SpawnedLocalStream,
    },
    Action, AppContext, Effect, Entity, EntityId, ModelAsRef, ModelContext, ModelHandle,
    UpdateModel, WindowId,
};
use crate::{GetSingletonModelHandle, ReadModel};

use super::{
    handle::{AnyViewHandle, ReadView, UpdateView, ViewAsRef, ViewHandle, WeakViewHandle},
    TypedActionView, View,
};

/// Structure that combines view identifiers and a handle to the application
/// context/application state.
pub struct ViewContext<'a, T: ?Sized> {
    app: &'a mut AppContext,
    window_id: WindowId,
    view_id: EntityId,
    view_type: PhantomData<T>,
}

impl<'a, T: View> ViewContext<'a, T> {
    pub(in crate::core) fn new(
        app: &'a mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Self {
        Self {
            app,
            window_id,
            view_id,
            view_type: PhantomData,
        }
    }

    /// Adds a callback that will be invoked immediately after the next frame is drawn.
    /// Note that the callback is only invoked once and is discarded after it is called.
    pub fn on_next_frame_drawn<F: 'static + Fn()>(&mut self, callback: F) {
        self.app.on_next_frame_drawn(self.window_id, callback);
    }

    pub fn handle(&self) -> WeakViewHandle<T> {
        WeakViewHandle::new(self.view_id)
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn view_id(&self) -> EntityId {
        self.view_id
    }

    pub fn font_cache(&self) -> &FontCache {
        self.app.font_cache()
    }

    pub fn windows(&self) -> &WindowManager {
        self.app.windows()
    }

    pub fn disable_key_bindings_dispatching(&mut self) {
        let window_id = self.window_id;
        log::info!("disabling actions for window id {window_id}");
        self.disable_key_bindings(window_id)
    }

    pub fn enable_key_bindings_dispatching(&mut self) {
        let window_id = self.window_id;
        log::info!("enabling actions for window id {window_id}");
        self.enable_key_bindings(window_id)
    }

    // Function to check if the parent view is focused.
    pub fn is_self_focused(&self) -> bool {
        self.app.check_view_focused(self.window_id, &self.view_id)
    }

    pub fn is_self_or_child_focused(&self) -> bool {
        self.app
            .check_view_or_child_focused(self.window_id, &self.view_id)
    }

    pub fn element_position_by_id<S>(&self, id: S) -> Option<RectF>
    where
        S: AsRef<str>,
    {
        let presenter = self.app.presenter(self.window_id);

        if let Some(presenter) = presenter {
            let borrowed_presenter = presenter.borrow();
            borrowed_presenter.position_cache().get_position(id)
        } else {
            None
        }
    }

    pub fn focus<S: View>(&mut self, handle: &ViewHandle<S>) {
        let handle: AnyViewHandle = handle.into();
        self.app.pending_effects.push_back(Effect::Focus {
            window_id: handle.window_id(self.app),
            view_id: handle.id(),
        });
    }

    pub fn focus_self(&mut self) {
        self.app.pending_effects.push_back(Effect::Focus {
            window_id: self.window_id,
            view_id: self.view_id,
        });
    }

    pub fn add_model<S, F>(&mut self, build_model: F) -> ModelHandle<S>
    where
        S: Entity,
        F: FnOnce(&mut ModelContext<S>) -> S,
    {
        self.app.add_model(build_model)
    }

    pub fn add_view<S, F>(&mut self, build_view: F) -> ViewHandle<S>
    where
        S: View,
        F: FnOnce(&mut ViewContext<S>) -> S,
    {
        self.app.add_view(self.window_id, build_view)
    }

    pub fn add_typed_action_view<V, F>(&mut self, build_view: F) -> ViewHandle<V>
    where
        V: TypedActionView + View,
        F: FnOnce(&mut ViewContext<V>) -> V,
    {
        // Add a new view, and set the parent view as the current context's view.
        self.app
            .add_typed_action_view_with_parent(self.window_id, build_view, self.view_id)
    }

    pub fn add_option_view<S, F>(&mut self, build_view: F) -> Option<ViewHandle<S>>
    where
        S: View,
        F: FnOnce(&mut ViewContext<S>) -> Option<S>,
    {
        self.app.add_option_view(self.window_id, build_view)
    }

    pub fn subscribe_to_model<E, F>(&mut self, handle: &ModelHandle<E>, mut callback: F)
    where
        E: Entity,
        E::Event: 'static,
        F: 'static + FnMut(&mut T, ModelHandle<E>, &E::Event, &mut ViewContext<T>),
    {
        let emitter_handle = handle.downgrade();
        self.app
            .subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromView {
                window_id: self.window_id,
                view_id: self.view_id,
                callback: Box::new(move |view, payload, app, window_id, view_id| {
                    if let Some(emitter_handle) = emitter_handle.upgrade(app) {
                        let model = view.downcast_mut().expect("downcast is type safe");
                        let payload = payload.downcast_ref().expect("downcast is type safe");
                        let mut ctx = ViewContext::new(app, window_id, view_id);
                        callback(model, emitter_handle, payload, &mut ctx);
                    }
                }),
            });
    }

    pub fn subscribe_to_view<V, F>(&mut self, handle: &ViewHandle<V>, mut callback: F)
    where
        V: View,
        V::Event: 'static,
        F: 'static + FnMut(&mut T, ViewHandle<V>, &V::Event, &mut ViewContext<T>),
    {
        let emitter_handle = handle.downgrade();

        self.app
            .subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromView {
                window_id: self.window_id,
                view_id: self.view_id,
                callback: Box::new(move |view, payload, app, window_id, view_id| {
                    if let Some(emitter_handle) = emitter_handle.upgrade(app) {
                        let model = view.downcast_mut().expect("downcast is type safe");
                        let payload = payload.downcast_ref().expect("downcast is type safe");
                        let mut ctx = ViewContext::new(app, window_id, view_id);
                        callback(model, emitter_handle, payload, &mut ctx);
                    }
                }),
            });
    }

    pub fn unsubscribe_to_view<V>(&mut self, handle: &ViewHandle<V>)
    where
        V: View,
        V::Event: 'static,
    {
        let target_entity = handle.id();

        // If we're currently emitting events for this entity, defer the unsubscribe.
        if let Some(ref mut pending) = self.app.pending_unsubscribes {
            if pending.entity_id == target_entity {
                pending
                    .keys
                    .insert(SubscriptionKey::View(self.window_id, self.view_id));

                // Remove subscriptions created earlier in this emission so subscribe-then-unsubscribe ordering is preserved.
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    self.app.subscriptions.entry(target_entity)
                {
                    entry.get_mut().retain(|subscription| match subscription {
                        Subscription::FromModel { .. } | Subscription::FromApp { .. } => true,
                        Subscription::FromView {
                            window_id, view_id, ..
                        } => *window_id != self.window_id || *view_id != self.view_id,
                    });

                    if entry.get().is_empty() {
                        entry.remove();
                    }
                }

                return;
            }
        }

        // Otherwise process immediately.
        self.app
            .subscriptions
            .entry(target_entity)
            .or_default()
            .retain(|subscription| match subscription {
                Subscription::FromModel { .. } | Subscription::FromApp { .. } => true,
                Subscription::FromView {
                    window_id, view_id, ..
                } => *window_id != self.window_id || *view_id != self.view_id,
            });
    }

    pub fn unsubscribe_to_model<E>(&mut self, handle: &ModelHandle<E>)
    where
        E: Entity,
        E::Event: 'static,
    {
        let target_entity = handle.id();

        // If we're currently emitting events for this entity, defer the unsubscribe.
        if let Some(ref mut pending) = self.app.pending_unsubscribes {
            if pending.entity_id == target_entity {
                pending
                    .keys
                    .insert(SubscriptionKey::View(self.window_id, self.view_id));

                // Remove subscriptions created earlier in this emission so subscribe-then-unsubscribe ordering is preserved.
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    self.app.subscriptions.entry(target_entity)
                {
                    entry.get_mut().retain(|subscription| match subscription {
                        Subscription::FromModel { .. } | Subscription::FromApp { .. } => true,
                        Subscription::FromView {
                            window_id, view_id, ..
                        } => *window_id != self.window_id || *view_id != self.view_id,
                    });

                    if entry.get().is_empty() {
                        entry.remove();
                    }
                }

                return;
            }
        }

        // Otherwise process immediately.
        self.app
            .subscriptions
            .entry(target_entity)
            .or_default()
            .retain(|subscription| match subscription {
                Subscription::FromModel { .. } | Subscription::FromApp { .. } => true,
                Subscription::FromView {
                    window_id, view_id, ..
                } => *window_id != self.window_id || *view_id != self.view_id,
            })
    }

    /// Prompt the user to pick file path(s) in the OS native file picker.
    pub fn open_file_picker(
        &mut self,
        callback: impl FnOnce(Result<Vec<String>, FilePickerError>, &mut ViewContext<T>)
            + Send
            + Sync
            + 'static,
        config: FilePickerConfiguration,
    ) {
        let window_id = self.window_id;
        let view_id = self.view_id;
        self.app.open_file_picker(
            move |result, app| {
                let mut view_context = ViewContext::new(app, window_id, view_id);
                callback(result, &mut view_context)
            },
            config,
        )
    }

    /// Prompt the user to save a file with the OS native save file dialog.
    ///
    /// The callback receives the chosen path (or `None` if cancelled), a mutable
    /// reference to the owning view, and its `ViewContext`.
    pub fn open_save_file_picker(
        &mut self,
        callback: impl FnOnce(Option<String>, &mut T, &mut ViewContext<T>) + Send + Sync + 'static,
        config: SaveFilePickerConfiguration,
    ) {
        let view_id = self.view_id;
        self.app.open_save_file_picker(
            move |path, app| {
                let weak = WeakViewHandle::<T>::new(view_id);
                if let Some(handle) = weak.upgrade(app) {
                    app.update_view(&handle, |view, ctx| {
                        callback(path, view, ctx);
                    });
                }
            },
            config,
        )
    }

    /// Emits the provided event on this `View`.
    ///
    /// Unlike DOM events, these events don't bubble or otherwise automatically
    /// propagate themselves up the view hierarchy.  In order for another view
    /// to receive any events emitted by this view, the receiver will need to
    /// explicitly subscribe to this view's events by calling
    /// [`Self::subscribe_to_view()`][^note].
    ///
    /// [^note]: This subscription is on a per-instance basis, not on a per-type
    ///     basis.
    pub fn emit(&mut self, payload: T::Event) {
        self.app.pending_effects.push_back(Effect::Event {
            entity_id: self.view_id,
            payload: Box::new(payload),
        });
    }

    /// When all else fails, `emit_a11y_content` comes to the rescue! In our UI framework, some stuff is just an Event, and not an Action. Sometimes a different View emits the event, while another one handles it… So instead of solving this, we simply use `emit_a11y_content(&mut self, content: AccessibilityContent)` on demand, meaning, whenever something meaningful happens and it’s not related to Action or View focusing, we should emit a11y content. A good example for this is announcing that a new update is available.
    ///
    /// ### When and how to use it?
    /// Whenever we feel like it’s important to make an announcement about events in the app. Note that this requires a ViewContext.
    pub fn emit_a11y_content(&mut self, content: AccessibilityContent) {
        let verbosity = self.a11y_verbosity;
        self.platform_delegate
            .set_accessibility_contents(content.with_verbosity(verbosity));
    }

    /// Delegates to the OS to request the user attention for the given window. For mac this bounces the
    /// icon in the dock. If the window is already focused this is a noop.
    pub fn request_user_attention(&mut self) {
        let window_id = self.window_id;
        self.app.request_user_attention(window_id);
    }

    /// Global actions are being phased out. Prefer dispatching typed actions instead of global actions.
    /// Dispatch a global action to be handled by the registered handler
    ///
    /// Note: The dispatch of the global action will be registered as an effect and flushed after
    /// the current view update is complete. This will ensure that the view has been re-inserted
    /// into the `AppContext`, so it will be accessible to the global action, if necessary
    #[track_caller]
    pub fn dispatch_global_action<A: Any>(&mut self, name: &'static str, arg: A) {
        let location = std::panic::Location::caller();
        self.app.pending_effects.push_back(Effect::GlobalAction {
            name,
            location,
            arg: Box::new(arg),
        });
    }

    pub fn dispatch_typed_action(&mut self, action: &dyn Action) {
        let window_id = self.window_id;
        let view_id = self.view_id;
        self.dispatch_typed_action_for_view(window_id, view_id, action);
    }

    /// Defers dispatching a typed action until effects are flushed.
    ///
    /// This is useful to avoid re-entrant view updates (e.g. triggering UI updates
    /// while a view in the responder chain is still mid-update).
    pub fn dispatch_typed_action_deferred<A: Action + 'static>(&mut self, action: A) {
        self.app.pending_effects.push_back(Effect::TypedAction {
            window_id: self.window_id,
            view_id: self.view_id,
            action: Box::new(action),
        });
    }

    pub fn observe<S, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S: Entity,
        F: 'static + FnMut(&mut T, ModelHandle<S>, &mut ViewContext<T>),
    {
        self.app
            .observations
            .entry(handle.id())
            .or_default()
            .push(Observation::FromView {
                window_id: self.window_id,
                view_id: self.view_id,
                callback: Box::new(move |view, observed_id, app, window_id, view_id| {
                    let view = view.downcast_mut().expect("downcast is type safe");
                    let observed = ModelHandle::new(observed_id, &app.ref_counts);
                    let mut ctx = ViewContext::new(app, window_id, view_id);
                    callback(view, observed, &mut ctx);
                }),
            });
    }

    /// Notifies the framework that this view is dirty and needs to be
    /// re-rendered.
    ///
    /// "Dirtiness" only applies to this specific instance, and not the entire
    /// view hierarchy rooted at this view.  Each dirty child view also needs to
    /// have `ctx.notify()` called on the child's `ViewContext` in order for the
    /// child to be re-rendered.
    pub fn notify(&mut self) {
        self.app
            .pending_effects
            .push_back(Effect::ViewNotification {
                window_id: self.window_id,
                view_id: self.view_id,
            });
    }

    /// Requests permissions to send desktop notifications. The `on_completion callback` can be invoked to
    /// propagate the outcome of the request (accepted/denied/other) back to the app.
    ///
    /// ## Platform-Specific
    /// * Linux: Always calls the `on_completion_callback` with a value of [`RequestPermissionsOutcome::Accepted`].
    pub fn request_desktop_notification_permissions<F>(&mut self, on_completion_callback: F)
    where
        F: 'static + Send + Sync + FnOnce(&mut T, RequestPermissionsOutcome, &mut ViewContext<T>),
    {
        let view_id = self.view_id;
        let window_id = self.window_id;
        self.app.request_desktop_notification_permissions(
            view_id,
            window_id,
            on_completion_callback,
        );
    }

    /// Sends a desktop notification. The `on_error_callback` can be invoked to
    /// propagate an error to the view that initiated the notification send.
    pub fn send_desktop_notification<F>(&mut self, content: UserNotification, on_error_callback: F)
    where
        F: 'static + Send + Sync + FnOnce(&mut T, NotificationSendError, &mut ViewContext<T>),
    {
        let view_id = self.view_id;
        let window_id = self.window_id;
        self.app
            .send_desktop_notification(content, view_id, window_id, on_error_callback);
    }

    /// Schedules a future to run on the main thread, invoking a callback on the
    /// main thread upon completion.
    ///
    /// The callback receives the output of the future, if any, in addition to
    /// mutable references to the spawning view and its context, allowing for
    /// dirtying of the view (via [`Self::notify()`]) if appropriate.
    ///
    /// This is private to [`ViewContext`] because we shouldn't ever need to
    /// poll a future on the main thread.  Currently, the only use is by
    /// [`Self::spawn()`] in order to pass the results of the background task to
    /// a callback executed on the main thread.
    ///
    /// TODO(vorporeal): Determine how best to eliminate this function and move
    ///     the relevant logic into `spawn()`.
    fn spawn_local<S, F, U>(&mut self, future: S, callback: F) -> impl Future<Output = ()>
    where
        S: 'static + Future,
        F: 'static + FnOnce(&mut T, S::Output, &mut ViewContext<T>) -> U,
        U: 'static,
    {
        let (tx, rx) = futures::channel::oneshot::channel();

        let task_id = self.app.spawn_local(future);

        self.app.task_callbacks.insert(
            task_id,
            TaskCallback::ViewFromFuture {
                window_id: self.window_id,
                view_id: self.view_id,
                callback: Box::new(move |view, output, app, window_id, view_id| {
                    let view = view.as_any_mut().downcast_mut().expect("this downcast should never fail, as correct typing is statically enforced via the generic parameters on spawn_local");
                    let output = *output.downcast().expect("this downcast should never fail, as correct typing is statically enforced via the generic parameters on spawn_local");
                    let result =
                        callback(view, output, &mut ViewContext::new(app, window_id, view_id));
                    let _ = tx.send(result);
                }),
            },
        );

        async move {
            if rx.await.is_err() {
                log::error!("sender unexpectedly dropped before receiver");
            }
        }
    }

    /// Schedules a future to run on a background thread, invoking a callback on
    /// the _main_ thread upon completion.
    ///
    /// This function is useful in situations where a long-running process needs
    /// to occur (e.g.: a network request), after which the view needs to be
    /// updated based on the result.
    ///
    /// The callback receives the output of the future, if any, in addition to
    /// mutable references to the spawning view and its context, allowing for
    /// dirtying of the view (via [`Self::notify`]) if appropriate.
    ///
    /// The future can be aborted by calling `abort` on the returned `SpawnedFutureHandle`. Note the
    /// future will only be aborted the _next_ time the future is polled.
    ///
    /// See [`Self::spawn_abortable`] for an alternative version of this function that accepts an
    /// `on_abort` function that is called when the future is aborted.
    pub fn spawn<S, F, U>(&mut self, future: S, callback: F) -> SpawnedFutureHandle
    where
        S: crate::r#async::Spawnable,
        <S as Future>::Output: crate::r#async::SpawnableOutput,
        F: 'static + FnOnce(&mut T, <S as Future>::Output, &mut ViewContext<T>) -> U,
        U: 'static,
    {
        self.spawn_abortable::<S, _, _>(
            future,
            |view, output, ctx| {
                callback(view, output, ctx);
            },
            |_, _| {},
        )
    }

    /// Schedules a future to run on a background thread, invoking the `on_resolve`
    /// callback on the _main_ thread upon completion. If the future is aborted, the
    /// `on_abort` function is called.
    ///
    /// This function is useful in situations where a long-running process needs
    /// to occur (e.g.: a network request), after which the view needs to be
    /// updated based on the result.
    ///
    /// The `on_resolve` callback receives the output of the future, if any, in addition to
    /// mutable references to the spawning view and its context, allowing for
    /// dirtying of the model (via [`Self::notify`]) if appropriate.
    ///
    /// The future can be aborted by calling `abort` on the returned `SpawnedFutureHandle`. Note, a
    /// future is not immediately killed on `abort`--it will only be aborted once the future's
    /// `poll` method returns.
    ///
    /// See [`Self::spawn`] for an alternative version of this function that doesn't
    /// require a callback if/when the future is aborted.
    pub fn spawn_abortable<S, F, A>(
        &mut self,
        future: S,
        on_resolve: F,
        on_abort: A,
    ) -> SpawnedFutureHandle
    where
        S: crate::r#async::Spawnable,
        <S as Future>::Output: crate::r#async::SpawnableOutput,
        F: 'static + FnOnce(&mut T, <S as Future>::Output, &mut ViewContext<T>),
        A: 'static + FnOnce(&mut T, &mut ViewContext<T>),
    {
        let (tx, rx) = futures::channel::oneshot::channel();

        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        self.app
            .background_executor()
            .spawn_boxed(Box::pin(async move {
                let abortable = Abortable::new(future, abort_registration);
                if tx.send(abortable.await).is_err() {
                    log::error!("Error sending background task result to main thread",);
                }
            }))
            .detach();

        let future = self.spawn_local(rx, |view, rx_result, ctx| {
            let output = match rx_result {
                Ok(output) => output,
                Err(_) => {
                    log::error!("sender unexpectedly dropped before receiver");
                    on_abort(view, ctx);
                    return;
                }
            };

            // Call the appropriate callback based on the output of resolving the future. If the
            // future returned `Ok`, the future was not aborted so we can call `on_resolve`. If
            // the future returned `Err`--the future was aborted.
            match output {
                Ok(output) => on_resolve(view, output, ctx),
                Err(_) => on_abort(view, ctx),
            }
        });

        let future_id = self.app.register_spawned_future(future.boxed());
        SpawnedFutureHandle::new(abort_handle, future_id)
    }

    /// Schedules a stream to be polled on the main thread, invoking callbacks
    /// upon the production of each item and upon the completion of the stream.
    ///
    /// This function is useful in situations where a view wants to process a
    /// stream of events (say, a debounced stream of mouse movements) and update
    /// itself in response to each.
    ///
    /// The callbacks receive mutable references to the spawning view and its
    /// context, allowing for updating of the view's internal state and dirtying
    /// it (via [`Self::notify`]) if appropriate.
    pub fn spawn_stream_local<S, F, G>(
        &mut self,
        stream: S,
        mut on_item: F,
        mut on_done: G,
    ) -> SpawnedLocalStream
    where
        S: 'static + crate::r#async::Stream,
        S::Item: SpawnableOutput,
        F: 'static + FnMut(&mut T, S::Item, &mut ViewContext<T>),
        G: 'static + FnMut(&mut T, &mut ViewContext<T>),
    {
        let (tx, rx) = futures::channel::oneshot::channel();

        let task_id = self.app.spawn_stream_local(stream, tx);
        self.app.task_callbacks.insert(
            task_id,
            TaskCallback::ViewFromStream {
                window_id: self.window_id,
                view_id: self.view_id,
                on_item: Box::new(move |view, output, app, window_id, view_id| {
                    let view = view.as_any_mut().downcast_mut().expect("this downcast should never fail, as correct typing is statically enforced via the generic parameters on spawn_local");
                    let output = *output.downcast().expect("this downcast should never fail, as correct typing is statically enforced via the generic parameters on spawn_local");
                    let mut ctx = ViewContext::new(app, window_id, view_id);
                    on_item(view, output, &mut ctx);
                }),
                on_done: Box::new(move |view, app, window_id, view_id| {
                    let view = view.as_any_mut().downcast_mut().expect("this downcast should never fail, as correct typing is statically enforced via the generic parameters on spawn_local");
                    let mut ctx = ViewContext::new(app, window_id, view_id);
                    on_done(view, &mut ctx);
                }),
            },
        );

        SpawnedLocalStream::new(
            async move {
                if rx.await.is_err() {
                    log::error!("sender unexpectedly dropped before receiver");
                }
            }
            .boxed_local(),
        )
    }

    pub fn close_window(&mut self) {
        self.app
            .windows()
            .close_window_async(self.window_id, TerminationMode::Cancellable);
    }

    /// Minimizes the window which this View is in.
    pub fn minimize_window(&mut self) {
        if let Some(window) = self.app.windows().platform_window(self.window_id) {
            window.minimize();
        }
    }

    /// Maximizes the window which this View is in, unless that window is already maximized, then it
    /// restores it, i.e. "un-maximizes" it.
    pub fn toggle_maximized_window(&mut self) {
        if let Some(window) = self.app.windows().platform_window(self.window_id) {
            window.toggle_maximized();
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(window) = self.app.windows().platform_window(self.window_id) {
            window.toggle_fullscreen();
        }
    }

    pub fn foreground_executor(&self) -> &Rc<Foreground> {
        self.app.foreground_executor()
    }

    pub fn background_executor(&self) -> &Arc<Background> {
        self.app.background_executor()
    }

    /// Create a window showing a modal dialog native to the platform. The modal will synchronously
    /// block all other interactions with the app until dismissed. Each button can have a callback
    /// attached to it in the [`crate::modals::ModalButton`] struct.
    pub fn show_native_platform_modal(
        &mut self,
        view_alert: AlertDialogWithCallbacks<ViewModalCallback<T>>,
    ) {
        let weak_handle = self.handle();
        let app_alert = AlertDialogWithCallbacks::for_app(
            view_alert.message_text,
            view_alert.info_text,
            view_alert
                .button_data
                .into_iter()
                .map(|button| {
                    let weak_handle = self.handle();
                    ModalButton::for_app(button.title, move |app| {
                        if let Some(handle) = weak_handle.upgrade(app) {
                            app.update_view(&handle, |view, ctx| {
                                (button.on_click)(view, ctx);
                            });
                        }
                    })
                })
                .collect(),
            move |app| {
                if let Some(handle) = weak_handle.upgrade(app) {
                    app.update_view(&handle, |view, ctx| {
                        (view_alert.on_disable)(view, ctx);
                    });
                }
            },
        );
        self.app.show_native_platform_modal(app_alert);
    }

    pub fn set_cursor_shape(&mut self, cursor: Cursor) {
        self.app
            .set_cursor_shape(cursor, self.window_id, self.view_id)
    }

    pub fn reset_cursor(&mut self) {
        self.app.reset_cursor()
    }

    /// Creates a handle which background tasks can use to spawn work for this view. Spawned tasks
    /// are executed on the main thread in the context of the view, and results are sent back to
    /// the background task.
    ///
    /// Note that the spawner *does not* keep a strong reference to the view. If the view is
    /// dropped, any pending or future tasks will be discarded.
    pub fn spawner(&mut self) -> ViewSpawner<T> {
        let (task_tx, task_rx) = async_channel::unbounded();
        let (completion_tx, _completion_rx) = futures::channel::oneshot::channel();

        let task_id = self.app.spawn_stream_local(task_rx, completion_tx);
        self.app.task_callbacks.insert(
            task_id,
            TaskCallback::ViewFromStream {
                window_id: self.window_id,
                view_id: self.view_id,
                on_item: Box::new(move |view, task, app, window_id, view_id| {
                    let view = view
                        .as_any_mut()
                        .downcast_mut()
                        .expect("unexpected view type");
                    let task: ViewTask<T> = *task
                        .downcast()
                        .expect("task from spawner should be ViewTask<T>");
                    let mut ctx = ViewContext::new(app, window_id, view_id);
                    task(view, &mut ctx);
                }),
                on_done: Box::new(move |_view, _app, _window_id, _view_id| {}),
            },
        );

        ViewSpawner {
            task_sender: task_tx,
        }
    }
}

impl<T> std::ops::Deref for ViewContext<'_, T> {
    type Target = AppContext;

    fn deref(&self) -> &Self::Target {
        self.app
    }
}

impl<T> std::ops::DerefMut for ViewContext<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.app
    }
}

/// A task which must run in the context of a view of type `V`.
type ViewTask<V> = Box<dyn FnOnce(&mut V, &mut ViewContext<V>) + Send + 'static>;

/// A handle for spawning view tasks from background threads.
pub struct ViewSpawner<V> {
    task_sender: async_channel::Sender<ViewTask<V>>,
}

impl<V> ViewSpawner<V> {
    /// Spawn a task that will execute on the main thread, in the context of a view.
    pub async fn spawn<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut V, &mut ViewContext<V>) -> R + Send + 'static,
    ) -> Result<R, ViewDropped> {
        let (tx, rx) = futures::channel::oneshot::channel();

        self.task_sender
            .send(Box::new(move |me, ctx| {
                let result = work(me, ctx);
                // If the background task has dropped the receiver, then we don't need to send
                // the result, and there's no one to inform regardless.
                let _ = tx.send(result);
            }))
            .await
            .map_err(|_| ViewDropped)?;

        rx.await.map_err(|_| ViewDropped)
    }
}

/// Error returned when a view has been dropped, and so references to it are invalid.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("View has been dropped")]
pub struct ViewDropped;

impl<V> ModelAsRef for ViewContext<'_, V> {
    fn model<T: Entity>(&self, handle: &ModelHandle<T>) -> &T {
        self.app.model(handle)
    }
}

impl<V> ReadModel for ViewContext<'_, V> {
    fn read_model<T, F, S>(&self, handle: &ModelHandle<T>, read: F) -> S
    where
        T: Entity,
        F: FnOnce(&T, &AppContext) -> S,
    {
        self.app.read_model(handle, read)
    }
}

impl<V: View> UpdateModel for ViewContext<'_, V> {
    fn update_model<T, F, S>(&mut self, handle: &ModelHandle<T>, update: F) -> S
    where
        T: Entity,
        F: FnOnce(&mut T, &mut ModelContext<T>) -> S,
    {
        self.app.update_model(handle, update)
    }
}

impl<V: View> ViewAsRef for ViewContext<'_, V> {
    fn view<T: View>(&self, handle: &ViewHandle<T>) -> &T {
        self.app.view(handle)
    }

    fn try_view<T: View>(&self, handle: &ViewHandle<T>) -> Option<&T> {
        self.app.try_view(handle)
    }
}

impl<V: View> UpdateView for ViewContext<'_, V> {
    fn update_view<T, F, S>(&mut self, handle: &ViewHandle<T>, update: F) -> S
    where
        T: View,
        F: FnOnce(&mut T, &mut ViewContext<T>) -> S,
    {
        self.app.update_view(handle, update)
    }
}

impl<V: View> ReadView for ViewContext<'_, V> {
    fn read_view<T, F, S>(&self, handle: &ViewHandle<T>, read: F) -> S
    where
        T: View,
        F: FnOnce(&T, &AppContext) -> S,
    {
        self.app.read_view(handle, read)
    }
}

impl<V: View> GetSingletonModelHandle for ViewContext<'_, V> {
    fn get_singleton_model_handle<T: crate::SingletonEntity>(&self) -> ModelHandle<T> {
        self.app.get_singleton_model_handle()
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
