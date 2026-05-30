use std::mem::size_of;

use windows::Win32::Foundation::{FALSE, HWND, TRUE};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_CLOAK, MARGINS,
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

/// Accent state values for `DWMWA_ACCENT_POLICY` (undocumented but widely used).
#[repr(u32)]
#[derive(Clone, Copy)]
enum AccentState {
    Disabled = 0,
    _EnableGradient = 1,
    _EnableTransparentGradient = 2,
    _EnableBlurBehind = 3,
    EnableAcrylicBlurBehind = 4,
    _EnableHostBackdrop = 5,
}

/// `ACCENT_POLICY` struct used with `DWMWA_ACCENT_POLICY` (attribute 19).
/// The `gradient_color` field is an ABGR color value that controls the tint
/// behind the acrylic blur effect.
#[repr(C)]
#[derive(Clone, Copy)]
struct AccentPolicy {
    accent_state: AccentState,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

/// `DWMWA_ACCENT_POLICY` attribute constant.
const DWMWA_ACCENT_POLICY: i32 = 19;

/// Default acrylic tint color: dark semi-transparent black in ABGR format.
/// Alpha=0xCC (~80%), Blue=0x00, Green=0x00, Red=0x00.
/// This provides a dark tint that complements dark mode themes instead of
/// the default light frosted appearance.
const DEFAULT_ACRYLIC_TINT: u32 = 0xCC000000;

/// Extension trait for Windows specific logic on a [`winit::window::Window`].
pub trait WindowExt {
    /// "Cloaks" the window. A cloaked window is one that is invisible, but can still be drawn to.
    fn set_cloaked(&self, cloaked: bool) -> Result<(), Error>;

    /// Enables or disables the acrylic blur backdrop with a dark tint color.
    ///
    /// This replaces winit's `BackdropType::TransientWindow` approach, which
    /// delegates to the OS default tint (a light frosted look). By using the
    /// legacy `DWMWA_ACCENT_POLICY` API directly, we can set a custom dark
    /// gradient color that looks correct with dark terminal themes.
    fn set_acrylic_backdrop(&self, enabled: bool) -> Result<(), Error>;
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

    fn set_acrylic_backdrop(&self, enabled: bool) -> Result<(), Error> {
        let Ok(RawWindowHandle::Win32(handle)) = self
            .window_handle()
            .map(|window_handle| window_handle.as_raw())
        else {
            return Err(Error::InvalidWindowHandle);
        };

        let hwnd = HWND(handle.hwnd.get() as _);

        let policy = if enabled {
            AccentPolicy {
                accent_state: AccentState::EnableAcrylicBlurBehind,
                accent_flags: 2, // ACCENT_FLAG_DRAW_ALL
                gradient_color: DEFAULT_ACRYLIC_TINT,
                animation_id: 0,
            }
        } else {
            AccentPolicy {
                accent_state: AccentState::Disabled,
                accent_flags: 0,
                gradient_color: 0,
                animation_id: 0,
            }
        };

        unsafe {
            // Set the accent policy to enable/disable acrylic with our tint color.
            DwmSetWindowAttribute(
                hwnd,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(DWMWA_ACCENT_POLICY),
                &policy as *const AccentPolicy as *const _,
                size_of::<AccentPolicy>() as u32,
            )?;

            // Extend the DWM frame into the entire client area so the acrylic
            // effect covers the whole window, not just the title bar.
            let margins = if enabled {
                MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                }
            } else {
                MARGINS {
                    cxLeftWidth: 0,
                    cxRightWidth: 0,
                    cyTopHeight: 0,
                    cyBottomHeight: 0,
                }
            };
            DwmExtendFrameIntoClientArea(hwnd, &margins)?;
        }

        Ok(())
    }
}
