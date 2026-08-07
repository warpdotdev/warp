//! Functionality relating to services that the application provides
//! to the host system.
//!
//! For example, on macOS, this module sets up integrations with
//! Finder such that the user can open a new Warp tab or window
//! in a given directory.

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
pub mod windows;

use warpui::AppContext;

/// Initializes application services.
///
/// `owns_desktop_application_service` controls whether this process registers the desktop
/// application service that a local launch forwards to. Only the user's primary desktop instance
/// may own it. It is consumed by the Linux/FreeBSD backend only.
pub fn init(_owns_desktop_application_service: bool, _ctx: &mut AppContext) {
    log::info!("Initializing app services");

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    linux::init(_owns_desktop_application_service, _ctx);
    #[cfg(target_os = "macos")]
    mac::init();
    #[cfg(windows)]
    windows::init(_ctx);
}

pub fn teardown(_ctx: &mut AppContext) {
    log::info!("Tearing down app services...");

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    linux::teardown(_ctx);
}
