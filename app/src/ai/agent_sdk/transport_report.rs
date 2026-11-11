//! Tells the server which transport a run's third-party harness is actually driven over.
//!
//! The server records the transport a worker dispatched at claim time, and follow-up attachment
//! delivery, Slack session links and the dispatch of later executions of the conversation read
//! that record. When the client falls back from ACP to a PTY, the record is only corrected by this
//! report, so it is retried on transient failures. It runs in the background so harness start
//! never waits on the server, and a report that matches the record is a no-op on the server, so
//! retrying is safe.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use http::StatusCode;
use warp_cli::agent::HarnessTransport;
use warp_server_client::graphql_helpers::is_auth_rejection;
use warpui::r#async::executor::Background;
use warpui::r#async::{FutureExt as _, Timer};

use crate::server::graphql::GraphQLError;
use crate::server::retry_strategies::{backoff_after_attempts, is_transient_graphql_or_http_error};
use crate::server::server_api::harness_support::{HarnessSupportClient, TransportReportRejected};

/// Upper bound on one attempt, so a server that hangs costs one attempt rather than the report.
const REPORT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// Total attempts. With [`backoff_after_attempts`] the waits between them add up to about 7.5s,
/// enough to ride out a brief outage or a server deploy.
const REPORT_MAX_ATTEMPTS: usize = 5;

/// Reports `transport`, with `fallback_reason` when the client fell back from the transport the
/// server requested, on `background`. Returns immediately.
pub(crate) fn report_transport_in_background(
    background: &Background,
    client: Arc<dyn HarnessSupportClient>,
    transport: HarnessTransport,
    fallback_reason: Option<&'static str>,
) {
    background
        .spawn(async move {
            if let Err(error) =
                report_with_retry(client.as_ref(), transport, fallback_reason, Timer::after).await
            {
                if mutation_unsupported(&error) {
                    log::info!(
                        "The server does not accept harness transport reports; not reporting: {error:#}"
                    );
                } else {
                    log::warn!("Failed to report the harness transport to the server: {error:#}");
                }
            }
        })
        .detach();
}

/// Sends the report, retrying transient failures (see [`is_transient_report_error`]) up to
/// [`REPORT_MAX_ATTEMPTS`] times and waiting with `sleep` between attempts. Any other error,
/// such as the 422 of a server without the mutation, is returned at once.
async fn report_with_retry<S, SF>(
    client: &dyn HarnessSupportClient,
    transport: HarnessTransport,
    fallback_reason: Option<&'static str>,
    mut sleep: S,
) -> anyhow::Result<()>
where
    S: FnMut(Duration) -> SF,
    SF: Future,
{
    let mut attempt = 1;
    loop {
        let error = match client
            .report_transport(transport, fallback_reason)
            .with_timeout(REPORT_ATTEMPT_TIMEOUT)
            .await
        {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => error,
            Err(_) => anyhow::anyhow!(
                "harness transport report timed out after {REPORT_ATTEMPT_TIMEOUT:?}"
            ),
        };
        if attempt >= REPORT_MAX_ATTEMPTS || !is_transient_report_error(&error) {
            return Err(error);
        }
        log::info!(
            "Harness transport report attempt {attempt}/{REPORT_MAX_ATTEMPTS} failed, retrying: {error:#}"
        );
        sleep(backoff_after_attempts(attempt)).await;
        attempt += 1;
    }
}

/// Whether sending the report again may succeed. A rejection (a user-facing error, or a report
/// this client could not build) never will, and neither will a request whose credentials the
/// server rejected, which the GraphQL layer answers with a plain error rather than the 401 or
/// 403 when the agent process may not refresh them. A GraphQL transport failure is retried when
/// it is a network error, 408, 429 or 5xx, and not otherwise, so the 422 of a server without
/// the mutation fails at once. Anything else is retried: an attempt that timed out, a failure
/// before the request was sent, or a server-side failure answered as GraphQL errors without
/// data.
fn is_transient_report_error(error: &anyhow::Error) -> bool {
    if error
        .chain()
        .any(|cause| cause.is::<TransportReportRejected>())
    {
        return false;
    }
    if error.chain().any(|cause| cause.is::<GraphQLError>()) {
        return is_transient_graphql_or_http_error(error);
    }
    !is_auth_rejection(error)
}

/// Whether `error` means the server's schema does not accept the report, as an older server
/// without the `reportHarnessTransport` mutation answers: a GraphQL validation failure (422).
fn mutation_unsupported(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<GraphQLError>(),
            Some(GraphQLError::HttpError { status, .. }) if *status == StatusCode::UNPROCESSABLE_ENTITY
        )
    })
}

#[cfg(test)]
#[path = "transport_report_tests.rs"]
mod tests;
