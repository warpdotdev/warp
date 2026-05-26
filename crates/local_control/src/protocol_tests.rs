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
fn excluded_action_names_are_not_deserialized() {
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
        vec![InvocationContext::OutsideWarp]
    );
}

#[test]
fn core_smoke_metadata_has_explicit_read_metadata_category() {
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
        assert_eq!(metadata.risk_tier, RiskTier::ReadOnlyMetadata);
        assert_eq!(
            metadata.state_data_category,
            StateDataCategory::MetadataRead
        );
        assert_eq!(
            metadata.permission_category,
            PermissionCategory::ReadMetadata
        );
        assert!(!metadata.authenticated_user.required);
        assert_eq!(metadata.target_scope, TargetScope::Instance);
    }
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
        PermissionCategory::MutateAppState
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
fn logged_out_safe_stub_actions_can_advertise_external_context() {
    let metadata = ActionKind::WindowCreate.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Stub
    );
    assert!(!metadata.authenticated_user.required);
    assert!(metadata
        .allowed_invocation_contexts
        .contains(&InvocationContext::OutsideWarp));
}


#[test]
fn readonly_capability_targets_are_implemented_with_expected_categories() {
    for action in [
        ActionKind::InstanceInspect,
        ActionKind::CapabilityList,
        ActionKind::CapabilityInspect,
        ActionKind::ActionList,
        ActionKind::ActionInspect,
        ActionKind::WindowList,
        ActionKind::WindowInspect,
        ActionKind::TabList,
        ActionKind::TabInspect,
        ActionKind::PaneList,
        ActionKind::PaneInspect,
        ActionKind::SessionList,
        ActionKind::SessionInspect,
        ActionKind::ThemeGet,
        ActionKind::KeybindingList,
        ActionKind::KeybindingGet,
        ActionKind::FileList,
        ActionKind::ProjectActive,
        ActionKind::ProjectList,
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert_eq!(metadata.permission_category, PermissionCategory::ReadMetadata);
        assert!(!metadata.authenticated_user.required);
    }

    for action in [
        ActionKind::BlockInspect,
        ActionKind::BlockOutput,
        ActionKind::InputGet,
        ActionKind::HistoryList,
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert_eq!(
            metadata.permission_category,
            PermissionCategory::ReadUnderlyingData
        );
        assert!(!metadata.authenticated_user.required);
    }
}

#[test]
fn block_output_uses_block_id_params() {
    assert_eq!(
        ActionKind::BlockOutput.metadata().parameter_spec,
        ActionParameterSpec::BlockId
    );
    let action = Action::with_params(
        ActionKind::BlockOutput,
        BlockIdParams {
            block_id: "block_1".to_owned(),
        },
    )
    .expect("params serialize");
    assert_eq!(action.params["block_id"], "block_1");
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
