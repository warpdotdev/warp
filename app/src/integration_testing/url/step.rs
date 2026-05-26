use std::sync::{Mutex, OnceLock};

use warpui::integration::TestStep;

/// Process-wide list of URLs that the test has observed being opened
/// since the last `install_open_url_capture()` step. Cleared by that
/// step so each test starts with a clean buffer.
fn captured_urls() -> &'static Mutex<Vec<String>> {
    static CAPTURED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    CAPTURED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Drain the captured URLs into a fresh Vec. Used by assertions.
pub(crate) fn drain_captured_urls() -> Vec<String> {
    let mut guard = captured_urls().lock().expect("captured_urls poisoned");
    std::mem::take(&mut *guard)
}

/// Read the captured URLs without clearing them.
pub(crate) fn read_captured_urls() -> Vec<String> {
    let guard = captured_urls().lock().expect("captured_urls poisoned");
    guard.clone()
}

/// Install a `before_open_url` callback that records every URL the app
/// would open into a process-wide buffer. The buffer is cleared first
/// so previously captured URLs from other tests don't bleed in.
///
/// The callback returns the URL unchanged; the test platform delegate's
/// `open_url` is a no-op, so no real browser is launched.
pub fn install_open_url_capture() -> TestStep {
    TestStep::new("Install open_url capture").with_action(|app, _window_id, _step_data_map| {
        // Reset before each test so leftover URLs from a prior test
        // don't poison our assertions.
        captured_urls()
            .lock()
            .expect("captured_urls poisoned")
            .clear();
        app.update(|ctx| {
            ctx.set_before_open_url(|url, _ctx| {
                let mut guard = captured_urls().lock().expect("captured_urls poisoned");
                guard.push(url.to_owned());
                url.to_owned()
            });
        });
    })
}
