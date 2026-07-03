pub mod app;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "windows")]
pub mod windows;

pub mod headless;

pub mod current {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            pub use super::linux::*;
        } else if #[cfg(target_os = "macos")] {
            pub use super::mac::*;
        } else if #[cfg(target_os = "windows")] {
            pub use super::windows::*;
        } else {
            pub use warpui_core::platform::test::*;
        }
    }
}

pub use warpui_core::platform::*;

pub use app::AppBuilder;

/// Returns whether the current device is a mobile device with touch input.
///
/// This is a cross-platform wrapper around the platform-specific implementation.
pub fn is_mobile_device() -> bool {
    false
}

/// A trait for accessing internal per-platform concrete implementations
/// through a wrapper type.
#[allow(dead_code)]
trait AsInnerMut<Inner: ?Sized> {
    fn as_inner_mut(&mut self) -> &mut Inner;
}
