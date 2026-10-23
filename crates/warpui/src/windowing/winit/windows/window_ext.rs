use std::ffi::c_void;

use warpui_core::platform::AcrylicTintColor;
use windows::Win32::Foundation::{COLORREF, FALSE, GetLastError, HWND, TRUE};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
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
    #[error("SetWindowCompositionAttribute failed (GetLastError = {0:#x})")]
    AccentPolicyFailed(u32),
}

/// `WCA_ACCENT_POLICY`, the attribute identifier used with
/// `SetWindowCompositionAttribute` to control a window's blur/acrylic accent.
/// This API is undocumented but stable and widely relied upon (e.g. by
/// Windows Terminal, Firefox, and the `window-vibrancy` crate) as the only way
/// to customize the tint color behind an acrylic blur; winit's system backdrop
/// API has no equivalent.
const WCA_ACCENT_POLICY: u32 = 19;

const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    /// ABGR-encoded gradient color: alpha in the high byte, then blue, green, red.
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttributeData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

unsafe extern "system" {
    /// Undocumented Win32 API (`user32.dll`) used to apply the legacy blur/acrylic
    /// accent policy to a window. Not exposed by the `windows` crate.
    fn SetWindowCompositionAttribute(hwnd: HWND, data: *mut WindowCompositionAttributeData)
    -> BOOL;
}

/// Builds the ABGR gradient color expected by [`AccentPolicy`] from a tint color and
/// an opacity percentage (0-100).
fn gradient_color(tint_color: AcrylicTintColor, tint_opacity: u8) -> u32 {
    let rgb: u32 = match tint_color {
        AcrylicTintColor::Dark => 0x000000,
        AcrylicTintColor::Light => 0xFFFFFF,
    };
    let alpha = (tint_opacity.min(100) as u32 * 255) / 100;
    (alpha << 24) | rgb
}

/// Applies the given accent policy to `hwnd` via `SetWindowCompositionAttribute`.
fn set_accent_policy(hwnd: HWND, mut policy: AccentPolicy) -> Result<(), Error> {
    let mut data = WindowCompositionAttributeData {
        attribute: WCA_ACCENT_POLICY,
        data: &mut policy as *mut AccentPolicy as *mut c_void,
        size: size_of::<AccentPolicy>(),
    };
    let ok = unsafe { SetWindowCompositionAttribute(hwnd, &mut data) };
    if ok.as_bool() {
        Ok(())
    } else {
        Err(Error::AccentPolicyFailed(unsafe { GetLastError().0 }))
    }
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

    /// Enables the Acrylic blur-behind accent with a custom tint, bypassing winit's
    /// system backdrop API (which has no way to customize the tint color and
    /// defaults to a light underpaint that clashes with dark themes).
    fn set_acrylic_tint(&self, tint_color: AcrylicTintColor, tint_opacity: u8)
    -> Result<(), Error>;

    /// Clears any accent policy previously applied by [`WindowExt::set_acrylic_tint`].
    /// Must be called when switching away from the Acrylic backdrop so no stale
    /// native state lingers on the window.
    fn clear_acrylic_tint(&self) -> Result<(), Error>;
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

    fn set_acrylic_tint(
        &self,
        tint_color: AcrylicTintColor,
        tint_opacity: u8,
    ) -> Result<(), Error> {
        let Ok(RawWindowHandle::Win32(handle)) = self
            .window_handle()
            .map(|window_handle| window_handle.as_raw())
        else {
            return Err(Error::InvalidWindowHandle);
        };

        set_accent_policy(
            HWND(handle.hwnd.get() as _),
            AccentPolicy {
                accent_state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
                accent_flags: 0,
                gradient_color: gradient_color(tint_color, tint_opacity),
                animation_id: 0,
            },
        )
    }

    fn clear_acrylic_tint(&self) -> Result<(), Error> {
        let Ok(RawWindowHandle::Win32(handle)) = self
            .window_handle()
            .map(|window_handle| window_handle.as_raw())
        else {
            return Err(Error::InvalidWindowHandle);
        };

        set_accent_policy(
            HWND(handle.hwnd.get() as _),
            AccentPolicy {
                accent_state: ACCENT_DISABLED,
                accent_flags: 0,
                gradient_color: 0,
                animation_id: 0,
            },
        )
    }
}
