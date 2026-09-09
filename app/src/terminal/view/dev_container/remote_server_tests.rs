use std::sync::Arc;

use super::*;

#[test]
fn setup_phases_are_distinct() {
    assert_ne!(
        DevContainerRemoteSetupPhase::Checking,
        DevContainerRemoteSetupPhase::Connecting
    );
}

#[test]
fn binary_check_connects_when_present() {
    assert_eq!(
        binary_check_decision(&Ok(true), None),
        BinaryCheckDecision::Connect
    );
}

#[test]
fn binary_check_installs_when_missing() {
    assert_eq!(
        binary_check_decision(&Ok(false), None),
        BinaryCheckDecision::Install
    );
}

#[test]
fn binary_check_fails_when_check_errors() {
    assert_eq!(
        binary_check_decision(&Err(Arc::new(Error::TimedOut)), None),
        BinaryCheckDecision::Failed
    );
}

#[test]
fn binary_check_fails_closed_when_unsupported() {
    let preinstall = PreinstallCheckResult::unsupported(
        remote_server::setup::UnsupportedReason::UnsupportedOs {
            os: "plan9".to_owned(),
        },
    );
    assert_eq!(
        binary_check_decision(&Ok(true), Some(&preinstall)),
        BinaryCheckDecision::Unsupported
    );
    assert!(
        unsupported_container_message(Some(&preinstall)).contains("plan9"),
        "{}",
        unsupported_container_message(Some(&preinstall))
    );
}

#[test]
fn musl_preinstall_does_not_block_install() {
    let preinstall =
        PreinstallCheckResult::parse("required_glibc=2.31\nlibc_family=musl\nstatus=supported\n");
    assert_eq!(
        binary_check_decision(&Ok(false), Some(&preinstall)),
        BinaryCheckDecision::Install
    );
    assert_eq!(
        binary_check_decision(&Ok(true), Some(&preinstall)),
        BinaryCheckDecision::Connect
    );
}

#[test]
fn session_connected_replaces_pane_only_for_current_attempt() {
    let session_id = SessionId::from(3);
    assert_eq!(
        session_connected_decision(session_id, session_id, true),
        SessionConnectedDecision::ReplaceBuildPane
    );
    assert_eq!(
        session_connected_decision(session_id, session_id, false),
        SessionConnectedDecision::DeregisterStale
    );
    assert_eq!(
        session_connected_decision(session_id, SessionId::from(4), true),
        SessionConnectedDecision::Ignore
    );
}
