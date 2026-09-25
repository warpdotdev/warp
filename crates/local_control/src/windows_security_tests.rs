use std::fs;

use super::*;

#[test]
fn current_user_sid_matches_own_process_token() {
    let current = current_user_sid().expect("current user SID");

    assert!(
        current.starts_with("S-1-"),
        "unexpected SID format: {current}"
    );
    assert_eq!(
        process_user_sid(std::process::id()).expect("own process SID"),
        current
    );
}

#[test]
fn process_user_check_accepts_same_user() {
    let current = current_user_sid().expect("current user SID");

    ensure_process_user(std::process::id(), &current).expect("same user is accepted");
}

#[test]
fn process_user_check_rejects_different_user() {
    let err = ensure_process_user(std::process::id(), LOCAL_SYSTEM_SID)
        .expect_err("different user is rejected");

    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn private_acl_is_accepted_after_protection() {
    let dir = tempfile::tempdir().expect("temp dir");
    set_private_acl(dir.path(), true).expect("directory is protected");
    let record = dir.path().join("inst_test.json");
    fs::write(&record, "{}").expect("write record");
    set_private_acl(&record, false).expect("record is protected");

    validate_private_acl(dir.path()).expect("protected directory is accepted");
    validate_private_acl(&record).expect("protected record is accepted");
}

#[test]
fn record_with_only_inherited_acl_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    set_private_acl(dir.path(), true).expect("directory is protected");
    let record = dir.path().join("inst_test.json");
    fs::write(&record, "{}").expect("write record");

    // Inherited entries are not a protected DACL, so the record still needs
    // its own protection before clients will trust it.
    let err = validate_private_acl(&record).expect_err("inherited record ACL is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn inherited_acl_is_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");

    let err = validate_private_acl(dir.path()).expect_err("inherited ACL is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn private_sddl_grants_only_owner_system_and_administrators() {
    assert_eq!(
        private_sddl("S-1-5-21-1", true),
        "D:P(A;OICI;FA;;;S-1-5-21-1)(A;OICI;FA;;;S-1-5-18)(A;OICI;FA;;;S-1-5-32-544)"
    );
    assert_eq!(
        private_sddl("S-1-5-21-1", false),
        "D:P(A;;FA;;;S-1-5-21-1)(A;;FA;;;S-1-5-18)(A;;FA;;;S-1-5-32-544)"
    );
    assert_eq!(broker_pipe_sddl("S-1-5-21-1"), "D:P(A;;GA;;;S-1-5-21-1)");
}

#[test]
fn broker_pipe_security_builds_attributes() {
    let mut security = BrokerPipeSecurity::new().expect("pipe security");

    assert!(!security.as_mut_ptr().is_null());
}
