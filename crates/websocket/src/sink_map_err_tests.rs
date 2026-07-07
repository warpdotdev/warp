use futures_test_sink::SinkMock;
use futures_util::SinkExt;
use thiserror::Error;

use super::*;

#[derive(Error, PartialEq, Debug, Copy, Clone)]
#[error("Unable to send to sink")]
struct UnmappedError(u8);

#[derive(Error, PartialEq, Debug, Copy, Clone)]
#[error("Unable to send to sink")]
struct MappedError(u8);

#[tokio::test]
async fn test_map_err() {
    let poll_results = vec![
        Poll::Ready(Ok(())),
        Poll::Pending,
        Poll::Ready(Err(UnmappedError(0))),
        Poll::Ready(Err(UnmappedError(1))),
    ]
    .into_iter();

    let mut sink = SinkMock::with_flush_feedback(poll_results.clone());

    // The unmapped sink should return an `UnmappedError`.
    assert_eq!(Ok(()), sink.send(()).await);
    assert_eq!(Err(UnmappedError(0)), sink.send(()).await);
    assert_eq!(Err(UnmappedError(1)), sink.send(()).await);

    let mut mapped_sink = map_err(SinkMock::with_flush_feedback(poll_results), |err| {
        MappedError(err.0)
    });

    // After mapping the item type should be unchanged, but the error should be a `MappedError`.
    assert_eq!(Ok(()), mapped_sink.send(()).await);
    assert_eq!(Err(MappedError(0)), mapped_sink.send(()).await);
    assert_eq!(Err(MappedError(1)), mapped_sink.send(()).await);
}
