use ::local_control::protocol::ActionKind;
use ::local_control::protocol::{
    Action, ActionParams, DriveObjectId, PaneSelector, PaneTarget, TabSelector, TabTarget,
    RequestEnvelope, TargetSelector, WindowSelector, WindowTarget, WorkflowArgument,
    WorkflowRunParams,
};
use ::local_control::{ErrorCode, InvocationContext, PermissionCategory};
use settings::Setting as _;
use warp_core::features::FeatureFlag;

use super::{
    capabilities, ensure_feature_enabled, ensure_settings_allow_action,
    outside_warp_action_enabled_for_settings, require_active_window_id, validate_action_params,
    validate_tab_create_target,
};
use crate::local_control::handlers::execution;
use crate::settings::{
    AllowOutsideWarpAppStateMutations, AllowOutsideWarpControl,
    AllowOutsideWarpMetadataConfigurationMutations, AllowOutsideWarpMetadataReads,
    AllowOutsideWarpUnderlyingDataMutations, AllowOutsideWarpUnderlyingDataReads,
    LocalControlSettings,
};

fn settings_with_values(
    outside_enabled: bool,
    outside_metadata_reads: bool,
    outside_app_state_mutations: bool,
) -> LocalControlSettings {
    LocalControlSettings {
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(outside_enabled)),
        allow_outside_warp_metadata_reads: AllowOutsideWarpMetadataReads::new(Some(
            outside_metadata_reads,
        )),
        allow_outside_warp_underlying_data_reads: AllowOutsideWarpUnderlyingDataReads::new(Some(
            false,
        )),
        allow_outside_warp_app_state_mutations: AllowOutsideWarpAppStateMutations::new(Some(
            outside_app_state_mutations,
        )),
        allow_outside_warp_metadata_configuration_mutations:
            AllowOutsideWarpMetadataConfigurationMutations::new(Some(false)),
        allow_outside_warp_underlying_data_mutations: AllowOutsideWarpUnderlyingDataMutations::new(
            Some(false),
        ),
    }
}

fn settings_with_outside_warp(
    outside_control: bool,
    outside_app_state_mutations: bool,
) -> LocalControlSettings {
    settings_with_values(outside_control, false, outside_app_state_mutations)
}

#[test]
fn tab_create_accepts_default_and_active_targets() {
    validate_tab_create_target(&TargetSelector::default()).expect("default target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Active),
        tab: Some(TabTarget::Active),
        pane: Some(PaneTarget::Active),
    })
    .expect("active target is accepted");
}

#[test]
fn tab_create_rejects_concrete_targets() {
    let err = validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        tab: None,
        pane: None,
    })
    .expect_err("concrete window target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        pane: None,
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: None,
        pane: Some(PaneTarget::Id {
            id: PaneSelector("pane".to_owned()),
        }),
    })
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn tab_create_rejects_unsupported_selector_forms() {
    let err = validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Index { index: 0 }),
        tab: None,
        pane: None,
    })
    .expect_err("indexed window target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Index { index: 0 }),
        pane: None,
    })
    .expect_err("indexed tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn capabilities_advertises_only_first_slice_core_actions() {
    assert_eq!(
        capabilities(),
        vec![
            ActionKind::InstanceList,
            ActionKind::AppPing,
            ActionKind::AppVersion,
            ActionKind::TabCreate,
            ActionKind::InputRun,
            ActionKind::DriveWorkflowRun,
        ]
    );
}

#[test]
fn outside_warp_discovery_requires_context_and_action_permission() {
    assert!(!outside_warp_action_enabled_for_settings(
        &settings_with_outside_warp(false, true),
        ActionKind::TabCreate
    ));
    assert!(!outside_warp_action_enabled_for_settings(
        &settings_with_outside_warp(true, false),
        ActionKind::TabCreate
    ));
    assert!(outside_warp_action_enabled_for_settings(
        &settings_with_outside_warp(true, true),
        ActionKind::TabCreate
    ));
}

#[test]
fn tab_create_requires_active_window() {
    let active = warpui::WindowId::from_usize(1);

    assert_eq!(
        require_active_window_id(Some(active)).expect("active"),
        active
    );
    let err = require_active_window_id(None).expect_err("missing active window");
    assert_eq!(err.code, ErrorCode::MissingTarget);
}

#[test]
fn feature_flag_disabled_denies_local_control() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(false);
    let err = ensure_feature_enabled().expect_err("feature flag disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn disabled_outside_warp_denies_before_granular_permission() {
    let settings = settings_with_values(false, true, true);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("outside-Warp parent context is disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn inside_warp_context_is_not_implemented() {
    let settings = settings_with_values(true, true, true);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("inside-Warp grants are not implemented");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
}

#[test]
fn disabled_granular_permission_denies_with_insufficient_permissions() {
    let settings = settings_with_values(true, true, false);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("read-write permission is disabled");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn execution_underlying_actions_require_mutate_underlying_data_permission() {
    let settings = settings_with_values(true, true, true);

    for action in [ActionKind::InputRun, ActionKind::DriveWorkflowRun] {
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::MutateUnderlyingData
        );
        let err = ensure_settings_allow_action(&settings, InvocationContext::OutsideWarp, action)
            .expect_err("underlying data mutation permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn execution_mutations_reject_malformed_params() {
    validate_action_params(
        &Action::with_params(
            ActionKind::InputRun,
            ActionParams::Text {
                text: "cargo check".to_owned(),
            },
        )
        .expect("input.run params serialize"),
    )
    .expect("input.run accepts non-empty text");

    let err = validate_action_params(
        &Action::with_params(
            ActionKind::InputRun,
            ActionParams::Text {
                text: "   ".to_owned(),
            },
        )
        .expect("input.run params serialize"),
    )
    .expect_err("input.run requires non-empty text");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    let err = validate_action_params(
        &Action::with_params(
            ActionKind::DriveWorkflowRun,
            ActionParams::WorkflowRun(WorkflowRunParams {
                id: DriveObjectId("".to_owned()),
                args: vec![],
            }),
        )
        .expect("drive.workflow.run params serialize"),
    )
    .expect_err("drive.workflow.run requires non-empty id");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn drive_workflow_run_rejects_excluded_submission_arguments() {
    for name in ["accepted_command", "accepted-command", "agent_prompt", "agent-prompt"] {
        let err = validate_action_params(
            &Action::with_params(
                ActionKind::DriveWorkflowRun,
                ActionParams::WorkflowRun(WorkflowRunParams {
                    id: DriveObjectId("workflow_123".to_owned()),
                    args: vec![WorkflowArgument {
                        name: name.to_owned(),
                        value: "payload".to_owned(),
                    }],
                }),
            )
            .expect("drive.workflow.run params serialize"),
        )
        .expect_err("excluded submissions are rejected");
        assert_eq!(err.code, ErrorCode::UnsupportedAction);
    }
}

#[test]
fn execution_attempts_fail_closed_without_approval_policy_and_include_audit() {
    let request = RequestEnvelope::new(
        Action::with_params(
            ActionKind::InputRun,
            ActionParams::Text {
                text: "cargo check".to_owned(),
            },
        )
        .expect("input.run params serialize"),
    );
    let err = execution::run_input(&request, "user_123")
        .expect_err("input.run fails closed without policy");
    assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
    let details = err.details.expect("audit details are attached");
    let audit: ::local_control::LocalControlAuditRecord =
        serde_json::from_str(&details).expect("audit decodes");
    assert_eq!(audit.action, "input.run");
    assert_eq!(
        audit.permission_category,
        PermissionCategory::MutateUnderlyingData
    );
    assert_eq!(audit.authenticated_user_subject, "user_123");
}

#[test]
fn tab_create_rejects_malformed_params() {
    let err = validate_action_params(&Action {
        kind: ActionKind::TabCreate,
        params: serde_json::json!({ "unexpected": true }),
    })
    .expect_err("tab.create params must be empty");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    validate_action_params(&Action {
        kind: ActionKind::TabCreate,
        params: serde_json::json!({}),
    })
    .expect("empty tab.create params are accepted");
}
