use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Result};
use warpui::r#async::Timer;
use warpui::{duration_with_jitter, RetryOption};

use crate::server::graphql::GraphQLError;
use crate::server::server_api::presigned_upload::HttpStatusError;

/// Common duration for a periodic poll. In our app, we generally have the following to update the same data:
/// - RTC messages
/// - Out-of-band queries based on user actions (i.e. fetch team info when user opens the settings page, user
/// starts the app)
/// However, we also periodically poll for updates in case RTC is down, the user's websocket
/// is borked, etc.
/// For team memberships, we also don't yet process messages for joining or leaving a team, so the user would see these
/// updates only after a periodic poll.
pub const PERIODIC_POLL: Duration = Duration::from_secs(60 * 10);

/// For a periodic poll, it's fine to wait for longer period of time between retries. However, we don't want this to be so
/// long that it's around the same as the overall periodic poll interval.
pub const PERIODIC_POLL_RETRY_STRATEGY: RetryOption = RetryOption::exponential(
    Duration::from_secs(2), /* interval */
    2.,                     /* exponential factor */
    3,                      /* max retry count */
)
.with_jitter(0.2 /* max_jitter_percentage */);

/// When there's an out-of-band request for a periodic poll, we want to retry quickly, because the UI is depending on the
/// request succeeding in a timely way. These are things like loading all object updates upon startup, checking the team
/// metadata when we visit the team page, etc.
pub const OUT_OF_BAND_REQUEST_RETRY_STRATEGY: RetryOption = RetryOption::exponential(
    Duration::from_millis(100), /* interval */
    5.,                         /* exponential factor */
    3,                          /* max retry count */
)
.with_jitter(0.5 /* max_jitter_percentage */);

// For listeners, retry up to 5 times, waiting between 10-40 seconds between retries.
pub const LISTENER_RETRY_STRATEGY: RetryOption = RetryOption::linear(
    Duration::from_secs(25), /* interval */
    5,                       /* max retry count */
)
.with_jitter(0.6 /* max_jitter_multiplier */);

/// Classify an HTTP-backed error as transient (worth retrying) or permanent (fail fast).
///
/// Transient: 5xx responses, 408, 429, or any error whose chain does not carry an
/// [`HttpStatusError`] (connection reset, timeout, DNS failure, etc.).
/// Permanent: other 4xx responses (bad signature, 404, 403, etc.).
pub(crate) fn is_transient_http_error(e: &anyhow::Error) -> bool {
    // Callers typically wrap an `HttpStatusError` cause with a `.context(...)` message for
    // human-friendly Display, so the typed error sits somewhere in the chain rather than as
    // the top-level error object — walk the chain.
    for cause in e.chain() {
        if let Some(http_err) = cause.downcast_ref::<HttpStatusError>() {
            return is_transient_status(http_err.status);
        }
    }
    true
}

/// Classify GraphQL/public-API status errors as transient or permanent.
///
/// Unlike [`is_transient_http_error`], errors without a typed HTTP/GraphQL transport
/// cause are treated as permanent. This is intended for GraphQL operations where
/// user-facing GraphQL errors are converted into plain `anyhow` errors at the
/// operation layer and should not be retried or placed into transient cooldowns.
pub(crate) fn is_transient_graphql_or_http_error(e: &anyhow::Error) -> bool {
    for cause in e.chain() {
        if let Some(graphql_err) = cause.downcast_ref::<GraphQLError>() {
            return match graphql_err {
                GraphQLError::RequestError(_) => true,
                GraphQLError::HttpError { status, .. } => is_transient_status(status.as_u16()),
                GraphQLError::StagingAccessBlocked
                | GraphQLError::IapChallengeBlocked
                | GraphQLError::ResponseError(_) => false,
            };
        }

        if let Some(http_err) = cause.downcast_ref::<HttpStatusError>() {
            return is_transient_status(http_err.status);
        }
    }

    false
}

fn is_transient_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500..=599)
}

/// Maximum total attempts per operation (initial attempt plus retries on transient errors).
pub(crate) const MAX_ATTEMPTS: usize = 3;

/// Base backoff between retry attempts; each subsequent attempt multiplies by [`BACKOFF_FACTOR`].
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);

/// Exponential growth factor for retry backoff.
const BACKOFF_FACTOR: f32 = 2.0;

/// Maximum jitter as a fraction of the backoff interval.
const BACKOFF_JITTER: f32 = 0.3;

/// Run `attempt_fn` with bounded exponential-backoff retries on transient failures.
///
/// `operation` is included in retry logs so concurrent callers can be distinguished.
///
/// `attempt_fn` is called repeatedly with a fresh `Future` per attempt, so callers that need
/// per-attempt state (e.g. cloning a request body) own that inside their closure.
///
/// Transient errors are retried up to [`MAX_ATTEMPTS`] total. Permanent errors return
/// immediately. A warning is logged between attempts so retries are visible in logs.
pub(crate) async fn with_bounded_retry<T, F, Fut>(operation: &str, mut attempt_fn: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut delay = INITIAL_BACKOFF;
    for attempt in 1..=MAX_ATTEMPTS {
        match attempt_fn().await {
            Ok(value) => return Ok(value),
            Err(e) if attempt >= MAX_ATTEMPTS || !is_transient_http_error(&e) => return Err(e),
            Err(e) => {
                log::warn!("{operation}: attempt {attempt}/{MAX_ATTEMPTS} failed, retrying: {e:#}");
                Timer::after(duration_with_jitter(delay, BACKOFF_JITTER)).await;
                delay = delay.mul_f32(BACKOFF_FACTOR);
            }
        }
    }
    // Unreachable when MAX_ATTEMPTS >= 1.
    Err(anyhow!(
        "retry loop exhausted without attempting operation (MAX_ATTEMPTS={MAX_ATTEMPTS})"
    ))
}
