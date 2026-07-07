use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use pin_project::{pin_project, pinned_drop};

/// Trait allowing you to attach a function to a [`Future`] that will be called if the future is
/// cancelled.  
pub trait OnCancelFutureExt
where
    Self: Future + Sized,
{
    /// Wraps the future with an [`OnCancelFutureExt`] that will execute the given function
    /// when the future is cancelled.
    fn on_cancel<D: FnMut()>(self, on_drop: D) -> OnCancelFuture<Self, D>;
}
impl<F: Future> OnCancelFutureExt for F {
    fn on_cancel<D: FnMut()>(self, on_cancel: D) -> OnCancelFuture<Self, D> {
        OnCancelFuture {
            inner: self,
            on_cancel,
            is_ready: false,
        }
    }
}

/// Wrapper around a [`Future`] that calls an `on_cancel` callback if the future is cancelled
/// before it resolved to ready. See [`OnCancelFuture::on_cancel`] for more details. A future is
/// considered cancelled if it is dropped before resolving to [`Poll::Ready`], see <https://google.github.io/comprehensive-rust/async/pitfalls/cancellation.html#cancellation>.
#[pin_project(PinnedDrop)]
pub struct OnCancelFuture<F: Future, D: FnMut()> {
    #[pin]
    inner: F,
    /// Function that is called when the future is cancelled.
    on_cancel: D,
    /// Whether the inner future is has returned [`Poll::Ready`] (indicating the future is
    /// complete).
    is_ready: bool,
}

impl<F: Future, D: FnMut()> Future for OnCancelFuture<F, D> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.project();
        let output = this.inner.poll(cx);
        *this.is_ready = output.is_ready();
        output
    }
}

#[pinned_drop]
impl<F: Future, D: FnMut()> PinnedDrop for OnCancelFuture<F, D> {
    fn drop(self: Pin<&mut Self>) {
        // If the future was dropped before it was resolved to ready, the future was cancelled.
        let this = self.project();
        if !*this.is_ready {
            (this.on_cancel)();
        }
    }
}

#[cfg(test)]
#[path = "on_cancel_tests.rs"]
mod tests;
