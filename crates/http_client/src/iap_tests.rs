use http::{HeaderMap, HeaderValue};
use reqwest::StatusCode;

use super::*;

#[test]
fn only_staging_application_destinations_allow_opt_in() {
    for url in [
        "https://staging.warp.dev/graphql/v2",
        "https://oz.staging.warp.dev/api/v1/agent",
        "wss://platform.staging.warp.dev/graphql/v2",
    ] {
        assert!(
            is_opt_in_destination(&reqwest::Url::parse(url).unwrap()),
            "{url}"
        );
    }
    for url in [
        "http://staging.warp.dev/graphql/v2",
        "https://staging.warp.dev:8080/graphql/v2",
        "https://user@staging.warp.dev/graphql/v2",
        "https://staging.warp.dev.evil.example/graphql/v2",
        "https://app.warp.dev/graphql/v2",
        "https://rtc.staging.warp.dev/graphql/v2",
        "https://sessions.staging.warp.dev/api/v1/session",
        "https://admin.staging.warp.dev/",
        "https://staging.warp.dev/shared_sessions/test",
        "https://staging.warp.dev/api/v1/oauth/token",
        "https://staging.warp.dev/api/v1/agent/artifacts/uid/download",
        "https://staging.warp.dev/iap/blocked",
        "https://platform.staging.warp.dev/static/js/app.js",
    ] {
        assert!(
            !is_opt_in_destination(&reqwest::Url::parse(url).unwrap()),
            "{url}"
        );
    }
}

fn headers_with_iap() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        IAP_GENERATED_RESPONSE_HEADER,
        HeaderValue::from_static("true"),
    );
    headers
}

#[test]
fn challenge_statuses_with_iap_header_are_challenges() {
    for status in [
        StatusCode::FOUND,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
    ] {
        assert!(is_iap_challenge(status, &headers_with_iap()));
    }
}

#[test]
fn challenge_status_without_iap_header_is_not_a_challenge() {
    assert!(!is_iap_challenge(StatusCode::FORBIDDEN, &HeaderMap::new()));
    assert!(!is_iap_challenge(
        StatusCode::UNAUTHORIZED,
        &HeaderMap::new()
    ));
}

#[test]
fn non_challenge_status_with_iap_header_is_not_a_challenge() {
    assert!(!is_iap_challenge(StatusCode::OK, &headers_with_iap()));
    assert!(!is_iap_challenge(
        StatusCode::INTERNAL_SERVER_ERROR,
        &headers_with_iap()
    ));
}
