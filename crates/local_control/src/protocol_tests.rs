use super::*;

#[test]
fn request_envelope_serializes_stable_action_names() {
    let request = RequestEnvelope::new(Action::new(ActionKind::WindowFocus));
    let value = serde_json::to_value(&request).expect("request serializes");
    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(value["action"]["kind"], "window.focus");
}

#[test]
fn strict_params_serialize_without_synthetic_discriminators() {
    let action = Action::with_params(
        ActionKind::SettingList,
        SettingListParams {
            namespace: Some("editor".to_owned()),
        },
    )
    .expect("setting.list params serialize");
    assert_eq!(action.params, serde_json::json!({ "namespace": "editor" }));
    let params = action
        .params_as::<SettingListParams>()
        .expect("setting.list params decode");
    assert_eq!(params.namespace.as_deref(), Some("editor"));

    let action = Action::with_params(
        ActionKind::TabCreate,
        TabCreateParams {
            tab_type: Some(TabType::Agent),
            shell: Some("zsh".to_owned()),
        },
    )
    .expect("tab.create params serialize");
    assert_eq!(
        action.params,
        serde_json::json!({ "tab_type": "agent", "shell": "zsh" })
    );
    assert!(action.params.get("type").is_none());
}

#[test]
fn strict_params_deny_unknown_fields() {
    let action = Action {
        kind: ActionKind::InputInsert,
        params: serde_json::json!({ "text": "hello", "submit": true }),
    };
    let error = action
        .params_as::<TextParams>()
        .expect_err("unknown params are rejected");
    assert_eq!(error.code, ErrorCode::InvalidParams);

    let action = Action {
        kind: ActionKind::WindowFocus,
        params: serde_json::json!({ "unexpected": true }),
    };
    assert!(action.params_as::<EmptyParams>().is_err());
}

#[test]
fn target_selector_roundtrips_exact_session_id() {
    let target = TargetSelector {
        session: Some(SessionTarget::Id {
            id: SessionSelector("session_1".to_owned()),
        }),
        ..TargetSelector::default()
    };
    let value = serde_json::to_value(&target).expect("target serializes");
    assert_eq!(
        value["session"],
        serde_json::json!({ "type": "id", "id": "session_1" })
    );
    assert_eq!(
        serde_json::from_value::<TargetSelector>(value).expect("target decodes"),
        target
    );
}

#[test]
fn response_error_serializes_machine_code() {
    let response = ResponseEnvelope::error(
        Uuid::nil(),
        ControlError::new(ErrorCode::InsufficientPermissions, "wrong action"),
    );
    let value = serde_json::to_value(&response).expect("response serializes");
    assert_eq!(value["response"]["status"], "error");
    assert_eq!(
        value["response"]["error"]["code"],
        "insufficient_permissions"
    );
}

#[test]
fn surface_list_result_serializes_stable_availability_shape() {
    let result = SurfaceListResult {
        surfaces: vec![
            SurfaceSummary {
                name: "theme_picker".to_owned(),
                is_available: true,
                unavailable_reason: None,
            },
            SurfaceSummary {
                name: "vertical_tabs".to_owned(),
                is_available: false,
                unavailable_reason: Some("vertical tabs are disabled".to_owned()),
            },
        ],
    };
    let value = serde_json::to_value(result).expect("surface list result serializes");
    assert_eq!(
        value,
        serde_json::json!({
            "surfaces": [
                {
                    "name": "theme_picker",
                    "is_available": true
                },
                {
                    "name": "vertical_tabs",
                    "is_available": false,
                    "unavailable_reason": "vertical tabs are disabled"
                }
            ]
        })
    );
}

#[test]
fn malformed_and_removed_action_names_are_not_deserialized() {
    for action in [
        "tab.create.extra",
        "auth.status",
        "auth.login",
        "block.list",
        "block.inspect",
        "block.output",
        "history.list",
        "file.list",
        "input.get",
        "input.clear",
        "input.mode.set",
        "input.run",
        "drive.list",
        "drive.inspect",
        "drive.open",
        "drive.notebook.open",
        "drive.env_var_collection.open",
        "drive.object.share.open",
        "drive.object.create",
        "drive.object.update",
        "drive.object.delete",
        "drive.object.insert",
        "drive.object.share_to_team",
        "drive.workflow.run",
    ] {
        assert!(serde_json::from_value::<ActionKind>(serde_json::json!(action)).is_err());
    }
}

#[test]
fn catalog_has_exactly_84_retained_actions() {
    assert_eq!(ActionKind::ALL.len(), 84);
}

#[test]
fn direct_surface_actions_have_stable_names() {
    assert_eq!(ActionKind::SurfaceList.as_str(), "surface.list");
    assert_eq!(
        ActionKind::SurfaceThemePickerOpen.as_str(),
        "surface.theme_picker.open"
    );
    assert_eq!(
        ActionKind::SurfaceKeybindingsOpen.as_str(),
        "surface.keybindings.open"
    );
    assert_eq!(
        ActionKind::SurfaceCodeReviewOpen.as_str(),
        "surface.code_review.open"
    );
    assert_eq!(
        ActionKind::SurfaceProjectExplorerOpen.as_str(),
        "surface.project_explorer.open"
    );
    assert_eq!(
        ActionKind::SurfaceGlobalSearchOpen.as_str(),
        "surface.global_search.open"
    );
    assert_eq!(
        ActionKind::SurfaceConversationListOpen.as_str(),
        "surface.conversation_list.open"
    );
    assert_eq!(
        ActionKind::SurfaceVerticalTabsOpen.as_str(),
        "surface.vertical_tabs.open"
    );
    assert_eq!(
        ActionKind::SurfaceAgentManagementOpen.as_str(),
        "surface.agent_management.open"
    );
}

#[test]
fn catalog_actions_share_uniform_authorization() {
    for kind in ActionKind::ALL {
        let metadata = kind.metadata();
        assert_eq!(
            metadata.implementation_status,
            ActionImplementationStatus::Implemented,
            "{} should be implemented",
            metadata.name,
        );
    }
}

#[test]
fn implemented_catalog_contains_all_retained_actions() {
    let actions = ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect::<Vec<_>>();
    assert_eq!(actions, ActionKind::ALL);
}
