use super::*;

fn assert_url_error(url: &str, key: &str) {
    i18n::set_locale("en");
    assert_eq!(validate_url(url), Err(i18n::t(key)));
}

#[test]
fn validate_url_accepts_https_with_host() {
    assert!(validate_url("https://api.example.com/v1").is_ok());
    assert!(validate_url("https://example.com").is_ok());
    assert!(validate_url("https://8.8.8.8/v1").is_ok());
}

#[test]
fn validate_url_rejects_http() {
    assert_url_error(
        "http://api.example.com/v1",
        "settings.custom_inference.error.url_https_required",
    );
    assert_url_error(
        "http://example.com",
        "settings.custom_inference.error.url_https_required",
    );
}

#[test]
fn validate_url_rejects_ftp_and_other_schemes() {
    assert_url_error(
        "ftp://files.example.com",
        "settings.custom_inference.error.url_https_required",
    );
    assert_url_error(
        "file:///etc/passwd",
        "settings.custom_inference.error.url_https_required",
    );
    assert_url_error(
        "ws://socket.example.com",
        "settings.custom_inference.error.url_https_required",
    );
}

#[test]
fn validate_url_rejects_malformed_strings() {
    assert_url_error("not a url", "settings.custom_inference.error.invalid_url");
    assert_url_error("https://", "settings.custom_inference.error.invalid_url");
}

#[test]
fn validate_url_rejects_empty_host() {
    assert_url_error(
        "https://?query=1",
        "settings.custom_inference.error.invalid_url",
    );
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
    for url in [
        "https://localhost:8080",
        "https://127.0.0.1/v1",
        "https://0.0.0.0/v1",
        "https://10.0.0.1/v1",
        "https://172.16.0.1/v1",
        "https://192.168.0.1/v1",
        "https://169.254.0.1/v1",
        "https://[::1]/v1",
        "https://[::]/v1",
        "https://[fc00::1]/v1",
        "https://[fe80::1]/v1",
        "https://[::ffff:192.168.0.1]/v1",
    ] {
        assert_url_error(url, "settings.custom_inference.error.url_restricted_host");
    }
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
