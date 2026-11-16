//! Login-item registration — makes Warp start automatically when the user
//! signs in to their OS.
//!
//! The user-facing toggle and "already registered" bookkeeping live on
//! [`crate::terminal::general_settings::GeneralSettings`]. This module owns
//! the platform-specific register/unregister logic for each OS where the
//! feature is supported.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use warp_core::channel::ChannelState;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use warpui::AppContext;

/// Reconciles whether Warp is registered to launch at login with the user's
/// current preference.
///
/// Respects the existing `app_added_as_login_item` bookkeeping so a user who
/// removed Warp from their OS's startup UI isn't silently re-added — the
/// platform backends only run the registration flow when the setting was
/// explicitly re-toggled.
///
/// Skipped entirely when the `WARP_INTEGRATION` env var is set, so integration
/// tests never touch the user's real login items / registry. Also skipped for
/// non-release-bundle builds (e.g. `cargo run`), so developer machines don't
/// auto-launch `target/debug/{warp,openwarp,...}` at sign-in.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn maybe_register_app_as_login_item(ctx: &mut AppContext) {
    if std::env::var("WARP_INTEGRATION").is_ok() {
        log::debug!("Not registering as a login item in integration tests");
        return;
    }
    if !ChannelState::is_release_bundle() {
        log::debug!("Not a release bundle, skipping login-item registration");
        return;
    }
    #[cfg(target_os = "macos")]
    macos::maybe_register_app_as_login_item(ctx);
    #[cfg(target_os = "windows")]
    windows::maybe_register_app_as_login_item(ctx);
}
