use super::*;

fn action_name(kind: ActionKind) -> String {
    serde_json::to_value(kind)
        .expect("action kind serializes")
        .as_str()
        .expect("action kind serializes as string")
        .to_owned()
}

fn error_code_name(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .expect("error code serializes")
        .as_str()
        .expect("error code serializes as string")
        .to_owned()
}

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
fn malformed_action_name_is_not_deserialized() {
    let action = serde_json::from_value::<ActionKind>(serde_json::json!("tab.create.extra"));
    assert!(action.is_err());
}

#[test]
fn file_content_and_unapproved_execution_actions_are_not_in_catalog() {
    for action in [
        "file.read",
        "file.write",
        "file.append",
        "file.delete",
        "drive.object.share-public",
        "drive.object.share-external",
        "agent.prompt.submit",
        "command.accept",
    ] {
        assert!(serde_json::from_value::<ActionKind>(serde_json::json!(action)).is_err());
    }
}

#[test]
fn action_catalog_has_unique_stable_names() {
    let names = ActionKind::ALL
        .iter()
        .copied()
        .map(action_name)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), ActionKind::ALL.len());

    for action in ActionKind::ALL {
        assert_eq!(action.as_str(), action_name(*action));
        assert_eq!(action.metadata().name, action.as_str());
    }
}

#[test]
fn implemented_metadata_matches_action_status() {
    let implemented = ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect::<std::collections::HashSet<_>>();
    let expected = ActionKind::ALL
        .iter()
        .copied()
        .filter(|action| action.is_implemented())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(implemented, expected);
    assert!(implemented.contains(&ActionKind::InputRun));
    assert!(implemented.contains(&ActionKind::DriveObjectShareToTeam));
    assert!(!implemented.contains(&ActionKind::DriveWorkflowRun));
}

#[test]
fn implemented_actions_allow_outside_warp_invocation() {
    for metadata in ActionKind::implemented_metadata() {
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![InvocationContext::OutsideWarp]
        );
    }
}

#[test]
fn representative_security_categories_are_stable() {
    let cases = [
        (
            ActionKind::TabList,
            StateDataCategory::MetadataRead,
            PermissionCategory::ReadMetadata,
        ),
        (
            ActionKind::BlockOutput,
            StateDataCategory::UnderlyingDataRead,
            PermissionCategory::ReadUnderlyingData,
        ),
        (
            ActionKind::TabCreate,
            StateDataCategory::AppStateMutation,
            PermissionCategory::MutateAppState,
        ),
        (
            ActionKind::SettingSet,
            StateDataCategory::MetadataConfigurationMutation,
            PermissionCategory::MutateMetadataConfiguration,
        ),
        (
            ActionKind::InputRun,
            StateDataCategory::UnderlyingDataMutation,
            PermissionCategory::MutateUnderlyingData,
        ),
        (
            ActionKind::DriveObjectShareToTeam,
            StateDataCategory::UnderlyingDataMutation,
            PermissionCategory::MutateUnderlyingData,
        ),
    ];

    for (action, state_category, permission_category) in cases {
        let metadata = action.metadata();
        assert_eq!(metadata.state_data_category, state_category);
        assert_eq!(metadata.permission_category, permission_category);
    }
}

#[test]
fn input_run_is_authenticated_underlying_mutation() {
    let metadata = ActionKind::InputRun.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Implemented
    );
    assert_eq!(
        metadata.state_data_category,
        StateDataCategory::UnderlyingDataMutation
    );
    assert_eq!(
        metadata.permission_category,
        PermissionCategory::MutateUnderlyingData
    );
    assert_eq!(metadata.parameter_spec, ActionParameterSpec::Text);
    assert!(metadata.requires_authenticated_user);
    assert!(metadata.authenticated_user.required);
}

#[test]
fn drive_sharing_contract_distinguishes_dialog_from_team_mutation() {
    let share_open = ActionKind::DriveObjectShareOpen.metadata();
    assert_eq!(share_open.name, "drive.object.share.open");
    assert_eq!(
        share_open.state_data_category,
        StateDataCategory::AppStateMutation
    );
    assert_eq!(
        share_open.permission_category,
        PermissionCategory::MutateAppState
    );
    assert!(share_open.requires_authenticated_user);

    let share_to_team = ActionKind::DriveObjectShareToTeam.metadata();
    assert_eq!(share_to_team.name, "drive.object.share_to_team");
    assert_eq!(
        share_to_team.state_data_category,
        StateDataCategory::UnderlyingDataMutation
    );
    assert_eq!(
        share_to_team.permission_category,
        PermissionCategory::MutateUnderlyingData
    );
    assert!(share_to_team.requires_authenticated_user);
}

#[test]
fn drive_selector_and_typed_params_serialize_stably() {
    let target = TargetSelector {
        drive_object: Some(DriveObjectTarget::Lookup {
            object_type: DriveObjectType::Notebook,
            name_or_path: "Team/Runbook".to_owned(),
        }),
        ..TargetSelector::default()
    };
    let value = serde_json::to_value(target).expect("target serializes");
    assert_eq!(value["drive_object"]["type"], "lookup");
    assert_eq!(value["drive_object"]["object_type"], "notebook");

    let params = ActionParams::DriveObjectId {
        id: DriveObjectId("drive_123".to_owned()),
    };
    let value = serde_json::to_value(params).expect("params serialize");
    assert_eq!(value["type"], "drive_object_id");
    assert_eq!(value["id"], "drive_123");
}

#[test]
fn structured_error_codes_have_unique_stable_strings() {
    let codes = [
        ErrorCode::LocalControlDisabled,
        ErrorCode::UnauthorizedLocalClient,
        ErrorCode::InsufficientPermissions,
        ErrorCode::AuthenticatedUserRequired,
        ErrorCode::AuthenticatedUserUnavailable,
        ErrorCode::ExecutionContextNotAllowed,
        ErrorCode::ProtocolVersionUnsupported,
        ErrorCode::InvalidRequest,
        ErrorCode::InvalidSelector,
        ErrorCode::InvalidParams,
        ErrorCode::NoInstance,
        ErrorCode::AmbiguousInstance,
        ErrorCode::AmbiguousTarget,
        ErrorCode::StaleTarget,
        ErrorCode::TargetStateConflict,
        ErrorCode::MissingTarget,
        ErrorCode::TransportUnavailable,
        ErrorCode::BridgeUnavailable,
        ErrorCode::UnsupportedAction,
        ErrorCode::NotAllowlisted,
        ErrorCode::Internal,
    ];
    let serialized = codes
        .into_iter()
        .map(error_code_name)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(serialized.len(), codes.len());
    assert!(serialized.contains("authenticated_user_unavailable"));
    assert!(serialized.contains("not_allowlisted"));
}
