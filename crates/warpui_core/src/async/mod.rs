use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::{FutureExt as _, pin_mut};
use futures_util::stream::AbortHandle;

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        mod wasm;
        use wasm as imp;
    } else {
        mod native;
        use native as imp;
    }
}

pub use futures_util::future::LocalBoxFuture;
// Re-export a variety of symbols from the internal implementation modules.
pub use imp::{BoxFuture, Spawnable, SpawnableOutput, Stream, Timer, TransportStream, block_on};
use thiserror::Error;

pub mod executor {
    #[derive(thiserror::Error, Debug)]
    pub enum Error {
        #[error("constructed off of the main thread")]
        NotOnMainThread,
    }

    pub use super::imp::executor::*;
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub struct FutureId(usize);

static NEXT_FUTURE_ID: AtomicUsize = AtomicUsize::new(0);
impl FutureId {
    /// \return the next view ID. Note the first return is 0.
    #[allow(clippy::new_without_default)]
    pub(super) fn new() -> FutureId {
        let raw = NEXT_FUTURE_ID.fetch_add(1, Ordering::Relaxed);
        FutureId(raw)
    }
}

/// A handle to a future that was spawned on an executor via `ctx#spawn`.
/// The handle can be used to abort the future using `#abort`. In tests, the
/// future ID can be used to await the spawned future.
#[derive(Debug, Clone)]
pub struct SpawnedFutureHandle {
    abort_handle: AbortHandle,
    future_id: FutureId,
}

impl SpawnedFutureHandle {
    /// Abort the spawned future associated with this handle.
    pub fn abort(&self) {
        self.abort_handle.abort()
    }

    pub fn abort_handle(&self) -> AbortHandle {
        self.abort_handle.clone()
    }

    /// The `FutureID` associated with this `SpawnedFuture`. In tests, this can be used to
    /// await the spawned future.
    pub fn future_id(&self) -> FutureId {
        self.future_id
    }

    pub fn new(abort_handle: AbortHandle, future_id: FutureId) -> Self {
        Self {
            abort_handle,
            future_id,
        }
    }
}

pub struct SpawnedLocalStream {
    #[allow(dead_code)]
    future: LocalBoxFuture<'static, ()>,
}

impl SpawnedLocalStream {
    #[cfg(test)]
    pub(crate) fn into_future(self) -> LocalBoxFuture<'static, ()> {
        self.future
    }

    pub(crate) fn new(future: LocalBoxFuture<'static, ()>) -> Self {
        Self { future }
    }
}

/// This trait impl allows us to use `Background` as an executor in some executor-agnostic libraries,
impl futures_util::task::Spawn for executor::Background {
    fn spawn_obj(
        &self,
        future: futures::task::FutureObj<'static, ()>,
    ) -> Result<(), futures::task::SpawnError> {
        self.spawn(future).detach();
        Ok(())
    }

    fn status(&self) -> Result<(), futures::task::SpawnError> {
        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("Timed out waiting for future")]
pub struct TimeoutError;

pub trait FutureExt: Future {
    /// Converts a future into one that will time out with an error after a
    /// given duration.
    ///
    /// Note that this timeout can only occur while the future is at an await
    /// point, so futures wrapped in this way must periodically yield back to
    /// the executor.
    fn with_timeout(
        self,
        timeout: Duration,
    ) -> impl Future<Output = Result<<Self as Future>::Output, TimeoutError>>;
}

impl<F: Future> FutureExt for F {
    async fn with_timeout(
        self,
        timeout: Duration,
    ) -> Result<<Self as Future>::Output, TimeoutError> {
        let fut = self.fuse();
        pin_mut!(fut);

        let mut timeout = Timer::after(timeout).fuse();

        futures::select! {
            value = fut => Ok(value),
            _ = timeout => Err(TimeoutError),
        }
    }
}
