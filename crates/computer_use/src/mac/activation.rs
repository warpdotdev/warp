//! Background window activation without private SkyLight APIs.
//!
//! Background input (delivering clicks/keys to a window that is not frontmost, without moving
//! the real cursor) requires the target window to believe it is in the AppKit-active input
//! state. We achieve that the way a real first click on an inactive window would, but without
//! the visual side effect of bringing the app to the front:
//!
//! 1. Install per-process [`CGEvent`] event taps (`CGEventTapCreateForPid`) on both the
//!    previously-frontmost app and the target app. The taps run on a dedicated run-loop thread
//!    and drop the focus-change messages that macOS would otherwise deliver to the previous app
//!    to switch the user's frontmost application.
//! 2. Send an `appKitDefined` activation primer event (an `NSEvent` of type `AppKitDefined`,
//!    subtype `ApplicationActivated`) directly to the target process via `CGEventPostToPid`.
//! 3. Send a single "primer" left click to the exact center of the window. While the window is
//!    inactive, macOS routes this first click through its activation flow instead of firing a
//!    UI action, so it activates the window without otherwise affecting the app.
//!
//! State is tracked in a process-global registry keyed by `(pid, window)` rather than on the
//! per-action [`super::Actor`] (which is recreated for every computer-use turn). This lets a
//! single computer-use session activate a window once and reuse that activation across later
//! turns without re-sending the disruptive center click, and lets concurrent computer-use
//! sessions targeting different windows coexist while serializing each activation handshake
//! through the registry lock.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventSubtype, NSEventType};
use objc2_core_foundation::{CFMachPort, CFRunLoop, CGPoint, kCFRunLoopDefaultMode};
use objc2_core_graphics::{
    CGEvent, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType, CGMouseButton,
};

use super::window::{self, WindowInfo};

/// Delay after the `appKitDefined` activation primer, giving AppKit time to process it before
/// the center primer click arrives.
const APPKIT_PRIMER_DELAY: Duration = Duration::from_millis(20);
/// Delay between the center primer's mouse-down and mouse-up.
const PRIMER_CLICK_HOLD: Duration = Duration::from_millis(30);
/// Delay after the center primer click settles.
const PRIMER_CLICK_SETTLE: Duration = Duration::from_millis(20);
/// How long each run-loop service iteration blocks before re-checking the stop flag.
const RUN_LOOP_SERVICE_INTERVAL: f64 = 0.1;

/// Ensures the window described by `info` (owned by `target_pid`) is in the AppKit-active input
/// state so background events are accepted, without raising it or moving the cursor.
///
/// The first call for a given `(pid, window)` installs focus-suppression taps and sends the
/// activation primers (including the center primer click). Subsequent calls for the same window
/// are a no-op, so the disruptive center click is never re-sent across turns. If the target app
/// is already frontmost, no activation is needed and this is a no-op.
pub fn ensure_activated(target_pid: libc::pid_t, info: &WindowInfo) {
    let window_number = info.number;
    if window_number <= 0 {
        return;
    }
    let key = (target_pid, window_number);

    let mut registry = registry().lock().unwrap();
    if registry.contains_key(&key) {
        // Already activated this window; do not re-activate (a second center click would land
        // as a real click rather than being absorbed by the activation flow).
        return;
    }

    // The window that currently owns input focus, so we can both protect it from deactivation
    // and detect when the target app is already frontmost.
    let previous = window::frontmost_window();
    if previous.map(|(pid, _)| pid) == Some(target_pid) {
        // The target app is already frontmost: events route to it normally and no focus would
        // be stolen, so there is nothing to activate or suppress.
        return;
    }

    let suppress = Arc::new(AtomicBool::new(true));
    let stop = Arc::new(AtomicBool::new(false));

    // Install focus-suppression taps before sending the activation click, so the focus-change
    // messages it triggers are intercepted. Skip the taps if there is no distinct previous app.
    let thread = match previous {
        Some((previous_pid, _)) if previous_pid != target_pid => {
            spawn_tap_thread(previous_pid, target_pid, suppress.clone(), stop.clone())
        }
        _ => None,
    };
    let has_taps = thread.is_some();

    // Activate: AppKit activation primer, then the center-of-window primer click.
    post_appkit_activation(
        target_pid,
        window_number,
        NSEventSubtype::ApplicationActivated.0,
    );
    thread::sleep(APPKIT_PRIMER_DELAY);
    post_center_primer(target_pid, info);

    registry.insert(
        key,
        ActiveSession {
            suppress,
            stop,
            thread,
            has_taps,
        },
    );
}

/// The process-global registry of activated windows, keyed by `(pid, window_number)`.
fn registry() -> &'static Mutex<HashMap<(libc::pid_t, i64), ActiveSession>> {
    static REGISTRY: OnceLock<Mutex<HashMap<(libc::pid_t, i64), ActiveSession>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Tracks an activated window: the focus-suppression tap thread (if any) and the flags used to
/// drive and tear it down.
struct ActiveSession {
    /// While set, the tap callback drops focus-change messages headed to the previous app.
    suppress: Arc<AtomicBool>,
    /// Signals the tap thread's run loop to exit.
    stop: Arc<AtomicBool>,
    /// The run-loop thread servicing the taps, joined on teardown.
    thread: Option<JoinHandle<()>>,
    has_taps: bool,
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        if self.has_taps {
            self.suppress.store(false, Ordering::SeqCst);
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Per-tap data handed to the C event-tap callback via its `user_info` pointer.
struct TapContext {
    /// Shared with [`ActiveSession`]; gates focus-message suppression.
    suppress: Arc<AtomicBool>,
    /// Whether this tap is attached to the previously-frontmost app (whose focus messages we
    /// drop) versus the target app (which we always let through).
    is_previous: bool,
}

/// Spawns the run-loop thread that installs and services the focus-suppression taps for
/// `previous_pid` and `target_pid`. Returns the thread handle once both taps are installed, or
/// `None` if tap creation failed (e.g. missing permission).
fn spawn_tap_thread(
    previous_pid: libc::pid_t,
    target_pid: libc::pid_t,
    suppress: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("cu-bg-activation".to_string())
        .spawn(move || run_tap_loop(previous_pid, target_pid, suppress, stop, ready_tx))
        .ok()?;

    match ready_rx.recv() {
        Ok(true) => Some(handle),
        _ => {
            // Tap installation failed (or the thread died); reclaim it and report no taps.
            let _ = handle.join();
            None
        }
    }
}

/// Body of the tap thread: creates the taps on this thread's run loop, signals readiness, then
/// services the run loop until asked to stop, tearing the taps down on exit.
fn run_tap_loop(
    previous_pid: libc::pid_t,
    target_pid: libc::pid_t,
    suppress: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<bool>,
) {
    let Some(run_loop) = CFRunLoop::current() else {
        let _ = ready_tx.send(false);
        return;
    };
    let mode = unsafe { kCFRunLoopDefaultMode };

    // Keep the taps, their run-loop sources, and the heap-allocated contexts alive for as long
    // as the run loop runs. The contexts are reclaimed (and dropped) after the loop exits.
    let mut taps = Vec::new();
    let mut sources = Vec::new();
    let mut contexts: Vec<*mut TapContext> = Vec::new();

    for (pid, is_previous) in [(previous_pid, true), (target_pid, false)] {
        let context = Box::into_raw(Box::new(TapContext {
            suppress: suppress.clone(),
            is_previous,
        }));
        // SAFETY: `tap_callback` matches the `CGEventTapCallBack` ABI and `context` is a valid,
        // owned `TapContext` pointer that outlives the tap (it is dropped only after the run
        // loop stops, below).
        let tap = unsafe {
            CGEvent::tap_create_for_pid(
                pid,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                u64::MAX,
                Some(tap_callback),
                context as *mut c_void,
            )
        };
        let Some(tap) = tap else {
            // Reclaim the context this tap would have owned.
            drop(unsafe { Box::from_raw(context) });
            continue;
        };
        let Some(source) = CFMachPort::new_run_loop_source(None, Some(&tap), 0) else {
            tap.invalidate();
            drop(unsafe { Box::from_raw(context) });
            continue;
        };
        run_loop.add_source(Some(&source), mode);
        CGEvent::tap_enable(&tap, true);
        taps.push(tap);
        sources.push(source);
        contexts.push(context);
    }

    let installed = !taps.is_empty();
    let _ = ready_tx.send(installed);
    if !installed {
        return;
    }

    // Service the taps until teardown. `run_in_mode` blocks up to the interval (sleeping when
    // idle), so re-checking the flag this way costs only a brief teardown latency.
    while !stop.load(Ordering::SeqCst) {
        CFRunLoop::run_in_mode(mode, RUN_LOOP_SERVICE_INTERVAL, false);
    }

    for tap in &taps {
        tap.invalidate();
    }
    drop(sources);
    drop(taps);
    for context in contexts {
        drop(unsafe { Box::from_raw(context) });
    }
}

/// The C event-tap callback. Returns the event to pass it through, or null to drop it.
///
/// Focus-change messages do not have a stable public `CGEventType` across macOS versions, so
/// they are identified by their raw values (13, 19, 20). When suppression is active we drop
/// those headed to the previous app (keeping it from being deactivated, i.e. keeping it the
/// user's frontmost app) while letting the target app's activation through.
unsafe extern "C-unwind" fn tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    let event_ptr = event.as_ptr();
    let Some(context) = (unsafe { (user_info as *const TapContext).as_ref() }) else {
        return event_ptr;
    };
    let is_focus_message = matches!(event_type.0, 13 | 19 | 20);
    if is_focus_message && context.is_previous && context.suppress.load(Ordering::SeqCst) {
        return std::ptr::null_mut();
    }
    event_ptr
}

/// Sends an `appKitDefined` application-activation event to the target process. `subtype`
/// selects activate ([`NSEventSubtype::ApplicationActivated`]) vs. deactivate
/// ([`NSEventSubtype::ApplicationDeactivated`]).
fn post_appkit_activation(target_pid: libc::pid_t, window_number: i64, subtype: i16) {
    let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::AppKitDefined,
        CGPoint { x: 0.0, y: 0.0 },
        NSEventModifierFlags::empty(),
        0.0,
        window_number as isize,
        None,
        subtype,
        0,
        0,
    ) else {
        return;
    };
    let Some(cg_event) = event.CGEvent() else {
        return;
    };
    // Associate the event with the target window so AppKit applies the activation to it.
    super::mouse::set_window_addressing_fields(&cg_event, window_number);
    CGEvent::post_to_pid(target_pid, Some(&cg_event));
}

/// Sends a single left click to the exact center of the window. On an inactive window this is
/// absorbed by the activation flow rather than firing a UI action; the center avoids the
/// title-bar traffic-light controls (which respond even when inactive).
fn post_center_primer(target_pid: libc::pid_t, info: &WindowInfo) {
    let center = CGPoint {
        x: info.x + info.width / 2.0,
        y: info.y + info.height / 2.0,
    };
    super::mouse::post_window_mouse_event(
        target_pid,
        info,
        CGEventType::LeftMouseDown,
        CGMouseButton::Left,
        center,
        1,
        1.0,
    );
    thread::sleep(PRIMER_CLICK_HOLD);
    super::mouse::post_window_mouse_event(
        target_pid,
        info,
        CGEventType::LeftMouseUp,
        CGMouseButton::Left,
        center,
        1,
        0.0,
    );
    thread::sleep(PRIMER_CLICK_SETTLE);
}
