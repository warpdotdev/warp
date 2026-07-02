//! Global toggle for animated ("smooth") mouse-wheel scrolling.
//!
//! The setting that backs this lives in the `app` crate
//! (`general.smooth_scrolling`), but the code that animates wheel scrolling
//! lives in the lower-level `warpui` windowing layer, which cannot depend on
//! `app`. This module is the small, dependency-free bridge between the two:
//! the app pushes the current setting value here, and the windowing layer
//! reads it when deciding whether to animate a discrete wheel delta.

use std::sync::atomic::{AtomicBool, Ordering};

static SMOOTH_SCROLL_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables animated mouse-wheel scrolling. Called by the app
/// whenever the `general.smooth_scrolling` setting is loaded or changed.
pub fn set_smooth_scroll_enabled(enabled: bool) {
    SMOOTH_SCROLL_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether animated mouse-wheel scrolling is currently enabled.
pub fn smooth_scroll_enabled() -> bool {
    SMOOTH_SCROLL_ENABLED.load(Ordering::Relaxed)
}
