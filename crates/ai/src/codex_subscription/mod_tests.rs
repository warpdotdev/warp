use super::*;

#[test]
fn known_expiry_refreshes_five_minutes_early() {
    assert_eq!(
        refresh_delay(Some(60 * 60)),
        Duration::from_secs(55 * 60)
    );
}

#[test]
fn near_expiry_refreshes_immediately() {
    assert_eq!(refresh_delay(Some(60)), Duration::ZERO);
}

#[test]
fn missing_expiry_refreshes_after_twenty_four_hours() {
    assert_eq!(refresh_delay(None), Duration::from_secs(24 * 60 * 60));
}
