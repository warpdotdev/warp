use std::time::Duration;

use super::{
    SHARED_SESSION_INACTIVITY_MAX_MINUTES, clamp_shared_session_end_minutes,
    clamp_shared_session_revoke_minutes, clamp_shared_session_warning_minutes,
    parse_shared_session_inactivity_minutes, shared_session_inactivity_display_text,
    shared_session_inactivity_minutes,
};

/// Zero means "disable this phase" and must be enterable, per APP-5313's zero semantics.
#[test]
fn parse_accepts_zero_as_disabled() {
    assert_eq!(parse_shared_session_inactivity_minutes("0"), Some(0));
}

#[test]
fn parse_rejects_non_numeric() {
    assert_eq!(parse_shared_session_inactivity_minutes("abc"), None);
    assert_eq!(parse_shared_session_inactivity_minutes(""), None);
    assert_eq!(parse_shared_session_inactivity_minutes("-5"), None);
    assert_eq!(parse_shared_session_inactivity_minutes("3.5"), None);
}

#[test]
fn parse_accepts_positive_values_within_bounds() {
    assert_eq!(parse_shared_session_inactivity_minutes("1"), Some(1));
    assert_eq!(parse_shared_session_inactivity_minutes("30"), Some(30));
    assert_eq!(parse_shared_session_inactivity_minutes(" 42 "), Some(42));
}

/// Regression test for review finding 3: `parse_shared_session_inactivity_minutes` must
/// reject values large enough that `minutes * 60` would overflow `u64`
/// (`307445734561825861` minutes wraps to 44 seconds in release and panics in debug).
#[test]
fn parse_rejects_values_that_would_overflow_when_converted_to_seconds() {
    assert_eq!(
        parse_shared_session_inactivity_minutes(
            &(SHARED_SESSION_INACTIVITY_MAX_MINUTES).to_string()
        ),
        Some(SHARED_SESSION_INACTIVITY_MAX_MINUTES),
        "the exact max boundary should still be accepted"
    );
    assert_eq!(
        parse_shared_session_inactivity_minutes(
            &(SHARED_SESSION_INACTIVITY_MAX_MINUTES + 1).to_string()
        ),
        None,
        "one past the max boundary must be rejected"
    );
    assert_eq!(
        parse_shared_session_inactivity_minutes("307445734561825861"),
        None,
        "a value whose *60 would overflow u64 must be rejected outright"
    );
    assert_eq!(
        parse_shared_session_inactivity_minutes(&u64::MAX.to_string()),
        None
    );
}

#[test]
fn minutes_reports_zero_for_a_zero_duration() {
    assert_eq!(shared_session_inactivity_minutes(Duration::ZERO), 0);
}

#[test]
fn minutes_rounds_up_for_a_nonzero_duration() {
    assert_eq!(
        shared_session_inactivity_minutes(Duration::from_secs(60)),
        1
    );
    assert_eq!(
        shared_session_inactivity_minutes(Duration::from_secs(61)),
        2
    );
    assert_eq!(
        shared_session_inactivity_minutes(Duration::from_secs(119)),
        2
    );
}

#[test]
fn display_text_reads_off_for_zero_and_a_plain_number_otherwise() {
    assert_eq!(shared_session_inactivity_display_text(0), "Off");
    assert_eq!(shared_session_inactivity_display_text(1), "1");
    assert_eq!(shared_session_inactivity_display_text(30), "30");
}

#[test]
fn clamp_revoke_never_exceeds_warning_or_end() {
    // Within bounds: unchanged.
    assert_eq!(clamp_shared_session_revoke_minutes(5, 25, 30), 5);
    // Above warning: pulled down to warning.
    assert_eq!(clamp_shared_session_revoke_minutes(50, 25, 30), 25);
    // Above end (but below warning is moot since warning < end normally): pulled to the
    // smaller of the two neighbors.
    assert_eq!(clamp_shared_session_revoke_minutes(50, 60, 30), 30);
}

#[test]
fn clamp_warning_stays_between_revoke_and_end() {
    // Within bounds: unchanged.
    assert_eq!(clamp_shared_session_warning_minutes(25, 10, 30), 25);
    // Below revoke: pulled up to revoke.
    assert_eq!(clamp_shared_session_warning_minutes(5, 10, 30), 10);
    // Above end: pulled down to end.
    assert_eq!(clamp_shared_session_warning_minutes(50, 10, 30), 30);
}

#[test]
fn clamp_end_never_falls_below_revoke_or_warning() {
    // Within bounds: unchanged.
    assert_eq!(clamp_shared_session_end_minutes(30, 10, 25), 30);
    // Below warning: pulled up to warning.
    assert_eq!(clamp_shared_session_end_minutes(5, 10, 25), 25);
    // Below revoke (warning also below revoke here): pulled up to the larger neighbor.
    assert_eq!(clamp_shared_session_end_minutes(5, 25, 10), 25);
}

/// A user can always re-enable a disabled/edge value: clamping never produces a value the
/// user cannot subsequently move away from by editing the same field again.
#[test]
fn clamping_is_idempotent_once_ordering_holds() {
    let revoke = clamp_shared_session_revoke_minutes(10, 25, 30);
    let warning = clamp_shared_session_warning_minutes(25, revoke, 30);
    let end = clamp_shared_session_end_minutes(30, revoke, warning);
    assert!(revoke <= warning);
    assert!(warning <= end);

    // Re-clamping already-consistent values must not change them further.
    assert_eq!(
        clamp_shared_session_revoke_minutes(revoke, warning, end),
        revoke
    );
    assert_eq!(
        clamp_shared_session_warning_minutes(warning, revoke, end),
        warning
    );
    assert_eq!(clamp_shared_session_end_minutes(end, revoke, warning), end);
}

// ---------------------------------------------------------------------------
// Zero (disabled) semantics matrix: a disabled neighbor is exempt from being a bound,
// and zero is always accepted unconditionally in every field regardless of the others.
// ---------------------------------------------------------------------------

#[test]
fn zero_is_always_accepted_unconditionally_in_every_field() {
    // Disabling revoke is allowed no matter what warn/end currently are.
    assert_eq!(clamp_shared_session_revoke_minutes(0, 25, 30), 0);
    assert_eq!(clamp_shared_session_revoke_minutes(0, 0, 0), 0);
    // Disabling warn is allowed no matter what revoke/end currently are.
    assert_eq!(clamp_shared_session_warning_minutes(0, 10, 30), 0);
    assert_eq!(clamp_shared_session_warning_minutes(0, 0, 0), 0);
    // Disabling end is allowed no matter what revoke/warn currently are.
    assert_eq!(clamp_shared_session_end_minutes(0, 10, 25), 0);
    assert_eq!(clamp_shared_session_end_minutes(0, 0, 0), 0);
}

#[test]
fn a_disabled_neighbor_does_not_bound_a_non_zero_value() {
    // Revoke disabled: warn's non-zero value isn't bounded below by it.
    assert_eq!(clamp_shared_session_warning_minutes(5, 0, 30), 5);
    // End disabled: warn's non-zero value isn't bounded above by it.
    assert_eq!(clamp_shared_session_warning_minutes(50, 10, 0), 50);
    // Both disabled: warn's non-zero value is left completely alone.
    assert_eq!(clamp_shared_session_warning_minutes(15, 0, 0), 15);

    // Warn and end disabled: revoke's non-zero value has no upper bound.
    assert_eq!(clamp_shared_session_revoke_minutes(999, 0, 0), 999);
    // Revoke and warn disabled: end's non-zero value has no lower bound.
    assert_eq!(clamp_shared_session_end_minutes(1, 0, 0), 1);
}

#[test]
fn re_enabling_a_disabled_field_is_never_blocked_by_its_neighbors() {
    // Starting from all-zero (every phase disabled), typing a positive value into any one
    // field must be accepted outright, since there are no non-zero neighbors to clamp
    // against.
    assert_eq!(clamp_shared_session_revoke_minutes(5, 0, 0), 5);
    assert_eq!(clamp_shared_session_warning_minutes(5, 0, 0), 5);
    assert_eq!(clamp_shared_session_end_minutes(5, 0, 0), 5);
}
