//! The host terminal's default colors, captured once by the startup probe,
//! and the transcript theme selection derived from them.
//!
//! Stored as process-wide state (like `ChannelState` and feature flags)
//! rather than an app-model singleton: the probe runs once in `session::init`
//! before any render and the result never changes for the process's lifetime.
//! When the probe never ran (tests, non-tty), readers see empty colors and
//! fall back to theme-derived styling.

use std::sync::OnceLock;

use warp::settings::TuiTheme;
use warp::tui_export::{Appearance, dark_theme, light_theme};
use warp_core::ui::theme::{ColorScheme, WarpTheme};
use warpui::SingletonEntity as _;
use warpui_core::AppContext;
use warpui_core::runtime::{BackgroundLuminance, ProbedTerminalColors, probe_terminal_colors};

static PROBED_COLORS: OnceLock<ProbedTerminalColors> = OnceLock::new();

/// Probes the host terminal for its default colors (via OSC 10/11 — call
/// before the TUI driver takes over stdin), caches the result process-wide
/// for style blending, and returns the selected transcript theme. Auto mode
/// selects light for a light terminal background; dark and undetectable
/// backgrounds keep the TUI's historical dark default.
pub(crate) fn probe_and_select_theme(selected_theme: TuiTheme) -> WarpTheme {
    let probed = probe_terminal_colors();
    set_probed_colors(probed);
    select_theme(selected_theme, probed)
}

fn select_theme(selected_theme: TuiTheme, probed_colors: ProbedTerminalColors) -> WarpTheme {
    match selected_theme {
        TuiTheme::Auto => match probed_colors.background_luminance() {
            BackgroundLuminance::Light => light_theme(),
            BackgroundLuminance::Dark | BackgroundLuminance::Unknown => dark_theme(),
        },
        TuiTheme::Light => light_theme(),
        TuiTheme::Dark => dark_theme(),
    }
}

pub(crate) fn theme_for(selected_theme: TuiTheme) -> WarpTheme {
    select_theme(selected_theme, probed_colors())
}

pub(crate) fn active_theme(ctx: &AppContext) -> TuiTheme {
    match Appearance::as_ref(ctx).theme().inferred_color_scheme() {
        ColorScheme::DarkOnLight => TuiTheme::Light,
        ColorScheme::LightOnDark => TuiTheme::Dark,
    }
}

/// Records the startup probe's result. Later calls are no-ops; the first
/// result wins for the lifetime of the process.
fn set_probed_colors(colors: ProbedTerminalColors) {
    let _ = PROBED_COLORS.set(colors);
}

/// The probed terminal colors, or empty colors when the probe never ran or
/// the terminal did not answer.
pub(crate) fn probed_colors() -> ProbedTerminalColors {
    PROBED_COLORS.get().copied().unwrap_or_default()
}

#[cfg(test)]
#[path = "terminal_background_tests.rs"]
mod tests;
