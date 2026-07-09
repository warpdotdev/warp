//! Experimental helper for locating the on-screen window under a point for a given process.
//!
//! PID-targeted synthetic mouse events (`CGEventPostToPid`) bypass the WindowServer's
//! hit-testing, so they arrive at the target process without an associated window. To let
//! AppKit route them, we reconstruct the target window (its number and bounds) so the event
//! can be built as a window-targeted `NSEvent` with window-local coordinates.

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowBounds,
    kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
};
type WindowDictionary = CFDictionary<CFString, CFType>;
type BoundsDictionary = CFDictionary<CFString, CFNumber>;

/// Describes an on-screen window: its window number and bounds in global screen points
/// (top-left origin), matching `kCGWindowBounds` and `CGEvent` location coordinates.
#[derive(Clone, Copy, Debug)]
pub struct WindowInfo {
    pub number: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WindowInfo {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// Finds the on-screen window owned by `pid` that contains the given point.
///
/// The point is in global screen points with a top-left origin. When no owned window's bounds
/// contain the point, this falls back to the frontmost normal window owned by `pid`.
pub fn window_at(pid: libc::pid_t, x: f64, y: f64) -> Option<WindowInfo> {
    // The returned list is ordered front-to-back.
    let info = window_list()?;

    // Read the window-info keys once; accessing the framework statics is unsafe.
    let owner_pid_key = unsafe { kCGWindowOwnerPID };
    let layer_key = unsafe { kCGWindowLayer };
    let number_key = unsafe { kCGWindowNumber };
    let bounds_key = unsafe { kCGWindowBounds };

    let mut fallback: Option<WindowInfo> = None;
    for dict in info.iter() {
        // Only consider windows owned by the target process.
        if dict_i64(&dict, owner_pid_key) != Some(pid as i64) {
            continue;
        }

        // Only consider normal (layer 0) windows; menus and similar live on other layers.
        if dict_i64(&dict, layer_key) != Some(0) {
            continue;
        }

        let (Some(number), Some((bx, by, bw, bh))) = (
            dict_i64(&dict, number_key),
            dict_bounds(&dict, bounds_key).and_then(|b| read_bounds(&b)),
        ) else {
            continue;
        };

        let window = WindowInfo {
            number,
            x: bx,
            y: by,
            width: bw,
            height: bh,
        };

        // Remember the frontmost owned window in case nothing contains the point.
        if fallback.is_none() {
            fallback = Some(window);
        }

        if window.contains(x, y) {
            return Some(window);
        }
    }

    fallback
}

/// Finds the on-screen window with the given `window_id`, returning its number and bounds in
/// global screen points (top-left origin). Used to resolve a `Target::Window` to concrete
/// geometry for window-local coordinate remapping and window-scoped screenshot scaling.
pub fn window_by_id(window_id: u32) -> Option<WindowInfo> {
    let info = window_list()?;

    let number_key = unsafe { kCGWindowNumber };
    let bounds_key = unsafe { kCGWindowBounds };

    for dict in info.iter() {
        if dict_i64(&dict, number_key) != Some(window_id as i64) {
            continue;
        }
        let (bx, by, bw, bh) = dict_bounds(&dict, bounds_key).and_then(|b| read_bounds(&b))?;
        return Some(WindowInfo {
            number: window_id as i64,
            x: bx,
            y: by,
            width: bw,
            height: bh,
        });
    }
    None
}

/// Returns the `(owner_pid, window_number)` of the frontmost on-screen normal window, i.e. the
/// window that currently has input focus. Used to deactivate the previous window when moving
/// focus to a target without raising it.
pub fn frontmost_window() -> Option<(libc::pid_t, i64)> {
    let info = window_list()?;

    let owner_pid_key = unsafe { kCGWindowOwnerPID };
    let layer_key = unsafe { kCGWindowLayer };
    let number_key = unsafe { kCGWindowNumber };

    // The list is front-to-back; the first normal (layer 0) window is the focused one.
    for dict in info.iter() {
        if dict_i64(&dict, layer_key) != Some(0) {
            continue;
        }
        if let (Some(pid), Some(number)) =
            (dict_i64(&dict, owner_pid_key), dict_i64(&dict, number_key))
        {
            return Some((pid as libc::pid_t, number));
        }
    }
    None
}

/// A description of an on-screen window, for diagnostics and enumeration.
pub struct WindowDescription {
    pub number: i64,
    pub owner_pid: i64,
    pub owner_name: Option<String>,
    pub title: Option<String>,
    pub layer: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Enumerates on-screen windows as crate-level [`crate::WindowInfo`] records, so the agent can
/// pick a window to target. Ordered front-to-back, excluding desktop elements.
pub fn enumerate_windows() -> Vec<crate::WindowInfo> {
    list_windows()
        .into_iter()
        .map(|w| crate::WindowInfo {
            window_id: w.number.max(0) as u32,
            pid: w.owner_pid as i32,
            app_name: w.owner_name.unwrap_or_default(),
            title: w.title.unwrap_or_default(),
            layer: w.layer as i32,
        })
        .collect()
}

/// Lists on-screen windows (excluding desktop elements), front-to-back, for diagnostics.
pub fn list_windows() -> Vec<WindowDescription> {
    let Some(info) = window_list() else {
        return Vec::new();
    };

    let owner_pid_key = unsafe { kCGWindowOwnerPID };
    let owner_name_key = unsafe { kCGWindowOwnerName };
    let name_key = unsafe { kCGWindowName };
    let layer_key = unsafe { kCGWindowLayer };
    let number_key = unsafe { kCGWindowNumber };
    let bounds_key = unsafe { kCGWindowBounds };

    let mut windows = Vec::new();
    for dict in info.iter() {
        let (Some(number), Some(owner_pid)) =
            (dict_i64(&dict, number_key), dict_i64(&dict, owner_pid_key))
        else {
            continue;
        };
        let (bx, by, bw, bh) = dict_bounds(&dict, bounds_key)
            .and_then(|b| read_bounds(&b))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));

        windows.push(WindowDescription {
            number,
            owner_pid,
            owner_name: dict_string(&dict, owner_name_key),
            // The window title requires the Screen Recording permission to be readable; it is
            // often empty otherwise.
            title: dict_string(&dict, name_key),
            layer: dict_i64(&dict, layer_key).unwrap_or(0),
            x: bx,
            y: by,
            width: bw,
            height: bh,
        });
    }
    windows
}

/// Returns the on-screen window list with its documented key and value types.
fn window_list() -> Option<CFRetained<CFArray<WindowDictionary>>> {
    let option =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let info = CGWindowListCopyWindowInfo(option, kCGNullWindowID)?;

    // SAFETY: Core Graphics documents the result as an array of dictionaries with CFString
    // keys and heterogeneous CFType values.
    Some(unsafe { CFRetained::cast_unchecked(info) })
}

/// Reads a string value from a window dictionary.
fn dict_string(dict: &WindowDictionary, key: &CFString) -> Option<String> {
    Some(dict.get(key)?.downcast::<CFString>().ok()?.to_string())
}

/// Reads an integer value from a window dictionary.
fn dict_i64(dict: &WindowDictionary, key: &CFString) -> Option<i64> {
    dict.get(key)?.downcast::<CFNumber>().ok()?.as_i64()
}

/// Reads a bounds dictionary from a window dictionary.
fn dict_bounds(dict: &WindowDictionary, key: &CFString) -> Option<CFRetained<BoundsDictionary>> {
    let bounds = dict.get(key)?.downcast::<CFDictionary>().ok()?;

    // SAFETY: Core Graphics documents kCGWindowBounds as a dictionary with CFString keys and
    // CFNumber values.
    Some(unsafe { CFRetained::cast_unchecked(bounds) })
}

/// Reads the `X`, `Y`, `Width`, `Height` numbers from a `kCGWindowBounds` dictionary.
fn read_bounds(bounds: &BoundsDictionary) -> Option<(f64, f64, f64, f64)> {
    let get = |name: &'static str| -> Option<f64> {
        let key = CFString::from_static_str(name);
        bounds.get(&key)?.as_f64()
    };

    Some((get("X")?, get("Y")?, get("Width")?, get("Height")?))
}
