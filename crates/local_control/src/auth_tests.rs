use chrono::Duration;

use super::*;
use crate::discovery::InstanceId;
use crate::protocol::{PermissionCategory, StateDataCategory};

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
        .verify_for_action(ActionKind::TabCreate)
        .expect("tab.create grant is accepted");
    let err = grant
        .verify_for_action(ActionKind::WindowCreate)
        .expect_err("other actions are rejected");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn scoped_credential_carries_permission_and_authenticated_user_metadata() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );
    assert_eq!(grant.risk_tier, RiskTier::MutatingNonDestructive);
    assert_eq!(
        grant.state_data_category,
        StateDataCategory::AppStateMutation
    );
    assert_eq!(
        grant.permission_category,
        PermissionCategory::MutateAppState
    );
    assert!(!grant.authenticated_user.required);
    assert!(grant.authenticated_user.subject.is_none());
}

#[test]
fn mismatched_permission_metadata_is_rejected() {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );
    grant.permission_category = PermissionCategory::ReadMetadata;
    let err = grant
        .verify_for_action(ActionKind::TabCreate)
        .expect_err("metadata mismatch is rejected");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
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
