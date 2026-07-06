use super::TransientHint;

#[test]
fn show_displays_the_notice() {
    let mut hint = TransientHint::default();
    assert_eq!(hint.current(), None);
    hint.show("notice".to_owned());
    assert_eq!(hint.current(), Some("notice"));
}

#[test]
fn matching_expiry_clears_the_notice() {
    let mut hint = TransientHint::default();
    let generation = hint.show("notice".to_owned());
    assert!(hint.clear_expired(generation));
    assert_eq!(hint.current(), None);
}

#[test]
fn superseded_expiry_does_not_clear_the_newer_notice() {
    let mut hint = TransientHint::default();
    let stale = hint.show("first".to_owned());
    hint.show("second".to_owned());
    assert!(!hint.clear_expired(stale));
    assert_eq!(hint.current(), Some("second"));
}

#[test]
fn expiry_after_clear_is_a_noop() {
    let mut hint = TransientHint::default();
    let generation = hint.show("notice".to_owned());
    assert!(hint.clear_expired(generation));
    assert!(!hint.clear_expired(generation));
    assert_eq!(hint.current(), None);
}
