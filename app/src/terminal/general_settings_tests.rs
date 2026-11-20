use settings::Setting as _;

use super::*;

#[test]
fn persistent_sessions_defaults_to_enabled() {
    assert!(PersistentSessions::default_value());
    assert_eq!(
        PersistentSessions::toml_path(),
        Some("general.persistent_sessions")
    );
}

#[test]
fn persistent_sessions_require_restore_session_and_no_appimage() {
    assert_eq!(persistent_sessions_enabled(true, true, false), cfg!(unix));
    assert!(!persistent_sessions_enabled(false, true, false));
    assert!(!persistent_sessions_enabled(true, false, false));
    assert!(!persistent_sessions_enabled(true, true, true));
}

#[test]
fn integration_tests_only_persist_sessions_when_opted_in() {
    assert!(!persistent_sessions_allowed_for_channel(
        Channel::Integration,
        false
    ));
    assert!(persistent_sessions_allowed_for_channel(
        Channel::Integration,
        true
    ));
    assert!(persistent_sessions_allowed_for_channel(
        Channel::Stable,
        false
    ));
    assert!(persistent_sessions_allowed_for_channel(Channel::Oss, false));
}
