use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use instant::Instant;
use warp_server_client::HttpStatusError;

use super::*;
use crate::server::server_api::harness_support::MockHarnessSupportClient;

fn background() -> Arc<Background> {
    Arc::new(Background::new(1, |_| "transport-report-test".to_string()))
}

/// Counts the waits between attempts instead of sleeping.
async fn no_wait(_: Duration) {}

fn http_error(status: u16) -> anyhow::Error {
    anyhow::Error::new(HttpStatusError::new(status, String::new()))
        .context(format!("API request failed with status {status}"))
}

/// A client whose `report_transport` answers with `results` in turn and counts its calls.
fn client_answering(
    results: Vec<fn() -> anyhow::Result<()>>,
) -> (MockHarnessSupportClient, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_mock = calls.clone();
    let mut client = MockHarnessSupportClient::new();
    client
        .expect_report_transport()
        .times(results.len())
        .returning(move |_, _| {
            let call = calls_for_mock.fetch_add(1, Ordering::SeqCst);
            results[call]()
        });
    (client, calls)
}

#[tokio::test]
async fn a_transient_failure_is_retried_with_the_same_report() {
    let mut client = MockHarnessSupportClient::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_mock = calls.clone();
    client
        .expect_report_transport()
        .withf(|transport, reason| {
            *transport == HarnessTransport::Pty && *reason == Some("no_sidecar")
        })
        .times(2)
        .returning(move |_, _| {
            if calls_for_mock.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(anyhow::anyhow!("connection reset"))
            } else {
                Ok(())
            }
        });

    report_with_retry(&client, HarnessTransport::Pty, Some("no_sidecar"), no_wait)
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_server_error_is_retried() {
    let (client, calls) = client_answering(vec![|| Err(http_error(503)), || Ok(())]);

    report_with_retry(&client, HarnessTransport::Acp, None, no_wait)
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn transient_failures_stop_at_the_attempt_bound() {
    let mut client = MockHarnessSupportClient::new();
    client
        .expect_report_transport()
        .times(REPORT_MAX_ATTEMPTS)
        .returning(|_, _| Err(http_error(502)));
    let waits = Arc::new(AtomicUsize::new(0));
    let waits_for_sleep = waits.clone();

    let result = report_with_retry(&client, HarnessTransport::Pty, None, move |_| {
        waits_for_sleep.fetch_add(1, Ordering::SeqCst);
        async {}
    })
    .await;

    assert!(result.is_err());
    assert_eq!(waits.load(Ordering::SeqCst), REPORT_MAX_ATTEMPTS - 1);
}

#[tokio::test]
async fn a_rejected_report_is_not_retried() {
    let (client, calls) = client_answering(vec![|| Err(http_error(400))]);

    let error = report_with_retry(&client, HarnessTransport::Pty, Some("bogus"), no_wait)
        .await
        .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!endpoint_missing(&error));
}

#[tokio::test]
async fn a_server_without_the_endpoint_is_not_retried() {
    let (client, calls) = client_answering(vec![|| Err(http_error(404))]);

    let error = report_with_retry(&client, HarnessTransport::Acp, None, no_wait)
        .await
        .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(endpoint_missing(&error));
}

#[test]
fn a_missing_endpoint_is_told_apart_from_other_failures() {
    for status in [404, 405, 501] {
        assert!(endpoint_missing(&http_error(status)), "{status}");
    }
    for status in [400, 401, 500, 503] {
        assert!(!endpoint_missing(&http_error(status)), "{status}");
    }
    assert!(!endpoint_missing(&anyhow::anyhow!("connection reset")));
}

#[test]
fn the_report_does_not_wait_for_the_server() {
    let mut client = MockHarnessSupportClient::new();
    client.expect_report_transport().times(1).returning(|_, _| {
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    });

    let background = background();
    let start = Instant::now();
    report_transport_in_background(&background, Arc::new(client), HarnessTransport::Acp, None);

    assert!(start.elapsed() < Duration::from_millis(50));
    std::thread::sleep(Duration::from_millis(300));
}
