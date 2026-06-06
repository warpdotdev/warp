use super::*;

#[test]
fn request_envelope_serializes_stable_action_names() {
    let request = RequestEnvelope::new(Action::new(ActionKind::WindowFocus));
    let value = serde_json::to_value(&request).expect("request serializes");
    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(value["action"]["kind"], "window.focus");
}

#[test]
fn input_staging_actions_are_non_executing_app_state_mutations() {
    for action in [
        ActionKind::InputInsert,
        ActionKind::InputReplace,
        ActionKind::InputClear,
        ActionKind::InputModeSet,
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert!(!metadata.authenticated_user.required);
    }

    let run_metadata = ActionKind::InputRun.metadata();
    assert_eq!(
        run_metadata.implementation_status,
        ActionImplementationStatus::Implemented
    );
    assert!(run_metadata.authenticated_user.required);
    assert_eq!(
        run_metadata.allowed_invocation_contexts,
        vec![InvocationContext::InsideWarp]
    );
}

#[test]
fn execution_actions_are_authenticated_and_implemented() {
    for action in [ActionKind::InputRun, ActionKind::DriveWorkflowRun] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert!(metadata.authenticated_user.required);
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![InvocationContext::InsideWarp]
        );
    }
}

#[test]
fn execution_action_params_roundtrip() {
    let action = Action::with_params(
        ActionKind::InputRun,
        ActionParams::Text {
            text: "cargo check".to_owned(),
        },
    )
    .expect("input.run params serialize");
    let ActionParams::Text { text } = action.params_as::<ActionParams>().expect("params decode")
    else {
        panic!("expected text params");
    };
    assert_eq!(text, "cargo check");

    let workflow = Action::with_params(
        ActionKind::DriveWorkflowRun,
        ActionParams::WorkflowRun(WorkflowRunParams {
            id: DriveObjectId("workflow_123".to_owned()),
            args: vec![WorkflowArgument {
                name: "name".to_owned(),
                value: "value".to_owned(),
            }],
        }),
    )
    .expect("drive.workflow.run params serialize");
    let ActionParams::WorkflowRun(params) =
        workflow.params_as::<ActionParams>().expect("params decode")
    else {
        panic!("expected workflow run params");
    };
    assert_eq!(params.id.0, "workflow_123");
    assert_eq!(params.args[0].name, "name");
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
        "input.accepted_command.run",
        "agent.prompt.submit",
        "drive.workflow.submit_external",
    ] {
        assert!(serde_json::from_value::<ActionKind>(serde_json::json!(action)).is_err());
    }
}
#[test]
fn drive_mutation_metadata_is_authenticated_and_implemented() {
    for action in [
        ActionKind::DriveObjectCreate,
        ActionKind::DriveObjectUpdate,
        ActionKind::DriveObjectDelete,
        ActionKind::DriveObjectInsert,
        ActionKind::DriveObjectShareToTeam,
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
        assert!(metadata.authenticated_user.required);
        assert_eq!(
            metadata.allowed_invocation_contexts,
            vec![InvocationContext::InsideWarp]
        );
    }
}

#[test]
fn drive_mutation_audit_payload_serializes_action_and_subject() {
    let payload = DriveMutationResult {
        object: DriveObjectSummary {
            object_type: DriveObjectType::Folder,
            id: DriveObjectId("folder_123".to_owned()),
            name: "Runbooks".to_owned(),
        },
        audit: Some(DriveMutationAudit {
            action: ActionKind::DriveObjectCreate.as_str().to_owned(),
            authenticated_user_subject: "user_123".to_owned(),
        }),
    };
    let value = serde_json::to_value(payload).expect("payload serializes");
    assert_eq!(value["object"]["object_type"], "folder");
    assert_eq!(value["audit"]["action"], "drive.object.create");
    assert_eq!(value["audit"]["authenticated_user_subject"], "user_123");
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
fn implemented_catalog_includes_the_foundation_slice() {
    let actions = ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect::<Vec<_>>();
    for action in [
        ActionKind::InstanceList,
        ActionKind::AppPing,
        ActionKind::AppVersion,
        ActionKind::TabCreate,
    ] {
        assert!(actions.contains(&action));
    }
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
fn logged_out_safe_app_state_actions_can_advertise_external_context() {
    let metadata = ActionKind::WindowCreate.metadata();
    assert_eq!(
        metadata.implementation_status,
        ActionImplementationStatus::Implemented
    );
    assert!(!metadata.authenticated_user.required);
    assert!(
        metadata
            .allowed_invocation_contexts
            .contains(&InvocationContext::OutsideWarp)
    );
}

#[test]
fn readonly_capability_targets_are_implemented() {
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
    ] {
        let metadata = action.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented
        );
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
