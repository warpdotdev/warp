use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use async_executor::LocalExecutor;
use futures::future::{BoxFuture, LocalBoxFuture};
use futures::{Future, FutureExt};
use futures_util::future::{AbortHandle, Abortable};

use crate::platform;
use crate::r#async::executor::Error;

pub type ForegroundTask = async_task::Task<()>;

pub struct BackgroundTask {
    inner: Option<tokio::task::JoinHandle<()>>,
}

impl BackgroundTask {
    pub fn abort(&self) {
        if let Some(inner) = &self.inner {
            inner.abort();
        }
    }

    pub fn detach(self) {
        // Nothing to do here; dropping the join handle will cause the task
        // to be detached.
    }
}

impl Future for BackgroundTask {
    type Output = Result<(), tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.inner {
            Some(inner) => inner.poll_unpin(cx),
            None => Poll::Pending,
        }
    }
}

pub enum Foreground {
    Platform {
        not_send_or_sync: PhantomData<Rc<()>>, // Make sure the type is `!Send` and `!Sync`.
        delegate: Arc<dyn platform::DispatchDelegate>,
    },
    Test {
        executor: LocalExecutor<'static>,
    },
}

impl Foreground {
    pub fn platform(delegate: Arc<dyn platform::DispatchDelegate>) -> Result<Self, Error> {
        if delegate.is_main_thread() {
            Ok(Self::Platform {
                not_send_or_sync: PhantomData,
                delegate,
            })
        } else {
            Err(Error::NotOnMainThread)
        }
    }

    pub fn test() -> Self {
        Self::Test {
            executor: LocalExecutor::new(),
        }
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
        match self {
            Foreground::Platform {
                not_send_or_sync: _,
                delegate: platform,
            } => {
                let platform = platform.clone();
                let schedule = move |task: async_task::Runnable| platform.run_on_main_thread(task);
                let (runnable, handle) = async_task::spawn_local(future, schedule);
                runnable.schedule();
                handle
            }
            Foreground::Test { executor } => executor.spawn(future),
        }
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

    pub async fn run<T>(&'_ self, future: impl Future<Output = T>) -> T {
        match self {
            Foreground::Platform {
                not_send_or_sync: _,
                delegate: _,
            } => unimplemented!("only the test executor can be run"),
            Foreground::Test { executor } => executor.run(future).await,
        }
    }
}

pub struct Background {
    runtime: Option<tokio::runtime::Runtime>,
}

impl Drop for Background {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            // Cancel all running tasks immediately instead of blocking until they complete.
            runtime.shutdown_background();
        }
    }
}

impl Default for Background {
    fn default() -> Self {
        let num_threads = if cfg!(any(test, feature = "integration_tests")) {
            // For tests, limit each test to a single background thread.
            // When running unit tests via [`App::test()`] on machines with
            // many logical cores, the time it takes to spawning the background
            // threads can far exceed the time it takes to actually run the
            // test.
            1
        } else {
            // In production, create a thread for each logical CPU core,
            // maximizing our possible parallelism.
            num_cpus::get()
        };

        Self::new(num_threads, |i| format!("background-executor-{i}"))
    }
}

impl Background {
    pub fn new(
        num_threads: usize,
        name_fn: impl Fn(usize) -> String + Send + Sync + 'static,
    ) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(num_threads)
            .thread_name_fn(move || {
                static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
                let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
                name_fn(id)
            })
            .enable_all()
            .build()
            .expect("should not fail to create tokio runtime for background executor");

        Self {
            runtime: Some(runtime),
        }
    }

    /// Schedule an asynchronous task to run on a background thread.
    ///
    /// If you have a boxed future, use `spawn_boxed` instead.
    pub fn spawn(&self, future: impl Send + Future<Output = ()> + 'static) -> BackgroundTask {
        self.spawn_boxed(future.boxed())
    }

    /// Schedule an asynchronous task to run on a background thread.
    ///
    /// This takes in a boxed future in order to avoid monomorphizing the
    /// underlying task implementation.  `spawn_boxed` generates significantly
    /// less code than a generic implementation, with no noticeable performance
    /// impact.
    pub fn spawn_boxed(&self, future: BoxFuture<'static, ()>) -> BackgroundTask {
        let inner = match &self.runtime {
            Some(runtime) => Some(runtime.spawn(future)),
            None => {
                log::error!("tried to spawn a background task after the executor was shut down");
                None
            }
        };
        BackgroundTask { inner }
    }

    /// Schedules an abortable asynchronous task to run on a background thread.
    ///
    /// This is the same as `spawn()` except the task may be aborted using the returned
    /// [`AbortHandle`].
    pub fn spawn_abortable(
        &self,
        future: impl Send + Future<Output = ()> + 'static,
    ) -> (BackgroundTask, AbortHandle) {
        let (handle, registration) = AbortHandle::new_pair();
        let task = self.spawn(Abortable::new(future, registration).map(|_| ()));
        (task, handle)
    }
}
