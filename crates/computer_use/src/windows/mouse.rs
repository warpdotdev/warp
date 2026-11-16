//! Mouse input handling for Windows.
//!
//! Absolute positioning is done via `SetCursorPos` (physical pixel coordinates on DPI-aware
//! processes; logical coordinates otherwise), which avoids the normalized-coordinate math
//! required by `SendInput` with `MOUSEEVENTF_ABSOLUTE`. Button presses, releases, and wheel
//! scrolls go through `SendInput`.

use std::ffi::c_void;
use std::mem::size_of;

use pathfinder_geometry::vector::Vector2I;
use windows::Win32::Foundation::{GetLastError, POINT};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SPI_GETWHEELSCROLLCHARS, SPI_GETWHEELSCROLLLINES,
    SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursorPos,
    SystemParametersInfoW,
};

use super::dpi::DpiAwarenessGuard;
use crate::{MouseButton, ScrollDirection, ScrollDistance};

/// One wheel "click" in `MOUSEEVENTF_WHEEL`/`MOUSEEVENTF_HWHEEL` units.
/// See the Win32 `WHEEL_DELTA` constant.
const WHEEL_DELTA: i32 = 120;

/// `XBUTTON1` / `XBUTTON2` values for `mouseData` when sending X-button events. These match the
/// Win32 header values and are not currently exposed through the `windows` crate's
/// `KeyboardAndMouse` module.
const XBUTTON1: u32 = 0x0001;
const XBUTTON2: u32 = 0x0002;

/// Nominal line height (in logical pixels at 100% scale) used as the baseline when translating
/// the user's `SPI_GETWHEELSCROLLLINES` setting into a pixel-per-click factor. The actual line
/// height we use is this value scaled by the cursor-monitor DPI over `USER_DEFAULT_SCREEN_DPI`,
/// so `ScrollDistance::Pixels` stays proportional to what the user sees on HiDPI displays (~20px
/// at 125% scale, ~24px at 150%) — including secondary monitors in mixed-DPI setups.
const NOMINAL_LINE_HEIGHT_PX: i32 = 16;

/// The "default" (1x) DPI value Windows reports; matches `USER_DEFAULT_SCREEN_DPI`.
const DEFAULT_DPI: u32 = 96;

/// Fallback used if `SPI_GETWHEELSCROLLLINES` is unavailable or returns a sentinel value (e.g.,
/// `WHEEL_PAGESCROLL`). Matches the documented Windows default of three lines per wheel click.
const DEFAULT_WHEEL_SCROLL_LINES: u32 = 3;

/// Upper bound applied to the user's `SPI_GETWHEELSCROLL{LINES,CHARS}` setting before it's
/// multiplied by `NOMINAL_LINE_HEIGHT_PX`. Without this clamp an unusually large configured value
/// (or a corrupt value written by a partial `SystemParametersInfoW` call) would produce a
/// huge pixels-per-click factor, forcing every small pixel scroll to round up to a single click.
const MAX_WHEEL_SCROLL_LINES: u32 = 100;

/// Manages mouse state and posts mouse events to the system.
pub struct Mouse;

impl Default for Mouse {
    fn default() -> Self {
        Self::new()
    }
}

impl Mouse {
    pub fn new() -> Self {
        Self
    }

    pub fn move_to(&mut self, target: Vector2I) -> Result<(), String> {
        // Ensure this thread is per-monitor-v2 DPI aware so `SetCursorPos` receives coordinates in
        // physical pixels rather than being scaled.
        let _dpi_guard = DpiAwarenessGuard::enter_per_monitor_v2();
        // SAFETY: SetCursorPos accepts any i32 coordinates; it will clamp to the available display
        // region. This has no preconditions.
        unsafe { SetCursorPos(target.x(), target.y()) }.map_err(|e| {
            format!(
                "Failed to move cursor to ({}, {}): {e}",
                target.x(),
                target.y()
            )
        })?;
        // Also emit a `SendInput` mouse-move so consumers of raw input (`WM_INPUT`) and low-level
        // mouse hooks (`WH_MOUSE_LL`) — common in games, anti-cheat, and some remote-desktop
        // clients — see the motion. `SetCursorPos` alone only posts `WM_MOUSEMOVE` to the window
        // under the cursor. Best-effort: we ignore a `SendInput` failure here because the cursor
        // is already at the target position from `SetCursorPos` above.
        match normalized_virtual_desk_coords(target) {
            Some((dx, dy)) => {
                let _ = send_mouse_event_with_coords(
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    0,
                    dx,
                    dy,
                );
            }
            None => {
                log::warn!(
                    "Skipping WM_INPUT-visible cursor move for ({}, {}): invalid virtual-screen \
                     metrics",
                    target.x(),
                    target.y(),
                );
            }
        }
        Ok(())
    }

    pub fn button_down(&mut self, button: &MouseButton) -> Result<(), String> {
        let (flags, mouse_data) = button_down_event(button);
        send_mouse_event(flags, mouse_data)
    }

    pub fn button_up(&mut self, button: &MouseButton) -> Result<(), String> {
        let (flags, mouse_data) = button_up_event(button);
        send_mouse_event(flags, mouse_data)
    }

    pub fn current_position(&mut self) -> Result<Vector2I, String> {
        // Match the DPI awareness used by `move_to` so the reported position is in the same
        // coordinate space as the coordinates we send.
        let _dpi_guard = DpiAwarenessGuard::enter_per_monitor_v2();
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: `point` is a valid, writable `POINT`.
        unsafe { GetCursorPos(&mut point) }
            .map_err(|e| format!("Failed to get cursor position: {e}"))?;
        Ok(Vector2I::new(point.x, point.y))
    }

    pub fn scroll(
        &mut self,
        direction: &ScrollDirection,
        distance: &ScrollDistance,
    ) -> Result<(), String> {
        // Match the DPI awareness used by `move_to` / `current_position` so
        // `cursor_monitor_dpi` (invoked via `pixels_per_click` → `scaled_line_height_px`) resolves
        // the cursor's monitor in physical pixels even when the host process is not manifest-
        // declared per-monitor-v2 DPI aware.
        let _dpi_guard = DpiAwarenessGuard::enter_per_monitor_v2();
        // Windows expresses wheel amounts in multiples of WHEEL_DELTA (120 per "click"). Positive
        // values scroll forward (up/right); negative values scroll backward (down/left).
        // Both `Clicks` and `Pixels` are treated as unsigned magnitudes here; the direction is
        // encoded separately in `ScrollDirection`, so we `saturating_abs()` either branch to avoid
        // a negative distance canceling out `ScrollDirection` and scrolling the wrong way.
        //
        // Resolve axis flags and sign from `direction` in a single match so the
        // vertical/horizontal decision lives in exactly one place.
        let (flags, sign) = match direction {
            ScrollDirection::Up => (MOUSEEVENTF_WHEEL, 1),
            ScrollDirection::Down => (MOUSEEVENTF_WHEEL, -1),
            // Horizontal wheel: positive = right, negative = left.
            ScrollDirection::Right => (MOUSEEVENTF_HWHEEL, 1),
            ScrollDirection::Left => (MOUSEEVENTF_HWHEEL, -1),
        };
        let is_horizontal = flags == MOUSEEVENTF_HWHEEL;

        let magnitude: i32 = match distance {
            ScrollDistance::Clicks(clicks) => clicks.saturating_abs().saturating_mul(WHEEL_DELTA),
            ScrollDistance::Pixels(pixels) => {
                // Derive pixels-per-click from the user's actual system setting
                // (`SPI_GETWHEELSCROLLLINES` for vertical, `SPI_GETWHEELSCROLLCHARS` for
                // horizontal) so we respect mouse / trackpad driver configuration instead of a
                // hard-coded constant.
                //
                // `pixels` is treated as a magnitude because the direction is encoded separately
                // in `ScrollDirection`. A zero-pixel request is a no-op; non-zero requests below
                // `pixels_per_click` round up to a single click so the scroll is still observable.
                let abs_pixels = pixels.saturating_abs();
                if abs_pixels == 0 {
                    0
                } else {
                    let per_click = pixels_per_click(is_horizontal);
                    let clicks = (abs_pixels / per_click).clamp(1, i32::MAX / WHEEL_DELTA);
                    clicks.saturating_mul(WHEEL_DELTA)
                }
            }
        };
        let signed_amount = magnitude.saturating_mul(sign);

        // Skip zero-delta wheel events (e.g., `Clicks(0)` or `Pixels(0)`). Windows would still
        // dispatch them as observable `WM_MOUSEWHEEL`s even though no scrolling happens.
        if signed_amount == 0 {
            return Ok(());
        }
        // `mouseData` is declared as a `u32` but `MOUSEEVENTF_WHEEL`/`HWHEEL` reinterpret the
        // bits as a signed `i32` (positive scrolls up/right, negative scrolls down/left). `as u32`
        // on an `i32` is the well-defined two's-complement reinterpretation we want here.
        send_mouse_event(flags, signed_amount as u32)
    }
}

/// Returns the number of pixels that correspond to one wheel "click" on the requested axis,
/// derived from the user's `SPI_GETWHEELSCROLL{LINES,CHARS}` setting. Horizontal wheel on Windows
/// is conventionally driven by `SPI_GETWHEELSCROLLCHARS`, not `SPI_GETWHEELSCROLLLINES`. Falls
/// back to the Windows default (3 lines/chars) if the setting is unavailable or set to the
/// `WHEEL_PAGESCROLL` sentinel.
fn pixels_per_click(is_horizontal: bool) -> i32 {
    let spi: SYSTEM_PARAMETERS_INFO_ACTION = if is_horizontal {
        SPI_GETWHEELSCROLLCHARS
    } else {
        SPI_GETWHEELSCROLLLINES
    };
    let mut units: u32 = DEFAULT_WHEEL_SCROLL_LINES;
    // SAFETY: `units` is a valid writable u32 and we pass its size implicitly via the fixed-layout
    // `SPI_GETWHEELSCROLL*` contract. The call does not retain the pointer beyond the call.
    let result = unsafe {
        SystemParametersInfoW(
            spi,
            0,
            Some(&mut units as *mut u32 as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    // If the call fails, or the user has configured page scrolling (WHEEL_PAGESCROLL == u32::MAX),
    // fall back to the documented default.
    if result.is_err() || units == 0 || units == u32::MAX {
        units = DEFAULT_WHEEL_SCROLL_LINES;
    }
    // Clamp to `[1, MAX_WHEEL_SCROLL_LINES]` so this function's caller can always divide by the
    // result without risking a divide-by-zero (even if a future refactor drops the `== 0` guard
    // above) and so an unusually large setting can't produce a huge pixels-per-click factor.
    let units = units.clamp(1, MAX_WHEEL_SCROLL_LINES);
    (units as i32).saturating_mul(scaled_line_height_px())
}

/// Returns the nominal line height in *physical* pixels for the monitor the cursor is currently
/// on, so `ScrollDistance::Pixels` translations stay proportional to the user's display scaling
/// even on mixed-DPI multi-monitor setups (which `GetDpiForSystem` can't express).
fn scaled_line_height_px() -> i32 {
    let dpi = cursor_monitor_dpi();
    // `NOMINAL_LINE_HEIGHT_PX * dpi / 96`, saturating; integer math is sufficient at the
    // precision we care about here.
    let scaled = (NOMINAL_LINE_HEIGHT_PX as i64).saturating_mul(dpi as i64) / DEFAULT_DPI as i64;
    // Re-clamp back into `i32` range and ensure at least 1 so callers can divide safely.
    scaled.clamp(1, i32::MAX as i64) as i32
}

/// Returns the effective DPI of the monitor currently containing the cursor, falling back to
/// `DEFAULT_DPI` if any step of the query fails. Using the cursor's monitor (rather than the
/// primary) keeps `ScrollDistance::Pixels` proportional to the display the user is actually
/// scrolling on.
fn cursor_monitor_dpi() -> u32 {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a valid, writable `POINT`.
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return DEFAULT_DPI;
    }
    // SAFETY: `MonitorFromPoint` has no preconditions; `MONITOR_DEFAULTTONEAREST` guarantees a
    // non-null handle when any monitor exists.
    let hmonitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    if hmonitor.is_invalid() {
        return DEFAULT_DPI;
    }
    let mut dpi_x: u32 = 0;
    let mut dpi_y: u32 = 0;
    // SAFETY: `hmonitor` is valid; `dpi_x`/`dpi_y` are writable u32s.
    if unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        return DEFAULT_DPI;
    }
    // Guard against the (unexpected) 0 return so we never produce a 0-pixel line height.
    if dpi_x == 0 { DEFAULT_DPI } else { dpi_x }
}

/// Translates a virtual-screen pixel coordinate into the `[0, 65535]` normalized absolute
/// coordinates `SendInput` expects when `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK` is set.
/// Returns `None` if the virtual screen metrics are unusable.
fn normalized_virtual_desk_coords(target: Vector2I) -> Option<(i32, i32)> {
    // SAFETY: `GetSystemMetrics` has no preconditions.
    let virt_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let virt_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let virt_w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let virt_h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if virt_w <= 0 || virt_h <= 0 {
        return None;
    }
    // Normalize into `[0, 65535]` across the virtual desktop. Use i64 to avoid overflow when the
    // virtual screen is large.
    let dx = (target.x() as i64 - virt_x as i64) * 65535 / virt_w as i64;
    let dy = (target.y() as i64 - virt_y as i64) * 65535 / virt_h as i64;
    Some((dx.clamp(0, 65535) as i32, dy.clamp(0, 65535) as i32))
}

/// Returns the `(flags, mouseData)` pair for a mouse button-down event.
fn button_down_event(button: &MouseButton) -> (MOUSE_EVENT_FLAGS, u32) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, 0),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, 0),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, 0),
        MouseButton::Back => (MOUSEEVENTF_XDOWN, XBUTTON1),
        MouseButton::Forward => (MOUSEEVENTF_XDOWN, XBUTTON2),
    }
}

/// Returns the `(flags, mouseData)` pair for a mouse button-up event.
fn button_up_event(button: &MouseButton) -> (MOUSE_EVENT_FLAGS, u32) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTUP, 0),
        MouseButton::Right => (MOUSEEVENTF_RIGHTUP, 0),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
        MouseButton::Back => (MOUSEEVENTF_XUP, XBUTTON1),
        MouseButton::Forward => (MOUSEEVENTF_XUP, XBUTTON2),
    }
}

/// Dispatches a single mouse event via `SendInput` with `dx` = `dy` = 0 (i.e., at the current
/// cursor position). Use [`send_mouse_event_with_coords`] for absolute-positioned events such as
/// `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE`.
fn send_mouse_event(flags: MOUSE_EVENT_FLAGS, mouse_data: u32) -> Result<(), String> {
    send_mouse_event_with_coords(flags, mouse_data, 0, 0)
}

/// Dispatches a single mouse event via `SendInput`. `dx`/`dy` are interpreted per Win32 docs:
/// absolute `[0, 65535]` normalized coordinates when `MOUSEEVENTF_ABSOLUTE` is set, otherwise
/// relative movement.
fn send_mouse_event_with_coords(
    flags: MOUSE_EVENT_FLAGS,
    mouse_data: u32,
    dx: i32,
    dy: i32,
) -> Result<(), String> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    // SAFETY: `input` is a valid `INPUT` of mouse type, with the correct size passed to
    // `SendInput`. The call does not retain any pointer beyond the call.
    let sent = unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
    if sent != 1 {
        // SAFETY: `GetLastError` has no preconditions; reads the calling thread's last-error.
        let last_error = unsafe { GetLastError() }.0;
        return Err(format!(
            "SendInput failed to dispatch mouse event (flags={:#x}, GetLastError={last_error})",
            flags.0,
        ));
    }
    Ok(())
}
