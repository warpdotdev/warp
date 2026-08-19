use anyhow::Context as _;
use http::StatusCode;
use warp_errors::{AnyhowErrorExt as _, ErrorExt};

use super::GraphQLError;

#[test]
fn transient_http_statuses_are_non_actionable() {
    for status in [
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let error = GraphQLError::HttpError {
            status,
            body: String::new(),
        };
        assert!(!error.is_actionable(), "{status} should be non-actionable");
    }
}

#[test]
fn other_http_statuses_stay_actionable() {
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
    ] {
        let error = GraphQLError::HttpError {
            status,
            body: String::new(),
        };
        assert!(error.is_actionable(), "{status} should stay actionable");
    }
}

#[test]
fn infrastructure_challenges_are_non_actionable() {
    assert!(!GraphQLError::StagingAccessBlocked.is_actionable());
    assert!(!GraphQLError::IapChallengeBlocked.is_actionable());
}

/// Pins the shape `report_error!` actually sees at the `HarnessAvailabilityModel::refresh`
/// sink: a `GraphQLError` wrapped in an `anyhow::Error` via `.context(..)`. Classification on
/// `GraphQLError` alone doesn't guarantee this — it also depends on `register_error!` having
/// registered the type and `AnyhowErrorExt::is_actionable` walking the chain to find it.
#[test]
fn anyhow_context_wrapped_transient_http_error_is_non_actionable() {
    let error: anyhow::Error = Err::<(), _>(GraphQLError::HttpError {
        status: StatusCode::REQUEST_TIMEOUT,
        body: String::new(),
    })
    .context("Failed to fetch available harnesses")
    .unwrap_err();

    assert!(!error.is_actionable());
}
