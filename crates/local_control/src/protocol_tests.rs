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
fn input_run_is_not_in_the_allowlisted_catalog() {
    let action = serde_json::from_value::<ActionKind>(serde_json::json!("input.run"));
    assert!(action.is_err());
}

#[test]
fn malformed_action_name_is_not_deserialized() {
    let action = serde_json::from_value::<ActionKind>(serde_json::json!("tab.create.extra"));
    assert!(action.is_err());
}

#[test]
fn tab_create_metadata_is_first_slice_logged_out_safe_mutation() {
    let metadata = ActionKind::TabCreate.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Implemented
    );
    assert_eq!(metadata.risk_tier, RiskTier::MutatingNonDestructive);
    assert_eq!(
        metadata.state_data_category,
        StateDataCategory::AppStateMutation
    );
    assert!(!metadata.requires_authenticated_user);
    assert!(!metadata.authenticated_user.required);
    assert_eq!(
        metadata.permission_category,
        PermissionCategory::MutateAppState
    );
    assert_eq!(
        metadata.allowed_invocation_contexts,
        vec![
            InvocationContext::InsideWarp,
            InvocationContext::OutsideWarp
        ]
    );
}

#[test]
fn structural_metadata_actions_are_logged_out_safe_read_metadata() {
    for action in [
        ActionKind::InstanceList,
        ActionKind::AppPing,
        ActionKind::AppInspect,
        ActionKind::AppVersion,
        ActionKind::AppActive,
        ActionKind::ActionList,
        ActionKind::ActionGet,
        ActionKind::WindowList,
        ActionKind::TabList,
        ActionKind::PaneList,
        ActionKind::SessionList,
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert_eq!(metadata.risk_tier, RiskTier::ReadOnlyMetadata);
        assert_eq!(
            metadata.state_data_category,
            StateDataCategory::MetadataRead
        );
        assert_eq!(
            metadata.permission_category,
            PermissionCategory::ReadMetadata
        );
        assert!(!metadata.requires_authenticated_user);
        assert!(!metadata.authenticated_user.required);
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![
                InvocationContext::InsideWarp,
                InvocationContext::OutsideWarp
            ]
        );
    }
}

#[test]
fn structural_metadata_actions_have_expected_target_scopes() {
    for action in [
        ActionKind::InstanceList,
        ActionKind::AppPing,
        ActionKind::AppInspect,
        ActionKind::AppVersion,
        ActionKind::AppActive,
    ] {
        assert_eq!(action.metadata().target_scope, TargetScope::Instance);
    }

    assert_eq!(
        ActionKind::ActionList.metadata().target_scope,
        TargetScope::Action
    );
    assert_eq!(
        ActionKind::ActionGet.metadata().target_scope,
        TargetScope::Action
    );
    assert_eq!(
        ActionKind::WindowList.metadata().target_scope,
        TargetScope::Window
    );
    assert_eq!(
        ActionKind::TabList.metadata().target_scope,
        TargetScope::Tab
    );
    assert_eq!(
        ActionKind::PaneList.metadata().target_scope,
        TargetScope::Pane
    );
    assert_eq!(
        ActionKind::SessionList.metadata().target_scope,
        TargetScope::Session
    );
}

#[test]
fn action_with_params_roundtrips_typed_action_get_params() {
    let action = Action::with_params(
        ActionKind::ActionGet,
        ActionGetParams {
            action: "tab.create".to_owned(),
        },
    )
    .expect("params serialize");
    assert_eq!(action.kind, ActionKind::ActionGet);
    assert_eq!(action.params["action"], "tab.create");

    let params = action
        .params_as::<ActionGetParams>()
        .expect("params deserialize");
    assert_eq!(params.action, "tab.create");
}

#[test]
fn action_metadata_serializes_security_categories() {
    let metadata = ActionKind::TabCreate.metadata();
    let value = serde_json::to_value(metadata).expect("metadata serializes");
    assert_eq!(value["name"], "tab.create");
    assert_eq!(value["state_data_category"], "app_state_mutation");
    assert_eq!(value["permission_category"], "mutate_app_state");
    assert_eq!(
        value["authenticated_user"]["required"],
        serde_json::json!(false)
    );
}

#[test]
fn default_permissions_preserve_security_categories() {
    assert_eq!(
        ActionKind::TabCreate.metadata().permission_category,
        PermissionCategory::MutateAppState
    );
    assert_eq!(
        ActionKind::InputInsert.metadata().permission_category,
        PermissionCategory::MutateUnderlyingData
    );
    assert_eq!(
        ActionKind::SettingSet.metadata().permission_category,
        PermissionCategory::MutateMetadataConfiguration
    );
    assert_eq!(
        ActionKind::TabList.metadata().permission_category,
        PermissionCategory::ReadMetadata
    );
}

#[test]
fn non_first_slice_actions_are_catalog_stubs() {
    let metadata = ActionKind::WindowCreate.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Stub
    );
    assert!(
        !metadata
            .allowed_invocation_contexts
            .contains(&InvocationContext::OutsideWarp)
    );
}
