//! Type-erased plumbing behind `ModelContext::spawn` and `ViewContext::spawn`.
//!
//! The typed entry points erase the entity type, the future's output type, and the callbacks
//! before they call [`AppContext::spawn_abortable_erased`]. The channel, the background task, and
//! the main-thread relay are therefore compiled once, instead of once per entity and output type.

use std::any::Any;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Aborted};
use warp_errors::report_error;

use super::TaskCallback;
use crate::r#async::{BoxFuture, SpawnableOutput, SpawnedFutureHandle};
use crate::{AppContext, EntityId, WindowId};

/// The output of a spawned future, with its type erased.
#[cfg(not(target_family = "wasm"))]
pub(super) type ErasedOutput = Box<dyn Any + Send>;
/// The output of a spawned future, with its type erased.
#[cfg(target_family = "wasm")]
pub(super) type ErasedOutput = Box<dyn Any>;

type ModelResolveCallback = dyn FnOnce(&mut dyn Any, ErasedOutput, &mut AppContext, EntityId);
type ModelAbortCallback = dyn FnOnce(&mut dyn Any, &mut AppContext, EntityId);
type ViewResolveCallback =
    dyn FnOnce(&mut dyn Any, ErasedOutput, &mut AppContext, WindowId, EntityId);
type ViewAbortCallback = dyn FnOnce(&mut dyn Any, &mut AppContext, WindowId, EntityId);

/// The main-thread callbacks for a spawned future, and the entity they run against.
pub(super) enum SpawnCallbacks {
    Model {
        model_id: EntityId,
        on_resolve: Box<ModelResolveCallback>,
        on_abort: Box<ModelAbortCallback>,
    },
    View {
        window_id: WindowId,
        view_id: EntityId,
        on_resolve: Box<ViewResolveCallback>,
        on_abort: Box<ViewAbortCallback>,
    },
}

/// Boxes a future's output so that futures with different output types share one type.
pub(super) fn erase_output<O>(future: BoxFuture<'static, O>) -> BoxFuture<'static, ErasedOutput>
where
    O: 'static + SpawnableOutput,
{
    Box::pin(async move { Box::new(future.await) as ErasedOutput })
}

/// Recovers the typed output of a spawned future inside its `on_resolve` callback.
pub(super) fn downcast_output<O: 'static>(output: ErasedOutput) -> O {
    *output
        .downcast()
        .expect("the spawn entry point fixes the output type")
}

/// Recovers the typed entity inside a spawned future's callback.
pub(super) fn downcast_entity<T: 'static>(entity: &mut dyn Any) -> &mut T {
    entity
        .downcast_mut()
        .expect("the spawn entry point fixes the entity type")
}

/// Unpacks what the main thread received for a spawned future. Returns the output if the future
/// resolved, or `None` if it was aborted.
fn resolved_output(received: Box<dyn Any>) -> Option<ErasedOutput> {
    let received: Result<Result<ErasedOutput, Aborted>, oneshot::Canceled> = *received
        .downcast()
        .expect("spawn_abortable_erased fixes the relayed type");
    match received {
        Ok(Ok(output)) => Some(output),
        Ok(Err(Aborted)) => None,
        Err(oneshot::Canceled) => {
            report_error!("sender unexpectedly dropped before receiver");
            None
        }
    }
}

impl AppContext {
    /// Runs a future on the background executor, then runs one callback on the main thread:
    /// `on_resolve` with the future's output, or `on_abort` if the future was aborted.
    pub(super) fn spawn_abortable_erased(
        &mut self,
        future: BoxFuture<'static, ErasedOutput>,
        callbacks: SpawnCallbacks,
    ) -> SpawnedFutureHandle {
        let (tx, rx) = oneshot::channel();

        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        self.background_executor()
            .spawn_boxed(Box::pin(async move {
                let abortable = Abortable::new(future, abort_registration);
                if tx.send(abortable.await).is_err() {
                    report_error!("Error sending background task result to main thread");
                }
            }))
            .detach();

        let (done_tx, done_rx) = oneshot::channel();
        let task_id = self.spawn_local(rx);
        let task_callback = match callbacks {
            SpawnCallbacks::Model {
                model_id,
                on_resolve,
                on_abort,
            } => TaskCallback::ModelFromFuture {
                model_id,
                callback: Box::new(move |model, received, app, model_id| {
                    match resolved_output(received) {
                        Some(output) => on_resolve(model, output, app, model_id),
                        None => on_abort(model, app, model_id),
                    }
                    let _ = done_tx.send(());
                }),
            },
            SpawnCallbacks::View {
                window_id,
                view_id,
                on_resolve,
                on_abort,
            } => TaskCallback::ViewFromFuture {
                window_id,
                view_id,
                callback: Box::new(move |view, received, app, window_id, view_id| {
                    match resolved_output(received) {
                        Some(output) => on_resolve(view, output, app, window_id, view_id),
                        None => on_abort(view, app, window_id, view_id),
                    }
                    let _ = done_tx.send(());
                }),
            },
        };
        self.task_callbacks.insert(task_id, task_callback);

        let done = async move {
            if done_rx.await.is_err() {
                report_error!("sender unexpectedly dropped before receiver");
            }
        };
        let future_id = self.register_spawned_future(done.boxed());
        SpawnedFutureHandle::new(abort_handle, future_id)
    }
}
