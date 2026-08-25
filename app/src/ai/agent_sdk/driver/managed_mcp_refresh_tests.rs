//! Tests for the proactive managed-MCP re-mint schedule math.

use chrono::{TimeDelta, Utc};

use super::{MIN_DELAY, REFRESH_LEAD, is_due, refresh_delay};

#[test]
fn wakes_one_lead_before_the_earliest_expiry() {
    let now = Utc::now();
    let expiry = now + TimeDelta::hours(3);
    let delay = refresh_delay(expiry, now);
    let expected = std::time::Duration::from_secs(3 * 60 * 60) - REFRESH_LEAD;
    // Allow a little slack for the sub-second remainder of `now`.
    assert!(
        delay >= expected - std::time::Duration::from_secs(1) && delay <= expected,
        "unexpected delay: {delay:?}"
    );
}

#[test]
fn never_spins_faster_than_the_minimum_delay() {
    let now = Utc::now();
    // Expiry inside the lead window, at it, and in the past all clamp.
    for offset in [
        TimeDelta::minutes(4),
        TimeDelta::zero(),
        TimeDelta::minutes(-10),
    ] {
        assert_eq!(refresh_delay(now + offset, now), MIN_DELAY);
    }
}

#[test]
fn due_exactly_within_the_lead_window() {
    let now = Utc::now();
    assert!(is_due(now + TimeDelta::minutes(4), now));
    assert!(is_due(now - TimeDelta::minutes(1), now));
    assert!(!is_due(now + TimeDelta::minutes(6), now));
}
