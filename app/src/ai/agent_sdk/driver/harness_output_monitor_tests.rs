use std::time::Duration;

use super::{outputs_stalled, pattern_for_match, STALL_CONFIRMATION_BUDGET, STALL_POLL_INTERVAL};

#[test]
fn pattern_for_match_returns_originating_needle() {
    let patterns: &[&'static str] = &["credit balance is too low", "invalid_api_key"];
    // The DFA gives us the matched substring (e.g. lifted from the grid as
    // mixed case); we must map back to the original `'static` needle.
    let resolved = pattern_for_match("Credit Balance Is Too Low", patterns);
    assert_eq!(resolved, Some("credit balance is too low"));
}

#[test]
fn pattern_for_match_is_case_insensitive() {
    let patterns: &[&'static str] = &["INVALID_API_KEY"];
    let resolved = pattern_for_match("invalid_api_key", patterns);
    assert_eq!(resolved, Some("INVALID_API_KEY"));
}

#[test]
fn pattern_for_match_returns_none_when_no_match() {
    let patterns: &[&'static str] = &["needle a", "needle b"];
    assert!(pattern_for_match("entirely different text", patterns).is_none());
}

#[test]
fn pattern_for_match_picks_first_matching_needle() {
    // When two needles lowercase-equal the same matched text (degenerate
    // case), we deterministically return the first one in the slice.
    let patterns: &[&'static str] = &["Foo", "foo"];
    let resolved = pattern_for_match("FOO", patterns);
    assert_eq!(resolved, Some("Foo"));
}

// --- outputs_stalled / stall confirmation ---

#[test]
fn outputs_stalled_returns_true_when_inputs_equal() {
    let snapshot = "Error: credit balance is too low\n";
    assert!(outputs_stalled(Some(snapshot), Some(snapshot)));
}

#[test]
fn outputs_stalled_returns_false_when_inputs_differ_by_any_byte() {
    // Spinner case: a single character differs between frames. The
    // confirmation loop must treat this as "still moving" and not
    // mistakenly declare the harness stalled.
    let before = "Retrying \u{2807}";
    let after = "Retrying \u{2826}";
    assert!(!outputs_stalled(Some(before), Some(after)));
}

#[test]
fn outputs_stalled_returns_false_when_either_input_is_none() {
    // A failed snapshot fetch must default to "not confirmed" rather than
    // killing the harness on a transient lookup error.
    assert!(!outputs_stalled(None, Some("data")));
    assert!(!outputs_stalled(Some("data"), None));
    assert!(!outputs_stalled(None, None));
}

#[test]
fn stall_confirmation_budget_matches_six_poll_intervals() {
    // The loop guarantees up to BUDGET/INTERVAL iterations. Pin the ratio
    // so a careless tweak to either constant can't accidentally turn the
    // confirmation into a single comparison or extend it indefinitely.
    assert_eq!(
        STALL_CONFIRMATION_BUDGET.as_secs() / STALL_POLL_INTERVAL.as_secs(),
        6
    );
    // And that they're cleanly divisible (no leftover sub-interval window).
    assert_eq!(
        STALL_CONFIRMATION_BUDGET.as_secs() % STALL_POLL_INTERVAL.as_secs(),
        0
    );
    // Sanity: STALL_POLL_INTERVAL must be non-zero or the loop would spin.
    assert!(STALL_POLL_INTERVAL > Duration::ZERO);
}
