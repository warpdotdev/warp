// Re-export a couple winit types and modules as the concrete implementations
// for the linux platform.
pub use crate::windowing::winit::app::App;

use crate::{
    windowing::{self, WindowingSystem},
    AppContext,
};

use super::{app::AppBackend, AsInnerMut};

/// An extension trait defining additional configurability for
/// applications when running on Linux.
pub trait AppBuilderExt {
    /// Sets the value to use for WM_CLASS (when running under X11) or app_id
    /// (when running under Wayland).
    ///
    /// This is used to identify the application and link it properly to its
    /// .desktop file and associated resources (like app icons).
    fn set_window_class(&mut self, window_class: String);

    /// Whether or not to force the use of XWayland for users running Wayland.
    fn force_x11(&mut self, force_x11: bool);
}

impl AppBuilderExt for super::AppBuilder {
    fn set_window_class(&mut self, window_class: String) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.set_window_class(window_class),
            AppBackend::Headless(_) => (),
        }
    }

    fn force_x11(&mut self, force_x11: bool) {
        match self.as_inner_mut() {
            AppBackend::CurrentPlatform(app) => app.force_x11(force_x11),
            AppBackend::Headless(_) => (),
        }
    }
}

/// Retrieves the windowing system that the user is running before the event loop is created.
pub fn user_windowing_system() -> WindowingSystem {
    // This mirrors winit's logic [here](https://github.com/rust-windowing/winit/blob/4cd6877e8e19e7e1ba957a409394dca1af4afcdd/src/platform_impl/linux/mod.rs#L735-L745).
    if std::env::var("WAYLAND_DISPLAY")
        .ok()
        .filter(|var| !var.is_empty())
        .or_else(|| std::env::var("WAYLAND_SOCKET").ok())
        .filter(|var| !var.is_empty())
        .is_some()
    {
        WindowingSystem::Wayland
    } else {
        WindowingSystem::X11
    }
}

pub fn is_wsl() -> bool {
    command::wsl::is_wsl()
}

pub fn is_wayland_env_var_set() -> bool {
    std::env::var_os("WARP_ENABLE_WAYLAND")
        .is_some_and(|warp_enable_wayland| warp_enable_wayland.eq_ignore_ascii_case("1"))
}

pub fn windowing_system_is_customizable(app: &AppContext) -> bool {
    !is_wayland_env_var_set()
        && app
            .windows()
            .windowing_system()
            .is_some_and(|windowing_system| {
                matches!(
                    windowing_system,
                    windowing::System::X11 { is_x_wayland: true } | windowing::System::Wayland
                )
            })
}
