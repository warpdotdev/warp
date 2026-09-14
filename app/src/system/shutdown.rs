//! Detection of an OS-initiated end of the desktop session (shutdown, restart,
//! or logout).
//!
//! Warp treats a shell exiting as the user finishing with that pane: the pane
//! closes, the last pane closing takes its tab with it, and the tab close
//! rewrites the session snapshot without that tab. When Windows ends the
//! session it kills every console process first, so that chain runs once per
//! tab and empties the snapshot we were meant to restore from
//! (warpdotdev/warp#15269) - which is why a hard kill preserves a session but a
//! polite OS restart does not.
//!
//! Knowing that the OS, not the user, started the teardown lets the shell-exit
//! handler leave those panes alone, so the snapshot on disk still describes the
//! windows and tabs that were open when the restart began.

use std::sync::atomic::{AtomicBool, Ordering};

/// Latched by platforms that push the signal at us rather than answering a
/// query. See [`note_session_ending`].
static SESSION_ENDING: AtomicBool = AtomicBool::new(false);

/// Records that the OS asked the app to quit as part of ending the session.
///
/// macOS delivers this only as a system-initiated termination request, with
/// nothing to query afterwards, so we latch it. Approving that request commits
/// us to quitting, so latching cannot leave a still-running app stuck in
/// shutdown mode.
pub fn note_session_ending() {
    if !SESSION_ENDING.swap(true, Ordering::SeqCst) {
        log::info!("OS session is ending; keeping open tabs intact for session restore");
    }
}

/// Returns whether the OS is ending the session (shutdown, restart, or logout).
pub fn is_session_ending() -> bool {
    SESSION_ENDING.load(Ordering::SeqCst) || platform_is_session_ending()
}

#[cfg(windows)]
fn platform_is_session_ending() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};

    // `SM_SHUTTINGDOWN` is set for the whole session the moment Windows starts
    // tearing it down, which is before it starts killing console processes -
    // the only signal that is already true by the time our shells begin dying.
    // We query it each time rather than latching it because Windows clears it
    // again if something aborts the shutdown, and we want normal behaviour back
    // when that happens.
    unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
}

#[cfg(not(windows))]
fn platform_is_session_ending() -> bool {
    // Nothing to query: macOS pushes the signal through `note_session_ending`,
    // and Linux session managers expose no equivalent.
    false
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
