//! Experimental bindings to private SkyLight event APIs used for background input.
//!
//! Two pieces are needed to deliver mouse events to a process without stealing the user's
//! cursor or focus, and have them accepted by Chromium/Electron renderers:
//!
//! - `SLEventPostToPid` posts an event to a specific process through an auth-signed SkyLight
//!   channel that renderers trust (`CGEventPostToPid` events are dropped by Chromium).
//! - `SLPSPostEventRecordTo` (the yabai "focus-without-raise" pattern) flips the target into
//!   the AppKit-active input state without raising its window, so the event is actually
//!   routed to the focused window/renderer.
//!
//! All of these are undocumented, resolved at runtime, and degrade gracefully when missing.
//! The synthetic event records below carry magic byte offsets reverse-engineered by yabai;
//! they are also OS-version-fragile (a documented `SIGABRT` exists on some macOS 14 builds),
//! so this should be OS-gated before shipping.

use std::ffi::{CStr, c_void};
use std::sync::OnceLock;

use objc2_core_graphics::CGEvent;

/// macOS `ProcessSerialNumber`, needed by the `SLPS*`/`GetProcessForPID` Carbon-era APIs.
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

type SLEventPostToPidFn = unsafe extern "C" fn(libc::pid_t, *mut c_void);
type SLPSPostEventRecordToFn = unsafe extern "C" fn(*mut ProcessSerialNumber, *mut u8) -> i32;
type GetProcessForPidFn = unsafe extern "C" fn(libc::pid_t, *mut ProcessSerialNumber) -> i32;

fn debug_enabled() -> bool {
    std::env::var_os("COMPUTER_USE_DEBUG").is_some()
}

/// Resolves a symbol from any loaded image via `dlsym(RTLD_DEFAULT, ...)`.
///
/// # Safety
/// `F` must be a function-pointer type matching the symbol's real ABI.
unsafe fn resolve_symbol<F: Copy>(name: &CStr) -> Option<F> {
    // The macOS value of `RTLD_DEFAULT`, used to search all loaded images.
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    let sym = unsafe { libc::dlsym(RTLD_DEFAULT, name.as_ptr()) };
    if sym.is_null() {
        None
    } else {
        // SAFETY: function pointers are pointer-sized; caller guarantees the ABI of `F`.
        Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&sym) })
    }
}

/// Resolves `SLEventPostToPid` once, ensuring SkyLight is loaded first.
fn sl_event_post_to_pid() -> Option<SLEventPostToPidFn> {
    static RESOLVED: OnceLock<Option<SLEventPostToPidFn>> = OnceLock::new();
    *RESOLVED.get_or_init(|| unsafe {
        // Ensure SkyLight is loaded (it usually already is in an AppKit process).
        const RTLD_LAZY: i32 = 0x1;
        libc::dlopen(
            c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
            RTLD_LAZY,
        );
        let resolved = resolve_symbol::<SLEventPostToPidFn>(c"SLEventPostToPid");
        if debug_enabled() {
            eprintln!(
                "[computer_use] SLEventPostToPid resolved={}",
                resolved.is_some()
            );
        }
        if resolved.is_none() {
            log::warn!(
                "SLEventPostToPid could not be resolved; falling back to CGEventPostToPid \
                 (Chromium/Electron clicks may be dropped)."
            );
        }
        resolved
    })
}

fn slps_post_event_record_to() -> Option<SLPSPostEventRecordToFn> {
    static RESOLVED: OnceLock<Option<SLPSPostEventRecordToFn>> = OnceLock::new();
    *RESOLVED.get_or_init(|| unsafe { resolve_symbol(c"SLPSPostEventRecordTo") })
}

fn get_process_for_pid() -> Option<GetProcessForPidFn> {
    static RESOLVED: OnceLock<Option<GetProcessForPidFn>> = OnceLock::new();
    *RESOLVED.get_or_init(|| unsafe { resolve_symbol(c"GetProcessForPID") })
}

/// Posts an event to a specific process, preferring SkyLight's `SLEventPostToPid` (accepted by
/// Chromium/Electron renderers) and falling back to `CGEventPostToPid`.
pub fn post_event_to_pid(pid: libc::pid_t, event: &CGEvent) {
    match sl_event_post_to_pid() {
        Some(post) => {
            let event_ptr = event as *const CGEvent as *mut c_void;
            unsafe { post(pid, event_ptr) };
        }
        None => CGEvent::post_to_pid(pid, Some(event)),
    }
}

/// Resolves the `ProcessSerialNumber` for a pid, or `None` if the lookup is unavailable/fails.
fn psn_for_pid(pid: libc::pid_t) -> Option<ProcessSerialNumber> {
    let get_process = get_process_for_pid()?;
    let mut psn = ProcessSerialNumber {
        high_long_of_psn: 0,
        low_long_of_psn: 0,
    };
    // SAFETY: `psn` is a valid out-pointer.
    let status = unsafe { get_process(pid, &mut psn) };
    (status == 0).then_some(psn)
}

/// Posts the "make key window" event record (yabai pattern) to a process for a window.
///
/// # Safety
/// `post` must be the real `SLPSPostEventRecordTo`.
unsafe fn post_make_key_record(
    post: SLPSPostEventRecordToFn,
    psn: &mut ProcessSerialNumber,
    window_id: u32,
) {
    let mut bytes = [0u8; 0xf8];
    bytes[0x04] = 0xf8;
    bytes[0x3a] = 0x10;
    bytes[0x3c..0x40].copy_from_slice(&window_id.to_ne_bytes());
    for b in &mut bytes[0x20..0x30] {
        *b = 0xff;
    }
    unsafe {
        bytes[0x08] = 0x01;
        post(psn, bytes.as_mut_ptr());
        bytes[0x08] = 0x02;
        post(psn, bytes.as_mut_ptr());
    }
}

/// Posts a window "focus" event record (yabai pattern). `activate` selects the got-focus
/// (`0x01`) vs lost-focus (`0x02`) variant.
///
/// # Safety
/// `post` must be the real `SLPSPostEventRecordTo`.
unsafe fn post_focus_record(
    post: SLPSPostEventRecordToFn,
    psn: &mut ProcessSerialNumber,
    window_id: u32,
    activate: bool,
) {
    let mut bytes = [0u8; 0xf8];
    bytes[0x04] = 0xf8;
    bytes[0x08] = 0x0d;
    bytes[0x8a] = if activate { 0x01 } else { 0x02 };
    bytes[0x3c..0x40].copy_from_slice(&window_id.to_ne_bytes());
    unsafe {
        post(psn, bytes.as_mut_ptr());
    }
}

/// Makes `target_window_id` (owned by `target_pid`) the key window for input routing *without
/// raising it*, using the yabai focus-without-raise pattern. Optionally deactivates the
/// `previous` (currently-frontmost) window first, which is required for input focus to
/// actually transfer to a different application (notably Chromium/Electron).
///
/// Returns `false` when the private symbols are unavailable.
pub fn focus_window_without_raise(
    target_pid: libc::pid_t,
    target_window_id: i64,
    previous: Option<(libc::pid_t, i64)>,
) -> bool {
    let Some(post) = slps_post_event_record_to() else {
        if debug_enabled() {
            eprintln!("[computer_use] focus-without-raise unavailable (SLPSPostEventRecordTo)");
        }
        return false;
    };
    let Some(mut target_psn) = psn_for_pid(target_pid) else {
        if debug_enabled() {
            eprintln!("[computer_use] focus-without-raise: no PSN for target pid {target_pid}");
        }
        return false;
    };

    // Deactivate the previously-focused window (in a different app) so input focus can move.
    if let Some((prev_pid, prev_window_id)) = previous
        && prev_pid != target_pid
        && let Some(mut prev_psn) = psn_for_pid(prev_pid)
    {
        unsafe { post_focus_record(post, &mut prev_psn, prev_window_id as u32, false) };
    }

    unsafe {
        // Activate (got-focus) the target window, then make it the key window.
        post_focus_record(post, &mut target_psn, target_window_id as u32, true);
        post_make_key_record(post, &mut target_psn, target_window_id as u32);
    }

    true
}
