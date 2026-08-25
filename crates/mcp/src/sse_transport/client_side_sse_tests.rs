//! Tests for the auto-reconnect stream's retry bounding and fatal-error
//! short-circuit.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt as _;

use super::client_side_sse::{
    BoxedSseResponse, FixedInterval, SseAutoReconnectStream, SseStreamReconnect,
};
use super::sse_client::SseTransportError;

/// A connector whose reconnect attempts always fail with the given error.
struct FailingConnector {
    error: fn() -> SseTransportError<reqwest::Error>,
    attempts: Arc<AtomicUsize>,
}

impl SseStreamReconnect for FailingConnector {
    type Error = SseTransportError<reqwest::Error>;
    type Future = futures::future::Ready<Result<BoxedSseResponse, Self::Error>>;

    fn retry_connection(&mut self, _last_event_id: Option<&str>) -> Self::Future {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        futures::future::ready(Err((self.error)()))
    }

    fn is_fatal_error(&self, error: &Self::Error) -> bool {
        matches!(error, SseTransportError::HttpStatus { .. })
    }
}

/// An initial stream that immediately fails, driving the reconnect path.
fn failing_stream() -> BoxedSseResponse {
    futures::stream::once(async { Err(sse_stream::Error::InvalidLine) }).boxed()
}

#[tokio::test]
async fn fatal_reconnect_errors_terminate_immediately() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut stream = Box::pin(SseAutoReconnectStream::new(
        failing_stream(),
        FailingConnector {
            error: || SseTransportError::HttpStatus {
                status: http::StatusCode::UNAUTHORIZED,
                body: String::new(),
                www_authenticate: None,
            },
            attempts: attempts.clone(),
        },
        Arc::new(FixedInterval {
            max_times: Some(100),
            duration: Duration::from_millis(1),
        }),
    ));

    let item = stream.next().await.expect("stream yields the fatal error");
    assert!(matches!(
        item,
        Err(SseTransportError::HttpStatus { status, .. })
            if status == http::StatusCode::UNAUTHORIZED
    ));
    // One attempt, not one hundred: auth rejections must not hammer the server.
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(stream.next().await.is_none(), "stream is terminated");
}

#[tokio::test]
async fn non_fatal_reconnect_errors_stop_after_the_retry_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut stream = Box::pin(SseAutoReconnectStream::new(
        failing_stream(),
        FailingConnector {
            error: || SseTransportError::UnexpectedEndOfStream,
            attempts: attempts.clone(),
        },
        Arc::new(FixedInterval {
            max_times: Some(3),
            duration: Duration::from_millis(1),
        }),
    ));

    let item = stream.next().await.expect("stream yields the final error");
    assert!(matches!(
        item,
        Err(SseTransportError::UnexpectedEndOfStream)
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert!(stream.next().await.is_none(), "stream is terminated");
}
