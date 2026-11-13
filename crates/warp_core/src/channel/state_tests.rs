use url::Url;

use super::{derive_http_origin_from_ws_url, host_is_local};

#[test]
fn wss_becomes_https_and_strips_path() {
    let got = derive_http_origin_from_ws_url("wss://rtc.app.warp.dev/graphql/v2");
    assert_eq!(got.as_deref(), Some("https://rtc.app.warp.dev"));
}

#[test]
fn ws_becomes_http_and_preserves_port() {
    let got = derive_http_origin_from_ws_url("ws://localhost:8080/graphql/v2");
    assert_eq!(got.as_deref(), Some("http://localhost:8080"));
}

#[test]
fn unparseable_input_returns_none() {
    assert!(derive_http_origin_from_ws_url("not a url").is_none());
    assert!(derive_http_origin_from_ws_url("https://app.warp.dev").is_none());
}

#[test]
fn ipv6_loopback_host_str_is_bracketed() {
    // Pins the form `Url::host_str` yields for IPv6 loopback; `LOCAL_HOSTS` matches it.
    assert_eq!(
        Url::parse("http://[::1]:8080").unwrap().host_str(),
        Some("[::1]")
    );
}

#[test]
fn local_server_urls_are_local() {
    // Every allowlisted host, including port/scheme variations, disables IAP.
    assert!(host_is_local("http://localhost:8080"));
    assert!(host_is_local("http://127.0.0.1:8080"));
    assert!(host_is_local("http://[::1]:8080"));
    assert!(host_is_local("http://host.docker.internal:8080"));
    assert!(host_is_local("http://localhost"));
    assert!(host_is_local("https://localhost:443"));
}

#[test]
fn non_local_server_urls_keep_iap() {
    // Security-critical: staging and production keep IAP enforced.
    assert!(!host_is_local("https://staging.warp.dev"));
    assert!(!host_is_local("https://app.warp.dev"));
}

#[test]
fn unrecognized_and_substring_hosts_keep_iap() {
    // Unparseable input and substring-only hosts are not local (exact match).
    assert!(!host_is_local("not a url"));
    assert!(!host_is_local("https://localhost.evil.example.com"));
    assert!(!host_is_local("https://mylocalhost.dev"));
}
