use windows::Win32::Foundation::{FALSE, HWND, TRUE};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows_core::BOOL;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid WindowHandle")]
    InvalidWindowHandle,
    #[error("Unknown error")]
    Other(#[from] windows::core::Error),
}

/// Extension trait for Windows specific logic on a [`winit::window::Window`].
pub trait WindowExt {
    /// "Cloaks" the window. A cloaked window is one that is invisible, but can still be drawn to.
    fn set_cloaked(&self, cloaked: bool) -> Result<(), Error>;
}

impl WindowExt for Window {
    fn set_cloaked(&self, cloaked: bool) -> Result<(), Error> {
        let Ok(RawWindowHandle::Win32(handle)) = self
            .window_handle()
            .map(|window_handle| window_handle.as_raw())
        else {
            return Err(Error::InvalidWindowHandle);
        };

        let value = if cloaked { TRUE } else { FALSE };
        unsafe {
            DwmSetWindowAttribute(
                HWND(handle.hwnd.get() as _),
                DWMWA_CLOAK,
                &value as *const BOOL as *const _,
                size_of::<BOOL>() as u32,
            )?
        }

        Ok(())
    }
}
