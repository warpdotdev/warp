pub mod app;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_family = "wasm")]
pub mod wasm;
#[cfg(target_os = "windows")]
pub mod windows;

pub mod headless;

pub mod current {
    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            pub use super::wasm::*;
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
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

pub use app::AppBuilder;
pub use warpui_core::platform::*;

/// Creates the native system clipboard implementation used by the GUI
/// platform delegate without requiring a graphical event loop.
#[cfg(not(target_family = "wasm"))]
pub fn create_system_clipboard() -> anyhow::Result<Box<dyn crate::Clipboard + Send>> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            Ok(Box::new(mac::clipboard::Clipboard::new()?))
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            Ok(Box::new(crate::windowing::winit::linux::LinuxClipboard::new()?))
        } else if #[cfg(target_os = "windows")] {
            Ok(Box::new(crate::windowing::winit::windows::WindowsClipboard::new()?))
        } else {
            anyhow::bail!("System clipboard is unavailable on this platform")
        }
    }
}

/// Returns whether the current device is a mobile device with touch input.
///
/// This is a cross-platform wrapper around the platform-specific implementation.
pub fn is_mobile_device() -> bool {
    #[cfg(target_family = "wasm")]
    {
        wasm::is_mobile_device()
    }
    #[cfg(not(target_family = "wasm"))]
    {
        false
    }
}

/// A trait for accessing internal per-platform concrete implementations
/// through a wrapper type.
#[allow(dead_code)]
trait AsInnerMut<Inner: ?Sized> {
    fn as_inner_mut(&mut self) -> &mut Inner;
}
