//! Linux-specific app level functionality for use with `winit`.
//!
//! For more information about X11 extensions and request codes/opcodes,
//! see: https://www.x.org/wiki/Development/Documentation/Protocol/OpCodes.

use std::sync::{Arc, Mutex};

use lazy_static::lazy_static;
use wgpu::rwh::{HasDisplayHandle, RawDisplayHandle};
use winit::event_loop::EventLoop;
use x11rb::protocol::xproto::ConnectionExt as _;

lazy_static! {
    static ref ENCOUNTERED_BAD_MATCH_FROM_DRI3_FENCE_FROM_FD: Arc<Mutex<bool>> = Default::default();
}

/// Returns whether a `BadMatch` was returned from a `DRI3FenceFromFd` request.
/// NOTE calling this function resets internal state. Subsequent calls to this function will return
/// false until a new `BadMatch` error is encountered for the aforementioned request.
pub fn take_encountered_bad_match_from_dri3_fence_from_fd() -> bool {
    let Ok(mut guard) = ENCOUNTERED_BAD_MATCH_FROM_DRI3_FENCE_FROM_FD.lock() else {
        return false;
    };

    std::mem::take(&mut guard)
}

/// Registers an xlib error hook with winit, if needed.
pub fn maybe_register_xlib_error_hook<T>(event_loop: &EventLoop<T>) {
    if !is_x11(event_loop) {
        return;
    }

    let extension_info_map = get_x11_extension_info_map();

    // Register a callback function with winit that we can use to
    // observe and consume error events from the Xlib event loop.
    // This returns a boolean indicating whether or not it was
    // "handled" by this error hook.  If `true` is returned,
    // winit will ignore the error.
    winit::platform::x11::register_xlib_error_hook(Box::new(move |_, error| {
        static GRAB_KEY_REQUEST_CODE: u8 = 33;
        static PRESENT_PIXMAP_MINOR_OPCODE: u8 = 1;

        /// Minor opcode for the `DRI3FenceFromFD` request within the DRI3 extension.
        /// See https://cgit.freedesktop.org/xorg/proto/dri3proto/tree/dri3proto.txt.
        static DRI3_FENCE_FROM_FD_MINOR_OPCODE: u8 = 4;

        let Some(error) = std::ptr::NonNull::new(error as *mut x11_dl::xlib::XErrorEvent) else {
            return false;
        };
        let error = unsafe { error.as_ref() };

        // Ignore errors due to global-hotkey attempting to register a
        // hotkey that's already been registered.
        if error.error_code == x11_dl::xlib::BadAccess
            && error.request_code == GRAB_KEY_REQUEST_CODE
        {
            return true;
        }

        let Some(extension_info) = extension_info_map.get(&error.request_code) else {
            // If we can't get information about the extension, let winit
            // handle it.  It will log the error, so we don't need to.
            return false;
        };

        // If there's a BadWindow error from a PresentPixmap
        // request, ignore it - this is a known bug in Mesa.
        if error.error_code == x11_dl::xlib::BadWindow
            && extension_info.name == "Present"
            && error.minor_code == PRESENT_PIXMAP_MINOR_OPCODE
        {
            log::warn!("Ignoring BadWindow error from PresentPixmap request");
            return true;
        }

        // Specifically handle a `BadMatch` from a `DRI3_FENCE_FROM_FD` request. From error
        // reporting, we only seem to get this error when a user has the Performance PRIME profile
        // enabled (indicating to NVIDIA Optimus that the NVIDIA GPU should always be used).
        if error.error_code == x11_dl::xlib::BadMatch
            && extension_info.name == "DRI3"
            && error.minor_code == DRI3_FENCE_FROM_FD_MINOR_OPCODE
        {
            log::warn!("Ignoring a BadMatch from a DRI3FenceFromFD request. The NVIDIA Performance PRIME profile is likely enabled.");
            *ENCOUNTERED_BAD_MATCH_FROM_DRI3_FENCE_FROM_FD
                .lock()
                .unwrap() = true;
            return true;
        }

        // For other errors from requests defined in extensions, log some
        // relevant extension information, then let winit decide what to do
        // with it. winit will log an error if we don't handle it, hence only logging a warning
        // here.
        log::warn!(
            "Detected X11 error in {} extension (major opcode: {}; first error: {})",
            extension_info.name,
            error.request_code,
            extension_info.first_error,
        );

        if *ENCOUNTERED_BAD_MATCH_FROM_DRI3_FENCE_FROM_FD
            .lock()
            .expect("Mutex should not be poisoned")
            && extension_info.name == "Present"
        {
            log::warn!("Ignoring an error from the PRESENT extension after catching a BadMatch from a DRI3FenceFromFD request. Minor opcode: {}; Error code: {}",
                error.minor_code,
                error.error_code);
            return true;
        }

        false
    }));
}

/// Queries the X11 server to get information about which extensions are
/// available and metadata about them.
fn get_x11_extension_info_map() -> std::collections::HashMap<u8, X11ExtensionInfo> {
    let mut extension_map = Default::default();
    let Ok((xcb, _)) = x11rb::rust_connection::RustConnection::connect(None) else {
        return extension_map;
    };

    let Ok(cookie) = xcb.list_extensions() else {
        return extension_map;
    };

    let Ok(extensions) = cookie.reply() else {
        return extension_map;
    };

    extensions.names.iter().for_each(|name| {
        if let Ok(cookie) = xcb.query_extension(&name.name) {
            if let Ok(result) = cookie.reply() {
                if let Ok(name) = String::from_utf8(name.name.clone()) {
                    extension_map.insert(
                        result.major_opcode,
                        X11ExtensionInfo {
                            name,
                            first_error: result.first_error,
                        },
                    );
                }
            }
        }
    });

    extension_map
}

/// A collection of information about an X11 extension.
struct X11ExtensionInfo {
    /// The name of the extension.
    name: String,

    /// The ID offset applied to errors defined in this extension.
    first_error: u8,
}

/// Returns whether or not the provided event loop is using X11 as the
/// underlying platform implementation.
fn is_x11<T>(event_loop: &EventLoop<T>) -> bool {
    matches!(
        event_loop
            .owned_display_handle()
            .display_handle()
            .map(|dh| dh.as_raw()),
        Ok(RawDisplayHandle::Xlib(_)) | Ok(RawDisplayHandle::Xcb(_))
    )
}
