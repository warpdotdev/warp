use core::pin::Pin;
use std::task::{Context, Poll};

use futures::{Sink, Stream};
use futures_util::stream::FusedStream;
use pin_project::pin_project;

/// Maps the error returned by the [`Sink`] using the provided `err` function.
pub fn map_err<S, I, E>(sink: S, err: impl FnMut(S::Error) -> E) -> impl Sink<I, Error = E>
where
    S: Sink<I>,
{
    SinkMapErr::new(sink, err)
}

/// Helper struct to map the [`Err`] of an underlying [`Sink`].
/// This is a fork of the `SinkMapErr` defined within `futures-util`
/// (https://docs.rs/futures/latest/futures/sink/struct.SinkMapErr.html) except that it does _not_
/// panic if the caller tries to write to the sink after a previous attempt to write returned an
/// error. See <https://github.com/rust-lang/futures-rs/issues/2108> for more details about the
/// issue with the original `SinkMapErr` struct.
#[pin_project]
struct SinkMapErr<Si, F> {
    #[pin]
    sink: Si,
    err_function: F,
}

impl<Si, F> SinkMapErr<Si, F> {
    fn new(sink: Si, err_function: F) -> Self {
        Self { sink, err_function }
    }
}

impl<Si, F, E, Item> Sink<Item> for SinkMapErr<Si, F>
where
    Si: Sink<Item>,
    F: FnMut(Si::Error) -> E,
{
    type Error = E;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let project = self.as_mut().project();
        let err_function = project.err_function;
        project.sink.poll_ready(cx).map_err(err_function)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Item) -> Result<(), Self::Error> {
        let project = self.as_mut().project();
        let err_function = project.err_function;
        project.sink.start_send(item).map_err(err_function)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let project = self.as_mut().project();
        let err_function = project.err_function;
        project.sink.poll_flush(cx).map_err(err_function)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let project = self.as_mut().project();
        let err_function = project.err_function;
        project.sink.poll_close(cx).map_err(err_function)
    }
}

/// Implement [`Stream`] by forwarding calls to the underlying [`Sink`].
impl<S: Stream, F> Stream for SinkMapErr<S, F> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().sink.poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.sink.size_hint()
    }
}

impl<S: FusedStream, F> FusedStream for SinkMapErr<S, F> {
    fn is_terminated(&self) -> bool {
        self.sink.is_terminated()
    }
}

#[cfg(test)]
#[path = "sink_map_err_tests.rs"]
mod tests;
