use super::*;

#[test]
fn missing_auth_mode_defaults_to_task_identity() {
    assert_eq!(AuthMode::parse(None).unwrap(), AuthMode::TaskIdentity);
}

#[test]
fn task_identity_auth_mode_is_explicitly_supported() {
    assert_eq!(
        AuthMode::parse(Some("task_identity")).unwrap(),
        AuthMode::TaskIdentity
    );
}

#[test]
fn none_auth_mode_is_explicitly_supported() {
    assert_eq!(AuthMode::parse(Some("none")).unwrap(), AuthMode::None);
}

#[test]
fn invalid_or_empty_auth_mode_fails_closed() {
    assert!(AuthMode::parse(Some("")).is_err());
    assert!(AuthMode::parse(Some("invalid")).is_err());
}

#[test]
fn task_identity_bootstrap_failure_does_not_downgrade() {
    let result = select_transport(AuthMode::TaskIdentity, || {
        Err(anyhow!("Missing task-identity credential"))
    });

    assert!(result.is_err());
}

#[test]
fn none_auth_mode_bypasses_task_identity_bootstrap() {
    let result = select_transport(AuthMode::None, || {
        panic!("Unauthenticated transport must not load task-identity credentials")
    });

    assert!(matches!(result.unwrap(), Transport::Unauthenticated));
}
