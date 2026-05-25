//! Experimental helper for locating the on-screen window under a point for a given process.
//!
//! PID-targeted synthetic mouse events (`CGEventPostToPid`) bypass the WindowServer's
//! hit-testing, so they arrive at the target process without an associated window. To let
//! AppKit route them, we reconstruct the target window (its number and bounds) so the event
//! can be built as a window-targeted `NSEvent` with window-local coordinates.

use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::{CFNumber, CFNumberRef};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::window::{
    copy_window_info, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
};

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
    let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    // The returned list is ordered front-to-back.
    let info = copy_window_info(option, 0)?;

    // Read the window-info keys once; accessing the framework statics is unsafe.
    let owner_pid_key = unsafe { kCGWindowOwnerPID } as *const c_void;
    let layer_key = unsafe { kCGWindowLayer } as *const c_void;
    let number_key = unsafe { kCGWindowNumber } as *const c_void;
    let bounds_key = unsafe { kCGWindowBounds } as *const c_void;

    let mut fallback: Option<WindowInfo> = None;
    for entry in info.iter() {
        let dict: CFDictionary =
            unsafe { CFDictionary::wrap_under_get_rule(*entry as CFDictionaryRef) };

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
            dict_dict(&dict, bounds_key).and_then(|b| read_bounds(&b)),
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

/// Returns the `(owner_pid, window_number)` of the frontmost on-screen normal window, i.e. the
/// window that currently has input focus. Used to deactivate the previous window when moving
/// focus to a target without raising it.
pub fn frontmost_window() -> Option<(libc::pid_t, i64)> {
    let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let info = copy_window_info(option, 0)?;

    let owner_pid_key = unsafe { kCGWindowOwnerPID } as *const c_void;
    let layer_key = unsafe { kCGWindowLayer } as *const c_void;
    let number_key = unsafe { kCGWindowNumber } as *const c_void;

    // The list is front-to-back; the first normal (layer 0) window is the focused one.
    for entry in info.iter() {
        let dict: CFDictionary =
            unsafe { CFDictionary::wrap_under_get_rule(*entry as CFDictionaryRef) };
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

/// A description of an on-screen window, for diagnostics.
pub struct WindowDescription {
    pub number: i64,
    pub owner_pid: i64,
    pub owner_name: Option<String>,
    pub layer: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Lists on-screen windows (excluding desktop elements), front-to-back, for diagnostics.
pub fn list_windows() -> Vec<WindowDescription> {
    let option = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let Some(info) = copy_window_info(option, 0) else {
        return Vec::new();
    };

    let owner_pid_key = unsafe { kCGWindowOwnerPID } as *const c_void;
    let owner_name_key = unsafe { kCGWindowOwnerName } as *const c_void;
    let layer_key = unsafe { kCGWindowLayer } as *const c_void;
    let number_key = unsafe { kCGWindowNumber } as *const c_void;
    let bounds_key = unsafe { kCGWindowBounds } as *const c_void;

    let mut windows = Vec::new();
    for entry in info.iter() {
        let dict: CFDictionary =
            unsafe { CFDictionary::wrap_under_get_rule(*entry as CFDictionaryRef) };

        let (Some(number), Some(owner_pid)) =
            (dict_i64(&dict, number_key), dict_i64(&dict, owner_pid_key))
        else {
            continue;
        };
        let (bx, by, bw, bh) = dict_dict(&dict, bounds_key)
            .and_then(|b| read_bounds(&b))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));

        windows.push(WindowDescription {
            number,
            owner_pid,
            owner_name: dict_string(&dict, owner_name_key),
            layer: dict_i64(&dict, layer_key).unwrap_or(0),
            x: bx,
            y: by,
            width: bw,
            height: bh,
        });
    }
    windows
}

/// Reads a string value from a CF dictionary keyed by a `*const c_void` (CFString) key.
fn dict_string(dict: &CFDictionary, key: *const c_void) -> Option<String> {
    let value = dict.find(key)?;
    let string = unsafe { CFString::wrap_under_get_rule(*value as CFStringRef) };
    Some(string.to_string())
}

/// Reads an integer value from a CF dictionary keyed by a `*const c_void` (CFString) key.
fn dict_i64(dict: &CFDictionary, key: *const c_void) -> Option<i64> {
    let value = dict.find(key)?;
    let number = unsafe { CFNumber::wrap_under_get_rule(*value as CFNumberRef) };
    number.to_i64()
}

/// Reads a nested CF dictionary value from a CF dictionary.
fn dict_dict(dict: &CFDictionary, key: *const c_void) -> Option<CFDictionary> {
    let value = dict.find(key)?;
    Some(unsafe { CFDictionary::wrap_under_get_rule(*value as CFDictionaryRef) })
}

/// Reads the `X`, `Y`, `Width`, `Height` numbers from a `kCGWindowBounds` dictionary.
fn read_bounds(bounds: &CFDictionary) -> Option<(f64, f64, f64, f64)> {
    let get = |name: &'static str| -> Option<f64> {
        let key = CFString::from_static_string(name);
        let value = bounds.find(key.as_concrete_TypeRef() as *const c_void)?;
        let number = unsafe { CFNumber::wrap_under_get_rule(*value as CFNumberRef) };
        number.to_f64()
    };

    Some((get("X")?, get("Y")?, get("Width")?, get("Height")?))
}
