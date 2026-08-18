use http::StatusCode;
use warp_errors::ErrorExt;

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
