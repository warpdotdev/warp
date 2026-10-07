pub mod executor;

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Future;
use futures_lite::FutureExt;
pub use futures_lite::future::block_on;
// There is no such thing as a background thread in wasm, so all futures are local.
pub use futures_util::future::LocalBoxFuture as BoxFuture;
use gloo::timers::future::TimeoutFuture;
use instant::Instant;

// Define a trait that is implemented by all types to allow us to not
// place any restrictions on SpawnableOutput.
trait Unrestricted {}
impl<T> Unrestricted for T {}

// In wasm, there's no such thing as a background thread, so all
// futures are local.  The implementation of wasm_bindgen_futures::JsFuture
// doesn't implement Send, so we want to relax that constraint when
// running in wasm.
trait_set::trait_set! {
    /// A trait representing a task which can be run in the background.
    pub trait Spawnable = 'static + Future;
    /// A trait representing a stream which can be polled in the background.
    pub trait Stream = 'static + futures::Stream;
    /// A trait representing a value which can be returned from a background
    /// task.
    ///
    /// We need to supply _some_ trait bound here, so we use Unrestricted, which
    /// doesn't apply any additional constraints on the output type.
    pub trait SpawnableOutput = Unrestricted;
    /// Bounds for async I/O streams passed to cross-platform networking code.
    /// On WASM, `Send` is not required since everything runs on the main thread.
    pub trait TransportStream = Unpin + 'static;
}

/// A future that emits timed events.
///
/// This must conform to the same API as [`async_io::Timer`].
pub struct Timer {
    /// The actual future that will resolve at some future time,
    /// producing the [`Instant`] at which it is configured to
    /// be ready.
    inner: Pin<Box<dyn Future<Output = Instant>>>,
    /// Whether or not a [`Stream`] representation of this timer
    /// is exhausted (and so should produce [`None`]).
    stream_exhausted: bool,
}

impl Timer {
    pub fn after(duration: std::time::Duration) -> Self {
        Self::new(duration, Instant::now() + duration)
    }

    pub fn at(instant: Instant) -> Self {
        let duration = instant - instant::Instant::now();
        Self::new(duration, instant)
    }

    pub fn never() -> Self {
        Self {
            inner: futures_lite::future::pending().boxed(),
            stream_exhausted: false,
        }
    }

    fn new(duration: std::time::Duration, instant: Instant) -> Self {
        let future = async move {
            // We're never scheduling a timeout for more than 50 days, so this cast to u32 is fine.
            TimeoutFuture::new(duration.as_millis() as u32).await;
            instant
        };
        Self {
            inner: Box::pin(future),
            stream_exhausted: false,
        }
    }
}

impl Future for Timer {
    type Output = Instant;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll(cx)
    }
}

impl futures::Stream for Timer {
    type Item = Instant;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.stream_exhausted {
            return Poll::Ready(None);
        }
        self.inner.poll(cx).map(|val| {
            self.stream_exhausted = true;
            Some(val)
        })
    }
}
