use http::{HeaderMap, HeaderValue};
use reqwest::StatusCode;

use super::*;

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
