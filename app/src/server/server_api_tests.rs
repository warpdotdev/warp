use futures::executor::block_on;
use mockito::Server;

use super::*;
use crate::server::retry_strategies::is_transient_http_error;

/// Sends a GET request to a mock endpoint returning `status`/`headers`/`body`, then feeds the
/// resulting response through [`ServerApi::error_from_response`].
fn error_from_mock_response(status: usize, headers: &[(&str, &str)], body: &str) -> anyhow::Error {
    let mut server = Server::new();
    let mut mock = server
        .mock("GET", "/error")
        .with_status(status)
        .with_body(body);
    for (name, value) in headers {
        mock = mock.with_header(*name, value);
    }
    mock.create();

    let url = format!("{}/error", server.url());
    block_on(async move {
        let response = http_client::Client::new_for_test()
            .get(url)
            .send()
            .await
            .unwrap();
        ServerApi::error_from_response(response).await
    })
}

/// The status carried by the [`HttpStatusError`] in `err`'s chain, if any.
fn status_in_chain(err: &anyhow::Error) -> Option<u16> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<HttpStatusError>())
        .map(|status_error| status_error.status)
}

#[test]
fn permanent_4xx_client_error_carries_status_and_fails_fast() {
    let err = error_from_mock_response(
        403,
        &[],
        r#"{"error": "checkpoint generation is incomplete"}"#,
    );

    assert_eq!(status_in_chain(&err), Some(403));
    assert!(!is_transient_http_error(&err));
    assert_eq!(err.to_string(), "checkpoint generation is incomplete");
    assert_eq!(
        err.downcast_ref::<ClientError>().unwrap().error,
        "checkpoint generation is incomplete"
    );
}

#[test]
fn permanent_4xx_without_parseable_body_still_carries_status() {
    let err = error_from_mock_response(404, &[], "not found");

    assert_eq!(status_in_chain(&err), Some(404));
    assert!(!is_transient_http_error(&err));
    assert_eq!(
        err.to_string(),
        "API request failed with status 404 Not Found"
    );
}

#[test]
fn transient_5xx_still_retries() {
    let err = error_from_mock_response(503, &[], "unavailable");

    assert_eq!(status_in_chain(&err), Some(503));
    assert!(is_transient_http_error(&err));
}

#[test]
fn at_capacity_header_wraps_capacity_error_and_still_carries_status() {
    let err = error_from_mock_response(
        403,
        &[(WARP_ERROR_CODE_HEADER, WARP_ERROR_CODE_AT_CAPACITY)],
        r#"{"error": "at capacity", "running_agents": 5}"#,
    );

    assert_eq!(status_in_chain(&err), Some(403));
    assert!(!is_transient_http_error(&err));
    assert_eq!(
        err.downcast_ref::<CloudAgentCapacityError>()
            .unwrap()
            .running_agents,
        5
    );
}

#[test]
fn out_of_credits_429_wraps_quota_limit_and_stays_transient() {
    let err = error_from_mock_response(
        429,
        &[(WARP_ERROR_CODE_HEADER, WARP_ERROR_CODE_OUT_OF_CREDITS)],
        r#"{"userDisplayMessage": "You're out of credits"}"#,
    );

    // 429 always retries regardless of error code, matching every other public-API caller.
    assert_eq!(status_in_chain(&err), Some(429));
    assert!(is_transient_http_error(&err));
    assert!(matches!(
        err.downcast_ref::<AIApiError>().unwrap(),
        AIApiError::QuotaLimit {
            user_display_message: Some(message)
        } if message == "You're out of credits"
    ));
}
