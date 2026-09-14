use super::*;

// Latches a process-global flag, so this is the only test in the crate that
// touches it. Under nextest each test runs in its own process; nothing else in
// this binary reads `is_session_ending`.
#[test]
fn note_session_ending_latches() {
    assert!(!is_session_ending());

    note_session_ending();
    assert!(is_session_ending());

    // Latched, so repeating it is a no-op rather than a toggle.
    note_session_ending();
    assert!(is_session_ending());
}
