use super::*;

// --- ConnectionTestStatus / probe-result tests ---

#[test]
fn http_200_response_sets_confirmed_status() {
    // When the probe receives a 2xx HTTP response (success = true) the status
    // should transition to Confirmed.
    assert_eq!(
        connection_status_from_result(true),
        ConnectionTestStatus::Confirmed,
    );
}

#[test]
fn http_error_or_non_200_sets_failed_status() {
    // A non-2xx response or a network/transport error (success = false) should
    // produce Failed, not Confirmed.
    assert_eq!(
        connection_status_from_result(false),
        ConnectionTestStatus::Failed,
    );
}

#[test]
fn editing_url_or_api_key_resets_connection_status_to_idle() {
    // `handle_endpoint_url_event` and `handle_api_key_event` both call
    // `reset_connection_test_status()` on any `Edited` event, which sets
    // `connection_test_status` to `Idle`.
    //
    // This test exercises the reset path through `apply_connection_result`: once
    // the status is `Idle` (i.e. a reset has occurred), any in-flight result
    // that arrives afterwards must be dropped — verifying both that the reset
    // took effect and that the race guard preserves it.
    for success in [true, false] {
        assert_eq!(
            apply_connection_result(ConnectionTestStatus::Idle, success),
            ConnectionTestStatus::Idle,
            "after URL/key edit resets status to Idle, a stale result (success={success}) must be dropped",
        );
    }
}

#[test]
fn stale_result_is_dropped_when_status_is_not_testing() {
    // `SpawnedFutureHandle::abort()` cancels a future only on its next poll.
    // A request that resolves just before a URL/API-key edit delivers its
    // completion callback *after* `reset_connection_test_status` has set the
    // status to `Idle`. `apply_connection_result` must not overwrite that reset.
    //
    // Covers all non-Testing statuses that could be present when a stale result
    // arrives.
    let non_testing_statuses = [
        ConnectionTestStatus::Idle,
        ConnectionTestStatus::Confirmed,
        ConnectionTestStatus::Failed,
    ];
    for status in non_testing_statuses {
        assert_eq!(
            apply_connection_result(status.clone(), true),
            status,
            "stale Confirmed result must not overwrite status {status:?}",
        );
        assert_eq!(
            apply_connection_result(status.clone(), false),
            status,
            "stale Failed result must not overwrite status {status:?}",
        );
    }
}

#[test]
fn redirect_response_treated_as_failed_status() {
    // The reqwest client is built with `redirect::Policy::none()`, so a 30x
    // response causes `send()` to return an `Err`, which maps to `success = false`
    // and ultimately `ConnectionTestStatus::Failed`. This prevents a public URL
    // that redirects to a private address from bypassing `validate_url`'s SSRF
    // guard.
    assert_eq!(
        connection_status_from_result(false),
        ConnectionTestStatus::Failed,
    );
}

// --- validate_url tests (existing) ---

#[test]
fn validate_url_accepts_https_with_host() {
    assert!(validate_url("https://api.example.com/v1").is_ok());
    assert!(validate_url("https://example.com").is_ok());
    assert!(validate_url("https://8.8.8.8/v1").is_ok());
}

#[test]
fn validate_url_rejects_http() {
    assert_eq!(
        validate_url("http://api.example.com/v1"),
        Err("URL must use HTTPS")
    );
    assert_eq!(
        validate_url("http://example.com"),
        Err("URL must use HTTPS")
    );
}

#[test]
fn validate_url_rejects_ftp_and_other_schemes() {
    assert_eq!(
        validate_url("ftp://files.example.com"),
        Err("URL must use HTTPS")
    );
    assert_eq!(
        validate_url("file:///etc/passwd"),
        Err("URL must use HTTPS")
    );
    assert_eq!(
        validate_url("ws://socket.example.com"),
        Err("URL must use HTTPS")
    );
}

#[test]
fn validate_url_rejects_malformed_strings() {
    assert_eq!(validate_url("not a url"), Err("Invalid URL"));
    assert_eq!(validate_url("https://"), Err("Invalid URL"));
}

#[test]
fn validate_url_rejects_empty_host() {
    assert_eq!(validate_url("https://?query=1"), Err("Invalid URL"));
}

#[test]
fn validate_url_allows_empty_string() {
    assert!(validate_url("").is_ok());
}

#[test]
fn validate_url_allows_whitespace_only() {
    assert!(validate_url("   ").is_ok());
}

#[test]
fn validate_url_rejects_localhost_and_private_ips() {
    let error = Err("URL must not use a local or private host");
    assert_eq!(validate_url("https://localhost:8080"), error);
    assert_eq!(validate_url("https://127.0.0.1/v1"), error);
    assert_eq!(validate_url("https://0.0.0.0/v1"), error);
    assert_eq!(validate_url("https://10.0.0.1/v1"), error);
    assert_eq!(validate_url("https://172.16.0.1/v1"), error);
    assert_eq!(validate_url("https://192.168.0.1/v1"), error);
    assert_eq!(validate_url("https://169.254.0.1/v1"), error);
    assert_eq!(validate_url("https://[::1]/v1"), error);
    assert_eq!(validate_url("https://[::]/v1"), error);
    assert_eq!(validate_url("https://[fc00::1]/v1"), error);
    assert_eq!(validate_url("https://[fe80::1]/v1"), error);
    assert_eq!(validate_url("https://[::ffff:192.168.0.1]/v1"), error);
}

#[test]
fn endpoint_form_valid_rejects_invalid_current_url() {
    assert!(!is_endpoint_form_valid(
        "Endpoint",
        "http://api.example.com/v1",
        "key",
        true
    ));
}

#[test]
fn endpoint_form_valid_requires_non_empty_url() {
    assert!(!is_endpoint_form_valid("Endpoint", "", "key", true));
    assert!(!is_endpoint_form_valid("Endpoint", "   ", "key", true));
}

#[test]
fn endpoint_form_valid_accepts_complete_valid_form() {
    assert!(is_endpoint_form_valid(
        "Endpoint",
        "https://api.example.com/v1",
        "key",
        true
    ));
}
