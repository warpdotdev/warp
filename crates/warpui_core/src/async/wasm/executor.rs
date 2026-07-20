use std::sync::Arc;

use futures::future::LocalBoxFuture;
use futures::{Future, FutureExt};
use futures_util::future::{AbortHandle, Abortable};
use tracing::Instrument as _;
use wasm_bindgen_futures::spawn_local;

use crate::r#async::executor::Error;
use crate::platform;

/// A handle to a task that will run on the main thread.
pub struct ForegroundTask;

impl ForegroundTask {
    /// Detaches the task to let it keep running in the background.
    pub fn detach(self) {}
}

/// A handle to a task that will run in the background.
///
/// In practice, for wasm, this task will be executed on the singular main
/// thread that all JavaScript code is run on.
pub struct BackgroundTask;

impl BackgroundTask {
    /// Detaches the task to let it keep running in the background.
    pub fn detach(self) {}
}

/// An executor that can be used to run tasks on the main thread.
pub struct Foreground;

impl Foreground {
    pub fn platform(_delegate: Arc<dyn platform::DispatchDelegate>) -> Result<Self, Error> {
        Ok(Foreground)
    }

    pub fn test() -> Self {
        Foreground
    }

    /// Schedule an asynchronous task to run on the main thread.
    ///
    /// If you have a boxed future, use `spawn_boxed` instead.
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> ForegroundTask {
        self.spawn_boxed(future.boxed_local())
    }

    /// Schedule an asynchronous task to run on the main thread.
    ///
    /// This takes in a boxed future in order to avoid monomorphizing the
    /// underlying task implementation.  `spawn_boxed` generates significantly
    /// less code than a generic implementation, with no noticeable performance
    /// impact.
    pub fn spawn_boxed(&self, future: LocalBoxFuture<'static, ()>) -> ForegroundTask {
        spawn_local(future.instrument(tracing::Span::current()));
        ForegroundTask
    }

    /// Schedules an abortable asynchronous task to run on the main thread.
    ///
    /// This is the same as `spawn()` except the task may be aborted using the returned
    /// [`AbortHandle`].
    pub fn spawn_abortable(
        &self,
        future: impl Future<Output = ()> + 'static,
    ) -> (ForegroundTask, AbortHandle) {
        let (handle, registration) = AbortHandle::new_pair();
        let task = self.spawn(Abortable::new(future, registration).map(|_| ()));
        (task, handle)
    }

    pub async fn run<T: 'static>(&'_ self, future: impl Future<Output = T> + 'static) -> T {
        future.await
    }
}

/// An executor that can be used to run background tasks.
///
/// In practice, for wasm, these tasks will be executed on the singular main
/// thread that all JavaScript code is run on.
pub struct Background;

impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}

impl Background {
    pub fn new() -> Self {
        Background
    }

    /// Schedule an asynchronous task to run on a background thread.
    ///
    /// If you have a boxed future, use `spawn_boxed` instead.
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> BackgroundTask {
        self.spawn_boxed(future.boxed_local())
    }

    /// Schedule an asynchronous task to run on a background thread.
    ///
    /// This takes in a boxed future in order to avoid monomorphizing the
    /// underlying task implementation.  `spawn_boxed` generates significantly
    /// less code than a generic implementation, with no noticeable performance
    /// impact.
    pub fn spawn_boxed(&self, future: LocalBoxFuture<'static, ()>) -> BackgroundTask {
        spawn_local(future.instrument(tracing::Span::current()));
        BackgroundTask
    }

    /// Schedules an abortable asynchronous task to run on a background thread.
    ///
    /// This is the same as `spawn()` except the task may be aborted using the returned
    /// [`AbortHandle`].
    pub fn spawn_abortable(
        &self,
        future: impl Future<Output = ()> + 'static,
    ) -> (BackgroundTask, AbortHandle) {
        let (handle, registration) = AbortHandle::new_pair();
        let task = self.spawn(Abortable::new(future, registration).map(|_| ()));
        (task, handle)
    }
}
