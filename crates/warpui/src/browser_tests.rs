use super::safe_browser_open_url;

#[test]
fn safe_browser_open_url_accepts_browser_safe_urls() {
    assert_eq!(
        safe_browser_open_url("https://example.com/path?q=1").as_deref(),
        Some("https://example.com/path?q=1")
    );
    assert_eq!(
        safe_browser_open_url("http://example.com/path?q=1").as_deref(),
        Some("http://example.com/path?q=1")
    );
    assert_eq!(
        safe_browser_open_url("mailto:support@warp.dev").as_deref(),
        Some("mailto:support@warp.dev")
    );
}

#[test]
fn safe_browser_open_url_accepts_warp_channel_urls() {
    for scheme in [
        "warp",
        "warppreview",
        "warpdev",
        "warplocal",
        "warposs",
        "warpintegration",
    ] {
        let url = format!("{scheme}://action/focus_cloud_mode");
        assert_eq!(safe_browser_open_url(&url).as_deref(), Some(url.as_str()));
    }
}

#[test]
fn safe_browser_open_url_rejects_script_capable_and_risky_urls() {
    for url in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
        "blob:https://example.com/550e8400-e29b-41d4-a716-446655440000",
        "about:blank",
        "file:///tmp/payload.html",
        "ftp://example.com/file",
    ] {
        assert_eq!(safe_browser_open_url(url), None, "{url}");
    }
}

#[test]
fn safe_browser_open_url_rejects_relative_and_malformed_urls() {
    for url in [
        "/relative/path",
        "example.com/path",
        "https://",
        "http://[::1",
        "",
    ] {
        assert_eq!(safe_browser_open_url(url), None, "{url}");
    }
}

#[test]
fn safe_browser_open_url_keeps_dangerous_content_as_url_data() {
    let url =
        "https://example.com/%22%3E%3Cscript%3Ealert(1)%3C/script%3E?next=javascript:alert(1)";
    let safe_url = safe_browser_open_url(url).expect("https URL should be allowed");
    let parsed_url = url::Url::parse(&safe_url).expect("safe URL should remain parseable");

    assert_eq!(parsed_url.scheme(), "https");
    assert_eq!(parsed_url.host_str(), Some("example.com"));
    assert_eq!(parsed_url.query(), Some("next=javascript:alert(1)"));
}
