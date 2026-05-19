use warpui::{async_assert, integration::AssertionCallback};

use super::step::{drain_captured_urls, read_captured_urls};

/// Asserts that the test has observed exactly one URL being opened, and
/// that it equals `expected`. Drains the capture buffer on success so
/// subsequent assertions in the same test see a clean slate.
pub fn assert_url_opened(expected: impl Into<String>) -> AssertionCallback {
    let expected = expected.into();
    Box::new(move |_app, _window_id| {
        let urls = read_captured_urls();
        if urls.iter().any(|u| u == &expected) {
            // Drain so this assertion is idempotent and the buffer is
            // cleared for any subsequent assertion in the test.
            let _ = drain_captured_urls();
            return warpui::integration::AssertionOutcome::Success;
        }
        async_assert!(
            false,
            "expected URL {expected:?} to be opened, captured: {urls:?}"
        )
    })
}

/// Asserts that no URL has been opened since the most recent
/// `install_open_url_capture()` (or last drain).
pub fn assert_no_url_opened() -> AssertionCallback {
    Box::new(move |_app, _window_id| {
        let urls = read_captured_urls();
        async_assert!(
            urls.is_empty(),
            "expected no URLs to be opened, captured: {urls:?}"
        )
    })
}
