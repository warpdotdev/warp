use chrono::Duration;

use super::*;
use crate::discovery::InstanceId;
use crate::scripting::ScriptingGrant;

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
fn authenticated_drive_mutation_requires_authenticated_user_grant() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::DriveObjectCreate,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    assert!(grant.authenticated_user.required);
    assert!(grant.authenticated_user.subject.is_none());
    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::DriveObjectCreate)
        .expect_err("missing authenticated user subject is rejected");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserRequired);
}

#[test]
fn authenticated_drive_mutation_grant_allows_only_granted_action() {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::DriveObjectCreate,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = Some("user_123".to_owned());
    grant
        .verify_for_action(&grant.instance_id, ActionKind::DriveObjectCreate)
        .expect("authenticated Drive mutation grant is accepted");
    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::DriveObjectUpdate)
        .expect_err("a different Drive mutation is rejected");
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
fn credential_request_accepts_registry_verified_inside_warp_terminal_proof() {
    let instance_id = InstanceId("inst_test".to_owned());
    let mut registry = TerminalSessionProofRegistry::default();
    let proof = registry.issue(instance_id.clone(), "session-1", Duration::minutes(5));
    let mut request = CredentialRequest::new(ActionKind::InputRun, InvocationContext::InsideWarp);
    request.execution_context_proof = Some(ExecutionContextProof::VerifiedWarpTerminal {
        proof_id: proof.proof_id,
        terminal_session_id: proof.terminal_session_id,
        proof_secret: proof.proof_secret,
    });

    request
        .verify_execution_context_proof_with_registry(&instance_id, &registry)
        .expect("verified terminal proof is accepted");
}

#[test]
fn registry_rejects_terminal_proof_for_wrong_instance() {
    let instance_id = InstanceId("inst_test".to_owned());
    let mut registry = TerminalSessionProofRegistry::default();
    let proof = registry.issue(instance_id, "session-1", Duration::minutes(5));
    let mut request = CredentialRequest::new(ActionKind::InputRun, InvocationContext::InsideWarp);
    request.execution_context_proof = Some(ExecutionContextProof::VerifiedWarpTerminal {
        proof_id: proof.proof_id,
        terminal_session_id: proof.terminal_session_id,
        proof_secret: proof.proof_secret,
    });
    let err = request
        .verify_execution_context_proof_with_registry(
            &InstanceId("other_instance".to_owned()),
            &registry,
        )
        .expect_err("wrong instance is rejected");

    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn authenticated_action_requires_terminal_scripting_grant() {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::InputRun,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = Some("user-1".to_owned());
    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::InputRun)
        .expect_err("missing terminal scripting grant is rejected");

    assert_eq!(err.code, ErrorCode::AuthenticatedUserRequired);
}

#[test]
fn authenticated_action_accepts_matching_terminal_scripting_grant() {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::InputRun,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = Some("user-1".to_owned());
    grant.scripting_grant = Some(ScriptingGrant::verified_warp_terminal(
        "session-1",
        "user-1",
        vec![ActionKind::InputRun],
        Duration::minutes(5),
    ));

    grant
        .verify_for_action(&grant.instance_id, ActionKind::InputRun)
        .expect("matching terminal scripting grant is accepted");
}

#[test]
fn execution_grant_rejects_outside_warp_context_even_with_subject() {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::DriveWorkflowRun,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    )
    .with_authenticated_user_subject("user_123");
    grant.scripting_grant = Some(ScriptingGrant::verified_warp_terminal(
        "session-1",
        "user_123",
        vec![ActionKind::DriveWorkflowRun],
        Duration::minutes(5),
    ));
    let err = grant
        .verify_for_action(&grant.instance_id, ActionKind::DriveWorkflowRun)
        .expect_err("drive.workflow.run cannot run outside Warp");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn credential_request_rejects_placeholder_inside_warp_terminal_proof() {
    let mut request = CredentialRequest::new(ActionKind::TabCreate, InvocationContext::InsideWarp);
    request.execution_context_proof = Some(ExecutionContextProof::VerifiedWarpTerminal {
        proof_id: "proof".to_owned(),
        terminal_session_id: "session".to_owned(),
        proof_secret: "secret".to_owned(),
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
        terminal_session_id: "session".to_owned(),
        proof_secret: "secret".to_owned(),
    });
    let err = request
        .verify_execution_context_proof()
        .expect_err("terminal proof is rejected for external context");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}
