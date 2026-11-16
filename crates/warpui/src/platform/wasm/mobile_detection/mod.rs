//! Mobile device detection utilities.

use std::sync::OnceLock;

mod user_agent;

pub use user_agent::is_mobile_user_agent;

/// Cached result of mobile device detection.
static IS_MOBILE: OnceLock<bool> = OnceLock::new();

/// Returns `true` if the current device appears to be a mobile device that would
/// benefit from soft keyboard support.
///
/// This function caches its result since the device type won't change during a session.
pub fn is_mobile_device() -> bool {
    *IS_MOBILE.get_or_init(detect_mobile_device)
}

/// Performs the actual mobile device detection by checking the user agent and touch capabilities.
fn detect_mobile_device() -> bool {
    let navigator = gloo::utils::window().navigator();
    let has_touch = navigator.max_touch_points() > 0;

    if !has_touch {
        return false;
    }

    let ua = navigator.user_agent().ok().unwrap_or_default();
    // Standard mobile OS (iPhone, Android, etc.) or iPad (reports as "Macintosh" with touch)
    is_mobile_user_agent(&ua) || ua.to_lowercase().contains("macintosh")
}
