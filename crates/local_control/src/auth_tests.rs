use chrono::Duration;

use super::*;
use crate::discovery::InstanceId;

#[test]
fn rejects_missing_authorization_header() {
    let token = AuthToken::from_secret("secret");
    let err = token
        .verify_authorization_header(None)
        .expect_err("rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}
#[test]
fn rejects_malformed_authorization_header() {
    let token = AuthToken::from_secret("secret");
    let err = token
        .verify_authorization_header(Some("Basic secret"))
        .expect_err("rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn rejects_wrong_bearer_token() {
    let token = AuthToken::from_secret("secret");
    let err = token
        .verify_authorization_header(Some("Bearer wrong"))
        .expect_err("rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn accepts_matching_bearer_token() {
    let token = AuthToken::from_secret("secret");
    token
        .verify_authorization_header(Some("Bearer secret"))
        .expect("accepted");
}

#[test]
fn scoped_credential_allows_only_granted_action() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );
    grant
        .verify_for_action(&grant.instance_id, ActionKind::TabCreate)
        .expect("tab.create grant is accepted");
    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::WindowCreate)
        .expect_err("other actions are rejected");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn scoped_credential_rejects_different_instance() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );
    let err = grant
        .verify_for_action(&InstanceId("inst_other".to_owned()), ActionKind::TabCreate)
        .expect_err("other instance is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}
#[test]
fn scoped_credential_rejects_expired_grant() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        InvocationContext::OutsideWarp,
        Duration::minutes(-1),
    );

    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::TabCreate)
        .expect_err("expired grant is rejected");
    assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn scoped_credential_carries_authenticated_user_metadata() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );
    assert!(!grant.authenticated_user.required);
    assert!(grant.authenticated_user.subject.is_none());
}

#[test]
fn authenticated_user_actions_require_subject() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::DriveInspect,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    assert!(grant.authenticated_user.required);
    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::DriveInspect)
        .expect_err("authenticated-user actions require a subject");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserRequired);
}

#[test]
fn credential_request_rejects_unverified_inside_warp_context() {
    let request = CredentialRequest::new(ActionKind::TabCreate, InvocationContext::InsideWarp);
    let err = request
        .verify_execution_context_proof()
        .expect_err("missing proof is rejected");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn credential_request_rejects_placeholder_inside_warp_terminal_proof() {
    let mut request = CredentialRequest::new(ActionKind::TabCreate, InvocationContext::InsideWarp);
    request.execution_context_proof = Some(ExecutionContextProof::VerifiedWarpTerminal {
        proof_id: "proof".to_owned(),
    });
    let err = request
        .verify_execution_context_proof()
        .expect_err("placeholder proof is rejected until broker support exists");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn credential_request_rejects_terminal_proof_for_external_client() {
    let mut request = CredentialRequest::new(ActionKind::TabCreate, InvocationContext::OutsideWarp);
    request.execution_context_proof = Some(ExecutionContextProof::VerifiedWarpTerminal {
        proof_id: "proof".to_owned(),
    });
    let err = request
        .verify_execution_context_proof()
        .expect_err("terminal proof is rejected for external context");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}
