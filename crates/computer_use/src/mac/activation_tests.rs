use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::*;

/// Regression guard for the background-computer-use focus-stuck bug: `end_all_sessions` must
/// remove the `(pid, window_number)` entry from the registry. Without this, a stale entry makes
/// the next `ensure_activated` hit the "already activated" no-op, leaving the target window
/// activated (keyboard focus stuck on it) and breaking a restart targeting the same window.
#[test]
fn end_session_clears_registry_so_restart_reactivates() {
    // Target our own process so the teardown's `ApplicationDeactivated` post is harmless; use a
    // window number that won't collide with a real activated window. `previous: None` means no
    // re-activation post is attempted.
    let key = (std::process::id() as libc::pid_t, i64::MAX);
    {
        let mut registry = registry().lock().unwrap();
        registry.insert(
            key,
            ActiveSession {
                suppress: Arc::new(AtomicBool::new(true)),
                stop: Arc::new(AtomicBool::new(false)),
                thread: None,
                has_taps: false,
                previous: None,
            },
        );
        assert!(registry.contains_key(&key));
    }

    end_all_sessions();

    // The entry must be gone, so a subsequent `ensure_activated` runs the full fresh activation
    // path rather than returning early on the stale key.
    assert!(!registry().lock().unwrap().contains_key(&key));

    // Idempotent: calling again with an empty registry is a harmless no-op.
    end_all_sessions();
    assert!(registry().lock().unwrap().is_empty());
}
