use std::cell::Cell;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use futures::executor::block_on;
use http::StatusCode;

use super::*;
use crate::server::graphql::GraphQLError;
use crate::server::server_api::presigned_upload::HttpStatusError;

fn http_err(status: u16) -> anyhow::Error {
    HttpStatusError {
        status,
        body: format!("status {status} body"),
    }
    .into()
}

fn graphql_http_err(status: StatusCode) -> anyhow::Error {
    GraphQLError::HttpError {
        status,
        body: format!("status {status} body"),
    }
    .into()
}

#[test]
fn transient_5xx_status_codes_are_retryable() {
    assert!(is_transient_http_error(&http_err(503)));
    assert!(is_transient_http_error(&http_err(500)));
}

#[test]
fn transient_408_and_429_are_retryable() {
    assert!(is_transient_http_error(&http_err(408)));
    assert!(is_transient_http_error(&http_err(429)));
}

#[test]
fn permanent_4xx_status_codes_are_not_retryable() {
    assert!(!is_transient_http_error(&http_err(403)));
    assert!(!is_transient_http_error(&http_err(404)));
    assert!(!is_transient_http_error(&http_err(400)));
}

#[test]
fn errors_without_http_status_are_treated_as_transient() {
    // Network-layer errors (connection reset, timeout, DNS failure) aren't `HttpStatusError`;
    // treat them as transient so the retry loop gives them a chance.
    let err = anyhow!("connection reset by peer");
    assert!(is_transient_http_error(&err));

    let err = anyhow!("Failed to send request: timed out");
    assert!(is_transient_http_error(&err));
}

#[test]
fn graphql_status_classifier_retries_transient_statuses() {
    assert!(is_transient_graphql_or_http_error(&graphql_http_err(
        StatusCode::SERVICE_UNAVAILABLE
    )));
    assert!(is_transient_graphql_or_http_error(&graphql_http_err(
        StatusCode::REQUEST_TIMEOUT
    )));
    assert!(is_transient_graphql_or_http_error(&graphql_http_err(
        StatusCode::TOO_MANY_REQUESTS
    )));
}

#[test]
fn graphql_status_classifier_fails_fast_on_permanent_statuses() {
    assert!(!is_transient_graphql_or_http_error(&graphql_http_err(
        StatusCode::UNAUTHORIZED
    )));
    assert!(!is_transient_graphql_or_http_error(&graphql_http_err(
        StatusCode::FORBIDDEN
    )));
    assert!(!is_transient_graphql_or_http_error(&graphql_http_err(
        StatusCode::NOT_FOUND
    )));
}

#[test]
fn graphql_status_classifier_fails_fast_without_typed_transport_error() {
    // GraphQL user-facing errors are converted to plain anyhow errors at the operation
    // layer, so GraphQL retry call sites should not treat unknown errors as transient.
    let err = anyhow!("user-facing GraphQL error");
    assert!(!is_transient_graphql_or_http_error(&err));
}

#[test]
fn retry_loop_succeeds_on_first_attempt() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_clone = attempts.clone();
    let result: Result<()> = block_on(with_bounded_retry("test retry", || {
        attempts_clone.set(attempts_clone.get() + 1);
        async { Ok(()) }
    }));
    result.unwrap();
    assert_eq!(attempts.get(), 1);
}

#[test]
fn retry_loop_retries_transient_and_eventually_succeeds() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_clone = attempts.clone();
    let result: Result<u32> = block_on(with_bounded_retry("test retry", || {
        let n = attempts_clone.get() + 1;
        attempts_clone.set(n);
        async move { if n < 2 { Err(http_err(503)) } else { Ok(n) } }
    }));
    assert_eq!(result.unwrap(), 2);
    assert_eq!(attempts.get(), 2);
}

#[test]
fn retry_loop_stops_at_max_attempts_on_persistent_transient() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_clone = attempts.clone();
    let result: Result<()> = block_on(with_bounded_retry("test retry", || {
        attempts_clone.set(attempts_clone.get() + 1);
        async { Err(http_err(503)) }
    }));
    assert!(result.is_err());
    assert_eq!(attempts.get(), MAX_ATTEMPTS);
}

#[test]
fn retry_loop_fails_fast_on_permanent_error() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_clone = attempts.clone();
    let result: Result<()> = block_on(with_bounded_retry("test retry", || {
        attempts_clone.set(attempts_clone.get() + 1);
        async { Err(http_err(403)) }
    }));
    assert!(result.is_err());
    assert_eq!(attempts.get(), 1, "permanent errors should not retry");
}
