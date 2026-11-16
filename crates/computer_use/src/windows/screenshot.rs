//! Screenshot capture for Windows using GDI.
//!
//! Captures the full virtual screen (all monitors) or a sub-region of it by compositing the screen
//! contents into an offscreen DIB via `BitBlt`, then reading the pixel data out with `GetDIBits`.
//! The resulting BGRA data is converted to RGBA and handed off to the shared screenshot processing
//! pipeline.

use std::mem::size_of;

use image::{DynamicImage, RgbaImage};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::dpi::DpiAwarenessGuard;
use crate::{Screenshot, ScreenshotParams};

/// Captures a screenshot of the full virtual screen (or a region of it).
///
/// On multi-monitor setups the virtual screen spans every display and its origin may be at
/// negative coordinates (e.g., if a secondary monitor is positioned left of the primary).
/// `ScreenshotRegion::validate` currently requires non-negative region coordinates, so callers
/// cannot reach areas with negative virtual-screen coordinates via region captures; those areas
/// are still included in the full-screen capture.
///
/// TODO: relax the non-negative check in `ScreenshotRegion::validate`
/// (`crates/computer_use/src/lib.rs`) so the region path can reach monitors positioned above /
/// left of the primary. The Win32 side of this module already supports negative coordinates; the
/// restriction is shared across Mac / Linux / Windows, so this is a platform-neutral follow-up.
pub fn take(params: ScreenshotParams) -> Result<Screenshot, String> {
    // Opt this thread into per-monitor-v2 DPI awareness so the virtual-screen metrics and `BitBlt`
    // all operate in physical pixels, regardless of the host process manifest. Dropped at end of
    // scope to restore prior context.
    let _dpi_guard = DpiAwarenessGuard::enter_per_monitor_v2();

    // SAFETY: GetSystemMetrics has no preconditions.
    let virt_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let virt_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let virt_w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virt_h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if virt_w <= 0 || virt_h <= 0 {
        return Err(format!(
            "Virtual screen has invalid dimensions ({virt_w}x{virt_h})"
        ));
    }
    let max_x = virt_x.saturating_add(virt_w);
    let max_y = virt_y.saturating_add(virt_h);

    // Determine the region to capture. Coordinates are in the same space as the virtual screen,
    // matching `SetCursorPos` (pixel coordinates for DPI-aware processes, logical coordinates
    // otherwise).
    let (src_x, src_y, width, height) = if let Some(region) = params.region {
        region.validate()?;
        let w = region.bottom_right.x() - region.top_left.x();
        let h = region.bottom_right.y() - region.top_left.y();
        // Validate against both ends of the virtual screen. `ScreenshotRegion::validate` only
        // enforces `top_left >= 0`, which can't catch the uncommon case where the virtual-screen
        // origin itself is positive (e.g., primary monitor repositioned) — without the explicit
        // `< virt_x/virt_y` check, `BitBlt` would silently sample pixels off the virtual screen.
        if region.top_left.x() < virt_x
            || region.top_left.y() < virt_y
            || region.bottom_right.x() > max_x
            || region.bottom_right.y() > max_y
        {
            return Err(format!(
                "Screenshot region ({}, {})-({}, {}) exceeds virtual screen bounds \
                 ({virt_x}, {virt_y})-({max_x}, {max_y})",
                region.top_left.x(),
                region.top_left.y(),
                region.bottom_right.x(),
                region.bottom_right.y(),
            ));
        }
        (region.top_left.x(), region.top_left.y(), w, h)
    } else {
        (virt_x, virt_y, virt_w, virt_h)
    };

    let rgba = capture_rgba(src_x, src_y, width, height)?;

    let img = RgbaImage::from_raw(width as u32, height as u32, rgba)
        .ok_or_else(|| "Failed to construct image from GDI pixel data".to_string())?;
    let img = DynamicImage::ImageRgba8(img);

    crate::screenshot_utils::process_screenshot(img, params)
}

/// Captures the screen into a freshly allocated RGBA buffer.
///
/// The caller is responsible for providing a valid region on the virtual screen; this function
/// does not clip coordinates itself. Source coordinates are in virtual-screen space (the same
/// space as `SetCursorPos`).
fn capture_rgba(src_x: i32, src_y: i32, width: i32, height: i32) -> Result<Vec<u8>, String> {
    /// `HGDI_ERROR` = `(HGDIOBJ)(LONG_PTR)-1`, returned by `SelectObject` on a type mismatch.
    /// Named here because `HGDIOBJ::is_invalid()` only checks for NULL, and the `windows` crate
    /// we're on doesn't expose an `HGDI_ERROR` constant we can compare against directly.
    const HGDI_ERROR_SENTINEL: isize = -1;

    // Use RAII guards so every GDI handle is released even on early returns.
    let screen_dc = ScreenDc::acquire()?;
    let mem_dc = MemoryDc::create_compatible(screen_dc.handle())?;
    let bitmap = Bitmap::create_compatible(screen_dc.handle(), width, height)?;

    // SAFETY: `mem_dc` and `bitmap` are valid GDI handles owned by the guards.
    let prev_object = unsafe { SelectObject(mem_dc.handle(), bitmap.handle().into()) };
    // `SelectObject` returns NULL on general failure or `HGDI_ERROR` on type mismatch; check both.
    if prev_object.is_invalid() || prev_object.0 as isize == HGDI_ERROR_SENTINEL {
        return Err("SelectObject failed for screenshot bitmap".to_string());
    }
    // RAII-restore the previously-selected object on drop so the `Bitmap` guard can safely
    // `DeleteObject` it even if `BitBlt` / `GetDIBits` panic (per MSDN, `DeleteObject` fails on
    // an HBITMAP still selected into a DC, which would leak both the bitmap and DC).
    let _restore_select_guard = SelectObjectGuard {
        dc: mem_dc.handle(),
        prev_object,
    };

    // SAFETY: both DCs are valid; BitBlt reads the screen and writes into the compatible memory
    // DC we just prepared.
    unsafe {
        BitBlt(
            mem_dc.handle(),
            0,
            0,
            width,
            height,
            Some(screen_dc.handle()),
            src_x,
            src_y,
            SRCCOPY,
        )
    }
    .map_err(|e| format!("BitBlt failed while capturing screen: {e}"))?;

    let buffer = read_bitmap_bits(mem_dc.handle(), bitmap.handle(), width, height)?;
    Ok(convert_bgra_to_rgba(buffer))
}

/// RAII guard that restores a previously-`SelectObject`'d GDI object into `dc` when dropped,
/// making the select/restore lifecycle panic-safe.
struct SelectObjectGuard {
    dc: HDC,
    prev_object: HGDIOBJ,
}

impl Drop for SelectObjectGuard {
    fn drop(&mut self) {
        // SAFETY: `dc` is the same DC the caller used with `SelectObject`; `prev_object` is the
        // handle that `SelectObject` returned. Both are still valid at this point because the
        // underlying DC / bitmap guards own them and haven't been dropped yet (Rust drops fields
        // and locals in reverse declaration order; this guard is declared before the outer
        // bitmap / DC guards go out of scope).
        unsafe { SelectObject(self.dc, self.prev_object) };
    }
}

/// Reads `width x height` pixels from `bitmap` as 32-bit top-down BGRA.
fn read_bitmap_bits(
    mem_dc: HDC,
    bitmap: HBITMAP,
    width: i32,
    height: i32,
) -> Result<Vec<u8>, String> {
    // BITMAPINFO has a flexible-array of color entries at the end; for 32bpp BI_RGB we don't need
    // any, and the single-element default is sufficient.
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative height requests a top-down DIB, so the first row in the buffer corresponds
            // to the top of the image.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: Default::default(),
    };

    let byte_count = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| format!("Screenshot dimensions {width}x{height} overflow buffer size"))?;
    let mut buffer = vec![0u8; byte_count];

    // SAFETY: `buffer` is large enough for the requested pixels; `info` is a valid BITMAPINFO
    // describing the requested format. `GetDIBits` does not retain any of the pointers after it
    // returns.
    let scanlines = unsafe {
        GetDIBits(
            mem_dc,
            bitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    // `GetDIBits` returns the number of scan lines actually copied. Anything less than the
    // requested height means the buffer is only partially populated; treat that as a failure so we
    // don't silently decode a truncated image.
    if scanlines != height {
        return Err(format!(
            "GetDIBits copied {scanlines} of {height} scan lines for screenshot"
        ));
    }

    Ok(buffer)
}

/// Converts a tightly packed BGRA buffer (as produced by `GetDIBits` with `biBitCount = 32` and
/// `BI_RGB`) to RGBA in-place.
///
/// GDI does not populate the alpha channel for `BI_RGB`, so we force it to `0xFF` to produce a
/// fully opaque RGBA image.
fn convert_bgra_to_rgba(mut buffer: Vec<u8>) -> Vec<u8> {
    for chunk in buffer.chunks_exact_mut(4) {
        // Swap B and R channels so the 4-byte BGRA pixel becomes RGBA.
        chunk.swap(0, 2);
        chunk[3] = 0xFF;
    }
    buffer
}

// ---------------------------------------------------------------------------
// RAII guards for GDI handles
// ---------------------------------------------------------------------------

/// RAII wrapper for the screen device context.
///
/// `GetDC(NULL)` returns a DC whose coordinate space spans the entire virtual screen, so `BitBlt`
/// can source pixels from any monitor.
struct ScreenDc(HDC);

impl ScreenDc {
    fn acquire() -> Result<Self, String> {
        // SAFETY: `GetDC(None)` returns a DC for the virtual screen or a null handle on failure.
        let hdc = unsafe { GetDC(None) };
        if hdc.is_invalid() {
            return Err("GetDC(NULL) returned a null handle".to_string());
        }
        Ok(Self(hdc))
    }

    fn handle(&self) -> HDC {
        self.0
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a DC returned by `GetDC(None)` and has not been released yet.
        let released = unsafe { ReleaseDC(None, self.0) };
        if released == 0 {
            // Not fatal (the process can still continue), but indicates a handle-lifetime
            // regression worth investigating.
            log::warn!("ReleaseDC returned 0 for the screen DC");
        }
    }
}

/// RAII wrapper for a memory device context created with `CreateCompatibleDC`.
struct MemoryDc(HDC);

impl MemoryDc {
    fn create_compatible(screen: HDC) -> Result<Self, String> {
        // SAFETY: `screen` is a valid DC returned from `GetDC`.
        let hdc = unsafe { CreateCompatibleDC(Some(screen)) };
        if hdc.is_invalid() {
            return Err("CreateCompatibleDC failed".to_string());
        }
        Ok(Self(hdc))
    }

    fn handle(&self) -> HDC {
        self.0
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: `self.0` was created by `CreateCompatibleDC` and has not been deleted yet.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// RAII wrapper for a GDI bitmap handle.
struct Bitmap(HBITMAP);

impl Bitmap {
    fn create_compatible(screen: HDC, width: i32, height: i32) -> Result<Self, String> {
        // SAFETY: `screen` is a valid DC; width and height are positive.
        let hbitmap = unsafe { CreateCompatibleBitmap(screen, width, height) };
        if hbitmap.is_invalid() {
            return Err(format!(
                "CreateCompatibleBitmap failed for {width}x{height} bitmap"
            ));
        }
        Ok(Self(hbitmap))
    }

    fn handle(&self) -> HBITMAP {
        self.0
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: `self.0` was created by `CreateCompatibleBitmap` and has not been deleted yet.
        // It must not be currently selected into a DC; callers restore the previous object before
        // dropping.
        let obj: HGDIOBJ = self.0.into();
        unsafe {
            let _ = DeleteObject(obj);
        }
    }
}
