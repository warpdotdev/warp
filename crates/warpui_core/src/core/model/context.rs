use std::{any::Any, future::Future, marker::PhantomData, sync::Arc};

use crate::{
    r#async::{SpawnableOutput, Timer},
    windowing::WindowManager,
    ReadModel, ReadView, UpdateView, View, ViewAsRef, ViewContext, ViewHandle, WeakModelHandle,
};
use anyhow::Result;
use futures::{
    stream::{AbortHandle, Abortable},
    FutureExt,
};
use thiserror::Error;

use crate::{
    accessibility::AccessibilityContent,
    core::{Observation, Subscription, SubscriptionKey, TaskCallback},
    r#async::{executor, SpawnedFutureHandle, SpawnedLocalStream},
    AppContext, Effect, Entity, EntityId, GetSingletonModelHandle, ModelAsRef, ModelHandle,
    RequestState, RetryOption, UpdateModel,
};

/// Error returned when a model has been dropped, and so references to it are invalid.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("Model has been dropped")]
pub struct ModelDropped;

/// Structure that combines model identifiers and a handle to the application
/// context/application state.
pub struct ModelContext<'a, T: ?Sized> {
    app: &'a mut AppContext,
    model_id: EntityId,
    model_type: PhantomData<T>,
}

impl<'a, T: Entity> ModelContext<'a, T> {
    pub(in crate::core) fn new(app: &'a mut AppContext, model_id: EntityId) -> Self {
        Self {
            app,
            model_id,
            model_type: PhantomData,
        }
    }

    pub fn handle(&self) -> WeakModelHandle<T> {
        WeakModelHandle::new(self.model_id)
    }

    pub fn background_executor(&self) -> Arc<executor::Background> {
        self.app.background_executor().clone()
    }

    pub fn model_id(&self) -> EntityId {
        self.model_id
    }

    pub fn windows(&self) -> &WindowManager {
        self.app.windows()
    }

    pub fn add_model<S, F>(&mut self, build_model: F) -> ModelHandle<S>
    where
        S: Entity,
        F: FnOnce(&mut ModelContext<S>) -> S,
    {
        self.app.add_model(build_model)
    }

    pub fn subscribe_to_model<S: Entity, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S::Event: 'static,
        F: 'static + FnMut(&mut T, &S::Event, &mut ModelContext<T>),
    {
        self.app
            .subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromModel {
                model_id: self.model_id,
                callback: Box::new(move |model, payload, app, model_id| {
                    let model = model.downcast_mut().expect("downcast is type safe");
                    let payload: &<S as Entity>::Event =
                        payload.downcast_ref().expect("downcast is type safe");
                    let mut ctx = ModelContext::new(app, model_id);
                    callback(model, payload, &mut ctx);
                }),
            });
    }

    pub fn unsubscribe_from_model<E>(&mut self, handle: &ModelHandle<E>)
    where
        E: Entity,
        E::Event: 'static,
    {
        let target_entity = handle.id();

        // If we're currently emitting events for this entity, defer the unsubscribe.
        if let Some(ref mut pending) = self.app.pending_unsubscribes {
            if pending.entity_id == target_entity {
                pending.keys.insert(SubscriptionKey::Model(self.model_id));

                // Remove subscriptions created earlier in this emission so subscribe-then-unsubscribe ordering is preserved.
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    self.app.subscriptions.entry(target_entity)
                {
                    entry.get_mut().retain(|subscription| match subscription {
                        Subscription::FromView { .. } | Subscription::FromApp { .. } => true,
                        Subscription::FromModel { model_id, .. } => *model_id != self.model_id,
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
                Subscription::FromView { .. } | Subscription::FromApp { .. } => true,
                Subscription::FromModel { model_id, .. } => *model_id != self.model_id,
            })
    }

    pub fn subscribe_to_view<V, F>(&mut self, handle: &ViewHandle<V>, mut callback: F)
    where
        V: View,
        V::Event: 'static,
        F: 'static + FnMut(&mut T, &V::Event, &mut ModelContext<T>),
    {
        self.app
            .subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromModel {
                model_id: self.model_id,
                callback: Box::new(move |model, payload, app, model_id| {
                    let model = model.downcast_mut().expect("downcast is type safe");
                    let payload = payload.downcast_ref().expect("downcast is type safe");
                    let mut ctx = ModelContext::new(app, model_id);
                    callback(model, payload, &mut ctx);
                }),
            });
    }

    pub fn unsubscribe_from_view<V>(&mut self, handle: &ViewHandle<V>)
    where
        V: View,
        V::Event: 'static,
    {
        let target_entity = handle.id();

        // If we're currently emitting events for this entity, defer the unsubscribe.
        if let Some(ref mut pending) = self.app.pending_unsubscribes {
            if pending.entity_id == target_entity {
                pending.keys.insert(SubscriptionKey::Model(self.model_id));

                // Remove subscriptions created earlier in this emission so subscribe-then-unsubscribe ordering is preserved.
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    self.app.subscriptions.entry(target_entity)
                {
                    entry.get_mut().retain(|subscription| match subscription {
                        Subscription::FromView { .. } | Subscription::FromApp { .. } => true,
                        Subscription::FromModel { model_id, .. } => *model_id != self.model_id,
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
                Subscription::FromView { .. } | Subscription::FromApp { .. } => true,
                Subscription::FromModel { model_id, .. } => *model_id != self.model_id,
            })
    }

    pub fn emit(&mut self, payload: T::Event) {
        self.app.pending_effects.push_back(Effect::Event {
            entity_id: self.model_id,
            payload: Box::new(payload),
        });
    }

    /// Global actions are being phased out. Prefer dispatching typed actions instead of global actions.
    /// Dispatch a global action to be handled by the registered handler
    ///
    /// Note: The dispatch of the global action will be registered as an effect and flushed after
    /// the current model update is complete. This will ensure that the model has been re-inserted
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

    pub fn observe<S, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S: Entity,
        F: 'static + FnMut(&mut T, ModelHandle<S>, &mut ModelContext<T>),
    {
        self.app
            .observations
            .entry(handle.id())
            .or_default()
            .push(Observation::FromModel {
                model_id: self.model_id,
                callback: Box::new(move |model, observed_id, app, model_id| {
                    let model = model.downcast_mut().expect("downcast is type safe");
                    let observed = ModelHandle::new(observed_id, &app.ref_counts);
                    let mut ctx = ModelContext::new(app, model_id);
                    callback(model, observed, &mut ctx);
                }),
            });
    }

    pub fn notify(&mut self) {
        // If the last effect is a model notification for this model,
        // don't add another one.
        if let Some(Effect::ModelNotification { model_id }) = self.app.pending_effects.back() {
            if *model_id == self.model_id {
                return;
            }
        }

        self.app
            .pending_effects
            .push_back(Effect::ModelNotification {
                model_id: self.model_id,
            });
    }

    /// Emit AccessibilityContent
    /// This method lets propagate any content to the screen reader on demand (doesn't need to be
    /// tied with actions or specific events).
    pub fn emit_a11y_content(&mut self, content: AccessibilityContent) {
        self.app
            .platform_delegate
            .set_accessibility_contents(content);
    }

    // Only public in crate::core so it can be used by ui/src/core/mod_test.rs.
    pub(in crate::core) fn spawn_local<S, F, U>(
        &mut self,
        future: S,
        callback: F,
    ) -> impl Future<Output = ()>
    where
        S: 'static + Future,
        F: 'static + FnOnce(&mut T, S::Output, &mut ModelContext<T>) -> U,
        U: 'static,
    {
        let (tx, rx) = futures::channel::oneshot::channel();

        let task_id = self.app.spawn_local(future);

        self.app.task_callbacks.insert(
            task_id,
            TaskCallback::ModelFromFuture {
                model_id: self.model_id,
                callback: Box::new(move |model, output, app, model_id| {
                    let model = model.downcast_mut().unwrap();
                    let output = *output.downcast().unwrap();
                    let result = callback(model, output, &mut ModelContext::new(app, model_id));
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

    /// Schedules a future that returns a Result type to run on the background thread.
    /// If the future resolves to success (Ok), call the set callback with RequestState::RequestSucceeded.
    /// If the future fails and we still have remaining retry counts, call the set callback
    /// with RequestState::RequestFailedRetryPending and retry based on the RetryOption.
    /// Otherwise, call the set callback with RequestState::RequestFailed.
    pub fn spawn_with_retry_on_error<P, S, F, M>(
        &mut self,
        future_closure: P,
        retry_option: RetryOption,
        callback: F,
    ) -> SpawnedFutureHandle
    where
        P: 'static + FnMut() -> S,
        S: crate::r#async::Spawnable + Future<Output = Result<M>>,
        <S as Future>::Output: crate::r#async::SpawnableOutput,
        F: 'static + FnMut(&mut T, RequestState<M>, &mut ModelContext<T>),
    {
        self.spawn_with_retry_on_error_when(future_closure, retry_option, |_| true, callback)
    }

    /// Like [`Self::spawn_with_retry_on_error`], but additionally consults `should_retry` on
    /// each failure. The chain stops immediately (calling the callback with
    /// [`RequestState::RequestFailed`]) when `should_retry` returns false, even if retries
    /// remain on the [`RetryOption`]. Use this for errors that are known to be permanent so
    /// they don't issue redundant requests — e.g. classify a 403/404 with
    /// `is_transient_http_error` and skip retries.
    pub fn spawn_with_retry_on_error_when<P, S, R, F, M>(
        &mut self,
        mut future_closure: P,
        mut retry_option: RetryOption,
        mut should_retry: R,
        mut callback: F,
    ) -> SpawnedFutureHandle
    where
        P: 'static + FnMut() -> S,
        S: crate::r#async::Spawnable + Future<Output = Result<M>>,
        <S as Future>::Output: crate::r#async::SpawnableOutput,
        R: 'static + FnMut(&anyhow::Error) -> bool,
        F: 'static + FnMut(&mut T, RequestState<M>, &mut ModelContext<T>),
    {
        let future = future_closure();

        self.spawn(future, move |me, res, ctx| match res {
            Ok(success) => {
                callback(me, RequestState::RequestSucceeded(success), ctx);
            }
            Err(e) => {
                if retry_option.remaining_retry_count == 0 || !should_retry(&e) {
                    callback(me, RequestState::RequestFailed(e), ctx);
                } else {
                    callback(me, RequestState::RequestFailedRetryPending(e), ctx);

                    let _ = ctx.spawn(
                        async move { Timer::after(retry_option.duration()).await },
                        move |_, _, ctx| {
                            retry_option.advance();
                            ctx.spawn_with_retry_on_error_when(
                                future_closure,
                                retry_option,
                                should_retry,
                                callback,
                            )
                        },
                    );
                }
            }
        })
    }

    /// Schedules a future to run on a background thread, invoking a callback on
    /// the _main_ thread upon completion.
    ///
    /// This function is useful in situations where a long-running process needs
    /// to occur (e.g.: a network request), after which the model needs to be
    /// updated based on the result.
    ///
    /// The callback receives the output of the future, if any, in addition to
    /// mutable references to the spawning view and its context, allowing for
    /// dirtying of the model (via [`Self::notify`]) if appropriate.
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
        F: 'static + FnOnce(&mut T, S::Output, &mut ModelContext<T>) -> U,
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
    /// to occur (e.g.: a network request), after which the model needs to be
    /// updated based on the result.
    ///
    /// The `on_resolve` callback receives the output of the future, if any, in addition to
    /// mutable references to the spawning model and its context, allowing for
    /// dirtying of the view (via [`Self::notify`]) if appropriate.
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
        F: 'static + FnOnce(&mut T, S::Output, &mut ModelContext<T>),
        A: 'static + FnOnce(&mut T, &mut ModelContext<T>),
    {
        let (tx, rx) = futures::channel::oneshot::channel();

        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        self.app
            .background_executor()
            .spawn_boxed(Box::pin(async move {
                let abortable = Abortable::new(future, abort_registration);
                let result = abortable.await;
                if tx.send(result).is_err() {
                    log::error!("Error sending background task result to main thread",);
                }
            }))
            .detach();

        let future = self.spawn_local(rx, |model, rx_result, ctx| {
            let output = match rx_result {
                Ok(output) => output,
                Err(_) => {
                    log::error!("sender unexpectedly dropped before receiver");
                    on_abort(model, ctx);
                    return;
                }
            };

            // Call the appropriate callback based on the output of resolving the future. If the
            // future returned `Ok`, the future was not aborted so we can call `on_resolve`. If
            // the future returned `Err`--the future was aborted.
            match output {
                Ok(output) => on_resolve(model, output, ctx),
                Err(_) => on_abort(model, ctx),
            }
        });

        let future_id = self.app.register_spawned_future(future.boxed());
        SpawnedFutureHandle::new(abort_handle, future_id)
    }

    /// Creates a handle which background tasks can use to spawn work for this model. Spawned tasks
    /// are executed on the main thread in the context of the model, and results are sent back to
    /// the background task.
    ///
    /// Note that the spawner *does not* keep a strong reference to the model. If the model is
    /// dropped, any pending or future tasks will be discarded.
    pub fn spawner(&mut self) -> ModelSpawner<T> {
        let (task_tx, task_rx) = async_channel::unbounded();
        let (completion_tx, _completion_rx) = futures::channel::oneshot::channel();

        let task_id = self.app.spawn_stream_local(task_rx, completion_tx);
        self.app.task_callbacks.insert(
            task_id,
            TaskCallback::ModelFromStream {
                model_id: self.model_id,
                on_item: Box::new(move |model, task, app, model_id| {
                    let model = model.downcast_mut().expect("unexpected model type");
                    let task: ModelTask<T> = *task
                        .downcast()
                        .expect("task from spawner should be ModelTask<T>");
                    let mut ctx = ModelContext::new(app, model_id);
                    task(model, &mut ctx);
                }),
                on_done: Box::new(move |_model, _app, _model_id| {}),
            },
        );

        ModelSpawner {
            task_sender: task_tx,
        }
    }

    pub fn spawn_stream_local<S, F, G>(
        &mut self,
        stream: S,
        mut on_item: F,
        on_done: G,
    ) -> SpawnedLocalStream
    where
        S: 'static + crate::r#async::Stream,
        S::Item: SpawnableOutput,
        F: 'static + FnMut(&mut T, S::Item, &mut ModelContext<T>),
        G: 'static + FnOnce(&mut T, &mut ModelContext<T>),
    {
        let (tx, rx) = futures::channel::oneshot::channel();

        let task_id = self.app.spawn_stream_local(stream, tx);
        self.app.task_callbacks.insert(
            task_id,
            TaskCallback::ModelFromStream {
                model_id: self.model_id,
                on_item: Box::new(move |model, output, app, model_id| {
                    let model = model.downcast_mut().unwrap();
                    let output = *output.downcast().unwrap();
                    let mut ctx = ModelContext::new(app, model_id);
                    on_item(model, output, &mut ctx);
                }),
                on_done: Box::new(move |model, app, model_id| {
                    let model = model.downcast_mut().unwrap();
                    let mut ctx = ModelContext::new(app, model_id);
                    on_done(model, &mut ctx);
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
}

impl<T> std::ops::Deref for ModelContext<'_, T> {
    type Target = AppContext;

    fn deref(&self) -> &Self::Target {
        self.app
    }
}

impl<T> std::ops::DerefMut for ModelContext<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.app
    }
}

impl<M> ViewAsRef for ModelContext<'_, M> {
    fn view<T: View>(&self, handle: &ViewHandle<T>) -> &T {
        self.app.view(handle)
    }

    fn try_view<T: View>(&self, handle: &ViewHandle<T>) -> Option<&T> {
        self.app.try_view(handle)
    }
}

impl<M> ReadView for ModelContext<'_, M> {
    fn read_view<T, F, S>(&self, handle: &ViewHandle<T>, read: F) -> S
    where
        T: View,
        F: FnOnce(&T, &AppContext) -> S,
    {
        self.app.read_view(handle, read)
    }
}

impl<M> UpdateView for ModelContext<'_, M> {
    fn update_view<T, F, S>(&mut self, handle: &ViewHandle<T>, update: F) -> S
    where
        T: View,
        F: FnOnce(&mut T, &mut ViewContext<T>) -> S,
    {
        self.app.update_view(handle, update)
    }
}

impl<M> ModelAsRef for ModelContext<'_, M> {
    fn model<T: Entity>(&self, handle: &ModelHandle<T>) -> &T {
        self.app.model(handle)
    }
}

impl<M> ReadModel for ModelContext<'_, M> {
    fn read_model<T, F, S>(&self, handle: &ModelHandle<T>, read: F) -> S
    where
        T: Entity,
        F: FnOnce(&T, &AppContext) -> S,
    {
        self.app.read_model(handle, read)
    }
}

impl<M> UpdateModel for ModelContext<'_, M> {
    fn update_model<T, F, S>(&mut self, handle: &ModelHandle<T>, update: F) -> S
    where
        T: Entity,
        F: FnOnce(&mut T, &mut ModelContext<T>) -> S,
    {
        self.app.update_model(handle, update)
    }
}

impl<M> GetSingletonModelHandle for ModelContext<'_, M> {
    fn get_singleton_model_handle<T: crate::SingletonEntity>(&self) -> ModelHandle<T> {
        self.app.get_singleton_model_handle()
    }
}

/// A task which must run in the context of a model of type `M`.
type ModelTask<M> = Box<dyn FnOnce(&mut M, &mut ModelContext<M>) + Send + 'static>;

/// A handle for spawning model tasks from background threads.
pub struct ModelSpawner<M> {
    task_sender: async_channel::Sender<ModelTask<M>>,
}

impl<M> Clone for ModelSpawner<M> {
    fn clone(&self) -> Self {
        Self {
            task_sender: self.task_sender.clone(),
        }
    }
}

impl<M> ModelSpawner<M> {
    /// Spawn a task that will execute on the main thread, in the context of a model.
    pub async fn spawn<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut M, &mut ModelContext<M>) -> R + Send + 'static,
    ) -> Result<R, ModelDropped> {
        let (tx, rx) = futures::channel::oneshot::channel();

        self.task_sender
            .send(Box::new(move |me, ctx| {
                let result = work(me, ctx);
                // If the background task has dropped the receiver, then we don't need to send
                // the result, and there's no one to inform regardless.
                let _ = tx.send(result);
            }))
            .await
            .map_err(|_| ModelDropped)?;

        rx.await.map_err(|_| ModelDropped)
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
