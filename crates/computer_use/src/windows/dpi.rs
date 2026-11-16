//! Thread-local DPI-awareness helper for Windows computer_use actions.
//!
//! Several Win32 APIs this crate calls (`SetCursorPos`, `GetCursorPos`,
//! `GetSystemMetrics(SM_*VIRTUALSCREEN)`, `BitBlt`, …) return *logical* coordinates if the
//! calling thread is not DPI-aware, which causes mis-located clicks and scaled/cropped screenshots
//! on HiDPI monitors.
//!
//! Rather than relying on the host process manifest, we opt every computer_use operation into
//! per-monitor-v2 awareness for the duration of the call via [`DpiAwarenessGuard`].

use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};

/// RAII guard that requests per-monitor-v2 DPI awareness for the current thread and restores the
/// previous context when dropped.
///
/// Requires Windows 10 version 1703 or newer (when `PER_MONITOR_AWARE_V2` shipped — 1607 only
/// had V1). On older systems (or when the process awareness cannot be overridden)
/// `SetThreadDpiAwarenessContext` returns a null context; in that case this guard is a no-op.
pub(super) struct DpiAwarenessGuard {
    previous: Option<DPI_AWARENESS_CONTEXT>,
}

impl DpiAwarenessGuard {
    /// Enters per-monitor-v2 DPI awareness for the calling thread.
    pub(super) fn enter_per_monitor_v2() -> Self {
        // SAFETY: `SetThreadDpiAwarenessContext` has no preconditions and mutates only
        // thread-local state.
        let prev =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let previous = if prev.0.is_null() { None } else { Some(prev) };
        Self { previous }
    }
}

impl Drop for DpiAwarenessGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.previous {
            // SAFETY: `prev` was returned by a prior successful call to
            // `SetThreadDpiAwarenessContext` on this same thread.
            unsafe {
                let _ = SetThreadDpiAwarenessContext(prev);
            }
        }
    }
}
