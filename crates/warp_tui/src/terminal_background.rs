//! The host terminal's default colors, captured once by the startup probe.
//!
//! Stored as process-wide state (like `ChannelState` and feature flags)
//! rather than an app-model singleton: the probe runs once in `session::init`
//! before any render and the result never changes for the process's lifetime.
//! When the probe never ran (feature flag off, tests, non-tty), readers see
//! empty colors and fall back to theme-derived styling.

use std::sync::OnceLock;

use warpui_core::runtime::ProbedTerminalColors;

static PROBED_COLORS: OnceLock<ProbedTerminalColors> = OnceLock::new();

/// Records the startup probe's result. Later calls are no-ops; the first
/// result wins for the lifetime of the process.
pub(crate) fn set_probed_colors(colors: ProbedTerminalColors) {
    let _ = PROBED_COLORS.set(colors);
}

/// The probed terminal colors, or empty colors when the probe never ran or
/// the terminal did not answer.
pub(crate) fn probed_colors() -> ProbedTerminalColors {
    PROBED_COLORS.get().copied().unwrap_or_default()
}
