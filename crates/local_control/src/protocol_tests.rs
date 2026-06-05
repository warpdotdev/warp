use super::*;

#[test]
fn request_envelope_serializes_stable_action_names() {
    let request = RequestEnvelope::new(Action::new(ActionKind::WindowFocus));
    let value = serde_json::to_value(&request).expect("request serializes");
    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(value["action"]["kind"], "window.focus");
}

#[test]
fn response_error_serializes_machine_code() {
    let response = ResponseEnvelope::error(
        Uuid::nil(),
        ControlError::new(ErrorCode::UnauthorizedLocalClient, "bad token"),
    );
    let value = serde_json::to_value(&response).expect("response serializes");
    assert_eq!(value["response"]["status"], "error");
    assert_eq!(
        value["response"]["error"]["code"],
        "unauthorized_local_client"
    );
}

#[test]
fn ambiguous_target_error_code_is_stable() {
    let value = serde_json::to_value(ErrorCode::AmbiguousTarget).expect("code serializes");
    assert_eq!(value, serde_json::json!("ambiguous_target"));
}

#[test]
fn malformed_action_name_is_not_deserialized() {
    let action = serde_json::from_value::<ActionKind>(serde_json::json!("tab.create.extra"));
    assert!(action.is_err());
}

#[test]
fn non_allowlisted_action_names_are_not_deserialized() {
    for action in [
        "file.write",
        "file.delete",
        "auth.api_key.set",
        "auth.api_key.status",
        "auth.api_key.revoke",
    ] {
        assert!(serde_json::from_value::<ActionKind>(serde_json::json!(action)).is_err());
    }
}

#[test]
fn tab_create_metadata_is_first_slice_logged_out_safe_action() {
    let metadata = ActionKind::TabCreate.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Implemented
    );
    assert!(!metadata.requires_authenticated_user);
    assert!(!metadata.authenticated_user.required);
    assert_eq!(
        metadata.allowed_invocation_contexts,
        vec![InvocationContext::OutsideWarp]
    );
    assert_eq!(metadata.target_scope, TargetScope::Tab);
}

#[test]
fn core_smoke_metadata_has_explicit_instance_policy() {
    for action in [
        ActionKind::InstanceList,
        ActionKind::AppPing,
        ActionKind::AppVersion,
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert!(!metadata.authenticated_user.required);
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![InvocationContext::OutsideWarp]
        );
        assert_eq!(metadata.target_scope, TargetScope::Instance);
    }
}

#[test]
fn implemented_catalog_is_exactly_the_first_slice() {
    let actions = ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        vec![
            ActionKind::InstanceList,
            ActionKind::AppPing,
            ActionKind::AppVersion,
            ActionKind::TabCreate,
        ]
    );
}

#[test]
fn action_metadata_serializes_action_policy() {
    let metadata = ActionKind::TabCreate.metadata();
    let value = serde_json::to_value(metadata).expect("metadata serializes");
    assert_eq!(value["name"], "tab.create");
    assert_eq!(value["implementation_status"], "implemented");
    assert_eq!(
        value["authenticated_user"]["required"],
        serde_json::json!(false)
    );
    assert_eq!(
        value["allowed_invocation_contexts"],
        serde_json::json!(["outside_warp"])
    );
    assert_eq!(value["target_scope"], "tab");
}

#[test]
fn logged_out_safe_stub_actions_can_advertise_external_context() {
    let metadata = ActionKind::WindowCreate.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Stub
    );
    assert!(!metadata.authenticated_user.required);
    assert!(
        metadata
            .allowed_invocation_contexts
            .contains(&InvocationContext::OutsideWarp)
    );
}

#[test]
fn authenticated_actions_are_warp_terminal_only_in_the_contract() {
    for action in [
        ActionKind::DriveInspect,
        ActionKind::DriveObjectCreate,
        ActionKind::DriveWorkflowRun,
        ActionKind::InputRun,
    ] {
        let metadata = action.metadata();
        assert!(metadata.authenticated_user.required);
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![InvocationContext::InsideWarp]
        );
    }
}
