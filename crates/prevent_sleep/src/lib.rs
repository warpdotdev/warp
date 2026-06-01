#[cfg_attr(macos, path = "mac.rs")]
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(noop, path = "noop.rs")]
mod imp;

use std::pin::Pin;
use std::task::{Context, Poll};

pub use imp::Guard;
use pin_project::pin_project;

/// Returns a guard that prevents the system from going to sleep while the guard is held.
///
/// Callers should provide a description of the reason for preventing sleep. Depending on
/// platform, this may appear in logs, so write it as though it may be user-visible, e.g.:
/// "Agent Mode request in-progress".
pub fn prevent_sleep(reason: &'static str) -> Guard {
    imp::prevent_sleep(reason)
}

/// A simple wrapper around a stream that optionally prevents the system from going to sleep
/// while the stream is being polled.
#[pin_project]
pub struct Stream<S> {
    #[pin]
    inner: S,
    guard: Option<Guard>,
}

impl<S> Stream<S> {
    /// Wraps the provided stream, maintaining the provided sleep guard as long as the stream is
    /// being polled.
    pub fn wrap(inner: S, guard: Option<Guard>) -> Self {
        Self { inner, guard }
    }
}

impl<S: futures::stream::Stream> futures::stream::Stream for Stream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}
