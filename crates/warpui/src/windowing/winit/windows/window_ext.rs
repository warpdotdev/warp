use windows::Win32::Foundation::{COLORREF, FALSE, HWND, TRUE};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
    WS_EX_LAYERED,
};
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

    /// Sets the window's uniform opacity (`0.0` fully transparent, `1.0` fully
    /// opaque) using a layered window. Unlike `set_cloaked` or hiding, this keeps
    /// the window in the z-order and does not change focus, which is what the
    /// cross-window tab-drag preview relies on while hovering over a target
    /// window's tab bar.
    fn set_alpha(&self, alpha: f32) -> Result<(), Error>;
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

    fn set_alpha(&self, alpha: f32) -> Result<(), Error> {
        let Ok(RawWindowHandle::Win32(handle)) = self
            .window_handle()
            .map(|window_handle| window_handle.as_raw())
        else {
            return Err(Error::InvalidWindowHandle);
        };

        let hwnd = HWND(handle.hwnd.get() as _);
        let alpha_byte = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;

        // SAFETY: `hwnd` is a valid top-level window handle obtained from winit.
        // `SetLayeredWindowAttributes` requires the `WS_EX_LAYERED` extended
        // style, so add it if it isn't already present. We intentionally leave
        // the style set afterwards: a fully-opaque (alpha 255) layered window
        // composites identically on DWM, which avoids the repaint quirks of
        // toggling the style off when restoring opacity.
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if ex_style & (WS_EX_LAYERED.0 as isize) == 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | (WS_EX_LAYERED.0 as isize));
            }
            SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha_byte, LWA_ALPHA)?;
        }

        Ok(())
    }
}
