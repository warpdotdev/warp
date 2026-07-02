use std::time::Duration;

use instant::Instant;

use super::{ExitConfirmation, CTRL_C_EXIT_WINDOW};

#[test]
fn starts_disarmed() {
    let confirmation = ExitConfirmation::default();
    assert!(!confirmation.is_armed());
    assert!(!confirmation.should_exit(Instant::now()));
}

#[test]
fn second_press_within_window_exits() {
    let mut confirmation = ExitConfirmation::default();
    let now = Instant::now();
    confirmation.arm(now);
    assert!(confirmation.is_armed());
    assert!(confirmation.should_exit(now + CTRL_C_EXIT_WINDOW / 2));
}

#[test]
fn press_after_window_lapses_does_not_exit() {
    let mut confirmation = ExitConfirmation::default();
    let now = Instant::now();
    confirmation.arm(now);
    assert!(!confirmation.should_exit(now + CTRL_C_EXIT_WINDOW));
}

#[test]
fn rearm_supersedes_stale_timer() {
    let mut confirmation = ExitConfirmation::default();
    let now = Instant::now();
    let first_expiry = confirmation.arm(now);
    let second_expiry = confirmation.arm(now + Duration::from_millis(300));

    assert!(
        !confirmation.disarm_expired(first_expiry),
        "the superseded window's timer must not disarm the newer window"
    );
    assert!(confirmation.is_armed());
    assert!(confirmation.disarm_expired(second_expiry));
    assert!(!confirmation.is_armed());
}

#[test]
fn disarm_reports_whether_armed() {
    let mut confirmation = ExitConfirmation::default();
    assert!(!confirmation.disarm());
    confirmation.arm(Instant::now());
    assert!(confirmation.disarm());
    assert!(!confirmation.is_armed());
}
