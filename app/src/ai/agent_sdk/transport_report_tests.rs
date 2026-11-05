use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use instant::Instant;

use super::*;
use crate::server::server_api::harness_support::MockHarnessSupportClient;

fn background() -> Arc<Background> {
    Arc::new(Background::new(1, |_| "transport-report-test".to_string()))
}

/// Counts the waits between attempts instead of sleeping.
async fn no_wait(_: Duration) {}

/// A GraphQL request answered with a non-OK HTTP status.
fn http_error(status: u16) -> anyhow::Error {
    anyhow::Error::new(GraphQLError::HttpError {
        status: StatusCode::from_u16(status).unwrap(),
        body: String::new(),
    })
}

/// A server-side failure, which the GraphQL layer answers as errors without data.
fn graphql_errors() -> anyhow::Error {
    anyhow::anyhow!("missing response data for ReportHarnessTransport: internal error")
}

fn rejected() -> anyhow::Error {
    TransportReportRejected("fixed transport".to_string()).into()
}

/// A 401 or 403 as the GraphQL layer re-raises it when the client may not refresh its
/// credentials: a plain error with no `GraphQLError` in the chain.
fn credentials_rejected() -> anyhow::Error {
    anyhow::anyhow!(AUTH_REJECTED_MESSAGE)
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
async fn a_server_side_failure_is_retried() {
    let (client, calls) = client_answering(vec![|| Err(graphql_errors()), || Ok(())]);

    report_with_retry(&client, HarnessTransport::Pty, Some("no_sidecar"), no_wait)
        .await
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_rejected_report_is_not_retried() {
    let (client, calls) = client_answering(vec![|| Err(rejected())]);

    let error = report_with_retry(&client, HarnessTransport::Pty, None, no_wait)
        .await
        .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!mutation_unsupported(&error));
}

#[tokio::test]
async fn a_client_error_status_is_not_retried() {
    let (client, calls) = client_answering(vec![|| Err(http_error(400))]);

    let error = report_with_retry(&client, HarnessTransport::Pty, None, no_wait)
        .await
        .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!mutation_unsupported(&error));
}

#[tokio::test]
async fn rejected_credentials_are_not_retried() {
    let (client, calls) = client_answering(vec![|| Err(credentials_rejected())]);

    let error = report_with_retry(&client, HarnessTransport::Pty, None, no_wait)
        .await
        .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!mutation_unsupported(&error));
}

#[tokio::test]
async fn a_server_without_the_mutation_is_not_retried() {
    let (client, calls) = client_answering(vec![|| Err(http_error(422))]);

    let error = report_with_retry(&client, HarnessTransport::Acp, None, no_wait)
        .await
        .unwrap_err();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(mutation_unsupported(&error));
}

#[test]
fn an_unsupported_mutation_is_told_apart_from_other_failures() {
    assert!(mutation_unsupported(&http_error(422)));
    for status in [400, 401, 404, 500, 503] {
        assert!(!mutation_unsupported(&http_error(status)), "{status}");
    }
    assert!(!mutation_unsupported(&graphql_errors()));
    assert!(!mutation_unsupported(&rejected()));
    assert!(!mutation_unsupported(&anyhow::anyhow!("connection reset")));
}

#[test]
fn transient_failures_are_told_apart_from_final_ones() {
    for status in [408, 429, 500, 502, 503] {
        assert!(is_transient_report_error(&http_error(status)), "{status}");
    }
    for status in [400, 401, 403, 404, 422] {
        assert!(!is_transient_report_error(&http_error(status)), "{status}");
    }
    assert!(is_transient_report_error(&graphql_errors()));
    assert!(is_transient_report_error(&anyhow::anyhow!(
        "harness transport report timed out"
    )));
    assert!(!is_transient_report_error(&rejected()));
    assert!(!is_transient_report_error(
        &rejected().context("while reporting")
    ));
    assert!(!is_transient_report_error(&credentials_rejected()));
    assert!(!is_transient_report_error(
        &credentials_rejected().context("while reporting")
    ));
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
