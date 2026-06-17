//! Unit tests for the pure helpers in `wait_for_events`.

use std::time::Duration;

use super::{
    watchdog_timeout_for_stamped_seconds, CLIENT_WATCHDOG_SAFETY_MARGIN,
    DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS, HARD_FLOOR,
};

#[test]
fn watchdog_timeout_constants_match_documented_values() {
    // The behavioural tests below assert the contract; this trips if
    // someone moves a constant without updating the documented intent.
    assert_eq!(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS, 30 * 60);
    assert_eq!(CLIENT_WATCHDOG_SAFETY_MARGIN, Duration::from_secs(30));
    assert_eq!(HARD_FLOOR, Duration::from_secs(5));
}

#[test]
fn watchdog_timeout_subtracts_margin_for_stamped_minute() {
    // A 60s stamped timeout has 30s of headroom after subtracting the
    // safety margin — that's the canonical "happy path" the safety
    // margin is designed for.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(60),
        Duration::from_secs(30)
    );
}

#[test]
fn watchdog_timeout_clamps_to_hard_floor_when_stamped_value_is_too_small() {
    // A 10s stamped timeout would become negative after subtracting the
    // 30s safety margin — the hard floor kicks in so the watchdog still
    // fires after a finite delay.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(10),
        HARD_FLOOR,
        "stamped 10s should clamp to HARD_FLOOR after subtracting the safety margin"
    );
}

#[test]
fn watchdog_timeout_falls_back_to_default_minus_margin_when_unset() {
    // Prost flattens scalars, so the proto's "unset" looks like `0` on
    // the Rust side; treat that as "use the default minus margin".
    let expected = Duration::from_secs(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS as u64)
        - CLIENT_WATCHDOG_SAFETY_MARGIN;
    assert_eq!(watchdog_timeout_for_stamped_seconds(0), expected);
}

#[test]
fn watchdog_timeout_clamps_negative_value_to_default_minus_margin() {
    // Defense against a buggy or malicious payload. `Duration::from_secs`
    // takes a `u64`; a negative value would underflow without the clamp.
    let expected = Duration::from_secs(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS as u64)
        - CLIENT_WATCHDOG_SAFETY_MARGIN;
    assert_eq!(watchdog_timeout_for_stamped_seconds(-42), expected);
}

#[test]
fn watchdog_timeout_preserves_large_stamped_value() {
    // Server-supplied values well above the margin pass through as
    // (stamped - margin). 15 minutes stays at 14m30s after the
    // subtraction.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(900),
        Duration::from_secs(900) - CLIENT_WATCHDOG_SAFETY_MARGIN
    );
}
