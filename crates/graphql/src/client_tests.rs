use http::{HeaderMap, HeaderValue, StatusCode};

use super::{ANONYMOUS_SESSION_REJECTED_HEADER, is_anonymous_session_rejection};

fn headers_with_rejection(value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ANONYMOUS_SESSION_REJECTED_HEADER,
        HeaderValue::from_static(value),
    );
    headers
}

#[test]
fn unauthorized_with_rejection_header_is_anonymous_session_rejection() {
    assert!(is_anonymous_session_rejection(
        StatusCode::UNAUTHORIZED,
        &headers_with_rejection("true"),
    ));
}

#[test]
fn unauthorized_without_rejection_header_is_not_anonymous_session_rejection() {
    assert!(!is_anonymous_session_rejection(
        StatusCode::UNAUTHORIZED,
        &HeaderMap::new(),
    ));
}

#[test]
fn rejection_header_requires_unauthorized_status() {
    assert!(!is_anonymous_session_rejection(
        StatusCode::FORBIDDEN,
        &headers_with_rejection("true"),
    ));
}

#[test]
fn rejection_header_requires_true_value() {
    assert!(!is_anonymous_session_rejection(
        StatusCode::UNAUTHORIZED,
        &headers_with_rejection("false"),
    ));
}
