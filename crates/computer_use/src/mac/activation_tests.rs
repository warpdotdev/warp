use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::*;

fn untapped_session(owner: Option<&str>) -> ActiveSession {
    ActiveSession {
        suppress: Arc::new(AtomicBool::new(true)),
        stop: Arc::new(AtomicBool::new(false)),
        thread: None,
        has_taps: false,
        previous: None,
        owner: owner.map(str::to_owned),
    }
}

/// Regression guard for the background-computer-use focus-stuck bug: ending a session must remove
/// that owner's `(pid, window_number)` entries from the registry (so the next `ensure_activated`
/// re-primes rather than hitting the "already activated" no-op), while leaving other owners'
/// concurrent sessions intact.
#[test]
fn end_sessions_for_owner_clears_only_that_owners_entries() {
    // Target our own process so any `ApplicationDeactivated` post is harmless; use distinct
    // window numbers per fake entry so keys don't collide.
    let pid = std::process::id() as libc::pid_t;
    let key_a = (pid, i64::MAX);
    let key_b = (pid, i64::MAX - 1);
    {
        let mut registry = registry().lock().unwrap();
        registry.insert(key_a, untapped_session(Some("conversation-a")));
        registry.insert(key_b, untapped_session(Some("conversation-b")));
    }

    end_sessions_for_owner("conversation-a");

    {
        let registry = registry().lock().unwrap();
        // Owner A's entry is gone so a restart re-primes; owner B's concurrent session survives.
        assert!(!registry.contains_key(&key_a));
        assert!(registry.contains_key(&key_b));
    }

    // Idempotent: ending an owner with no active session is a harmless no-op.
    end_sessions_for_owner("conversation-a");
    assert!(registry().lock().unwrap().contains_key(&key_b));

    // Clean up the surviving entry so the process-global registry doesn't leak across tests.
    end_sessions_for_owner("conversation-b");
    assert!(!registry().lock().unwrap().contains_key(&key_b));
}
