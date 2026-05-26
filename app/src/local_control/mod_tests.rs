use ::local_control::protocol::ActionKind;
use ::local_control::protocol::{
    Action, PaneSelector, PaneTarget, TabSelector, TabTarget, TargetSelector, WindowSelector,
    WindowTarget,
};
use ::local_control::{ErrorCode, InvocationContext};
use settings::Setting as _;
use warp_core::features::FeatureFlag;

use super::{
    capabilities, ensure_agent_profile_allows_action, ensure_feature_enabled,
    ensure_settings_allow_action, outside_warp_action_enabled_for_settings,
    require_active_window_id, validate_action_params, validate_tab_create_target,
};
use crate::ai::execution_profiles::{AIExecutionProfile, WarpControlPermission};
use crate::settings::{
    AllowInsideWarpAppStateMutations, AllowInsideWarpControl, AllowInsideWarpMetadataReads,
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
        allow_inside_warp_control: AllowInsideWarpControl::new(Some(true)),
        allow_inside_warp_metadata_reads: AllowInsideWarpMetadataReads::new(Some(true)),
        allow_inside_warp_app_state_mutations: AllowInsideWarpAppStateMutations::new(Some(true)),
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

fn settings_with_permissions(
    outside_enabled: bool,
    metadata_reads: bool,
    underlying_data_reads: bool,
    app_state_mutations: bool,
    metadata_configuration_mutations: bool,
    underlying_data_mutations: bool,
) -> LocalControlSettings {
    LocalControlSettings {
        allow_inside_warp_control: AllowInsideWarpControl::new(Some(true)),
        allow_inside_warp_metadata_reads: AllowInsideWarpMetadataReads::new(Some(true)),
        allow_inside_warp_app_state_mutations: AllowInsideWarpAppStateMutations::new(Some(true)),
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(outside_enabled)),
        allow_outside_warp_metadata_reads: AllowOutsideWarpMetadataReads::new(Some(metadata_reads)),
        allow_outside_warp_underlying_data_reads: AllowOutsideWarpUnderlyingDataReads::new(Some(
            underlying_data_reads,
        )),
        allow_outside_warp_app_state_mutations: AllowOutsideWarpAppStateMutations::new(Some(
            app_state_mutations,
        )),
        allow_outside_warp_metadata_configuration_mutations:
            AllowOutsideWarpMetadataConfigurationMutations::new(Some(
                metadata_configuration_mutations,
            )),
        allow_outside_warp_underlying_data_mutations: AllowOutsideWarpUnderlyingDataMutations::new(
            Some(underlying_data_mutations),
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
        ..TargetSelector::default()
    })
    .expect("active target is accepted");
}

#[test]
fn tab_create_rejects_concrete_targets() {
    let err = validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        ..TargetSelector::default()
    })
    .expect_err("concrete window target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        ..TargetSelector::default()
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        pane: Some(PaneTarget::Id {
            id: PaneSelector("pane".to_owned()),
        }),
        ..TargetSelector::default()
    })
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn tab_create_rejects_unsupported_selector_forms() {
    let err = validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Index { index: 0 }),
        ..TargetSelector::default()
    })
    .expect_err("indexed window target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_tab_create_target(&TargetSelector {
        tab: Some(TabTarget::Index { index: 0 }),
        ..TargetSelector::default()
    })
    .expect_err("indexed tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn capabilities_advertises_current_implemented_actions() {
    let capabilities = capabilities();

    assert!(capabilities.contains(&ActionKind::InstanceList));
    assert!(capabilities.contains(&ActionKind::AppPing));
    assert!(capabilities.contains(&ActionKind::AppVersion));
    assert!(capabilities.contains(&ActionKind::TabCreate));
    assert!(capabilities.contains(&ActionKind::InputRun));
    assert!(capabilities.contains(&ActionKind::DriveObjectCreate));
    assert!(capabilities.contains(&ActionKind::DriveObjectUpdate));
    assert!(capabilities.contains(&ActionKind::DriveObjectDelete));
    assert!(capabilities.contains(&ActionKind::DriveObjectInsert));
    assert!(capabilities.contains(&ActionKind::DriveObjectShareToTeam));
    assert!(!capabilities.contains(&ActionKind::DriveWorkflowRun));
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
fn inside_warp_context_allows_implemented_local_action_categories() {
    let settings = settings_with_values(true, true, true);

    ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect("inside-Warp app-state mutation grants are allowed by default");
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
fn permission_categories_map_to_the_corresponding_setting() {
    let settings = settings_with_permissions(true, true, false, true, false, true);

    ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::AppPing,
    )
    .expect("metadata reads are enabled");
    ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect("app-state mutations are enabled");
    ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::InputRun,
    )
    .expect("underlying data mutations are enabled");

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::SettingSet,
    )
    .expect_err("metadata configuration mutations are disabled");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);

    let disabled_metadata = settings_with_permissions(true, false, true, true, true, true);
    let err = ensure_settings_allow_action(
        &disabled_metadata,
        InvocationContext::OutsideWarp,
        ActionKind::AppPing,
    )
    .expect_err("metadata reads are disabled");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn permission_denial_is_checked_before_selector_resolution() {
    let settings = settings_with_values(true, true, false);
    let target = TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("stale_window".to_owned()),
        }),
        ..TargetSelector::default()
    };

    let permission_error = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("permission is denied before target validation");
    assert_eq!(permission_error.code, ErrorCode::InsufficientPermissions);

    let selector_error = validate_tab_create_target(&target).expect_err("selector is stale");
    assert_eq!(selector_error.code, ErrorCode::StaleTarget);
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

#[test]
fn agent_profile_helper_denies_disabled_warp_control_category() {
    let profile = AIExecutionProfile {
        warp_control_app_state_mutations: WarpControlPermission::NeverAllow,
        ..Default::default()
    };

    let err = ensure_agent_profile_allows_action(&profile, ActionKind::TabCreate)
        .expect_err("profile denies app-state mutations");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn agent_profile_helper_allows_non_denied_warp_control_category() {
    let profile = AIExecutionProfile {
        warp_control_app_state_mutations: WarpControlPermission::AlwaysAsk,
        ..Default::default()
    };

    ensure_agent_profile_allows_action(&profile, ActionKind::TabCreate)
        .expect("profile permits app-state mutation requests");
}
