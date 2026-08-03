use chrono::Duration;

use super::*;
use crate::discovery::InstanceId;

#[test]
fn rejects_missing_authorization_header() {
    let token = AuthToken::from_secret("secret");
    let error = token
        .verify_authorization_header(None)
        .expect_err("rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn rejects_malformed_authorization_header() {
    let token = AuthToken::from_secret("secret");
    let error = token
        .verify_authorization_header(Some("Basic secret"))
        .expect_err("rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn rejects_wrong_bearer_token() {
    let token = AuthToken::from_secret("secret");
    let error = token
        .verify_authorization_header(Some("Bearer wrong"))
        .expect_err("rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn accepts_matching_bearer_token() {
    AuthToken::from_secret("secret")
        .verify_authorization_header(Some("Bearer secret"))
        .expect("accepted");
}

#[test]
fn scoped_credential_allows_only_granted_action() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        Duration::minutes(5),
    );
    grant
        .verify_for_action(&grant.instance_id, ActionKind::TabCreate)
        .expect("tab.create grant is accepted");
    let error = grant
        .verify_for_action(&grant.instance_id, ActionKind::WindowCreate)
        .expect_err("other actions are rejected");
    assert_eq!(error.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn scoped_credential_rejects_different_instance() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        Duration::minutes(5),
    );
    let error = grant
        .verify_for_action(&InstanceId("inst_other".to_owned()), ActionKind::TabCreate)
        .expect_err("other instance is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn scoped_credential_rejects_expired_grant() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        Duration::minutes(-1),
    );
    let error = grant
        .verify_for_action(&grant.instance_id, ActionKind::TabCreate)
        .expect_err("expired grant is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn scoped_credential_allows_confirmation_required_action_scope() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::WindowClose,
        Duration::minutes(5),
    );
    grant
        .verify_for_action(&grant.instance_id, ActionKind::WindowClose)
        .expect("exact-action credential is separate from one-shot confirmation");
}

#[test]
fn credential_request_carries_only_action() {
    let request = CredentialRequest::new(ActionKind::TabCreate);
    assert_eq!(request.action, ActionKind::TabCreate);
    assert_eq!(request.protocol_version, crate::protocol::PROTOCOL_VERSION);
}
