use std::env;

use super::cursor_theme::non_empty_var;
use super::zbus::{get_max_monitor_scale, get_system_cursor_size};

static ENV_CURSOR_SIZE: &'static &str = &"XCURSOR_SIZE";
static ENV_WAYLAND_DISPLAY: &'static &str = &"WAYLAND_DISPLAY";

/// The default Xcursor size assumed by winit's cursor loading when
/// `XCURSOR_SIZE` is not set.
const DEFAULT_CURSOR_SIZE: u32 = 24;

/// Ensures that `XCURSOR_SIZE` reflects the desktop's cursor size in
/// *physical* pixels on GNOME Wayland sessions.
///
/// On Wayland, winit renders the pointer cursor client-side and loads the
/// Xcursor theme at `XCURSOR_SIZE * surface_scale`. The surface scale of the
/// cursor surface is learned from `wl_surface.enter` events, which Mutter
/// does not send for cursor surfaces, so the scale stays at 1. GNOME is also
/// the one major desktop environment that does not export `XCURSOR_SIZE`
/// into the session environment (KDE does), leaving winit with a 24px
/// fallback. On a HiDPI display with a scale factor of 2 this makes the
/// cursor render at half (or less) of the system cursor's size.
///
/// To compensate, we pre-scale the configured cursor size by the largest
/// monitor scale and export it before winit initializes its Wayland state.
/// This is intentionally gated on Mutter's DisplayConfig D-Bus service:
/// other compositors either send the required enter events or already
/// export `XCURSOR_SIZE`, and pre-scaling there would overshoot.
pub fn ensure_cursor_size() {
    // If the XCURSOR_SIZE value is explicitly set,
    // then we do not want to modify the user's environment
    if non_empty_var(ENV_CURSOR_SIZE).is_some() {
        return;
    }

    // On X11 the cursor size is resolved via Xresources/XSETTINGS instead.
    if non_empty_var(ENV_WAYLAND_DISPLAY).is_none() {
        return;
    }

    // Only Mutter exhibits the missing-enter-events behavior; its
    // DisplayConfig service doubles as the source for the monitor scale
    // and as the detection mechanism for running under GNOME.
    let scale = match get_max_monitor_scale() {
        Ok(scale) => scale,
        Err(err) => {
            log::debug!("Not adjusting XCURSOR_SIZE, no Mutter display config available: {err:#}");
            return;
        }
    };

    let logical_size = get_system_cursor_size().unwrap_or(DEFAULT_CURSOR_SIZE);
    let size = compute_cursor_size(logical_size, scale);
    log::info!("Setting XCURSOR_SIZE={size} (cursor size {logical_size} at monitor scale {scale})");
    env::set_var(ENV_CURSOR_SIZE, size.to_string());
}

/// Converts a cursor size in logical pixels to physical pixels for the
/// given monitor scale factor.
fn compute_cursor_size(logical_size: u32, scale: f64) -> u32 {
    (f64::from(logical_size) * scale.max(1.0)).round() as u32
}

#[cfg(test)]
#[path = "cursor_size_tests.rs"]
mod tests;
