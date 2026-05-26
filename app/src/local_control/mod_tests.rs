use ::local_control::protocol::ActionKind;
use ::local_control::protocol::{
    Action, PaneSelector, PaneTarget, TabSelector, TabTarget, TargetSelector, WindowSelector,
    WindowTarget,
};
use ::local_control::{ErrorCode, InvocationContext};
use settings::Setting as _;
use warp_core::features::FeatureFlag;

use super::{
    capabilities, ensure_feature_enabled, ensure_settings_allow_action,
    outside_warp_control_enabled_for_settings, require_active_window_id_for_action,
    validate_action_params, validate_tab_create_target,
};
use crate::settings::{LocalControlMode, LocalControlModeSetting, LocalControlSettings};

fn settings_with_mode(mode: LocalControlMode) -> LocalControlSettings {
    LocalControlSettings {
        local_control_mode: LocalControlModeSetting::new(Some(mode)),
    }
}
#[test]
fn tab_create_accepts_default_active_and_window_targets() {
    validate_tab_create_target(&TargetSelector::default()).expect("default target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Active),
        tab: Some(TabTarget::Active),
        pane: Some(PaneTarget::Active),
        session: None,
    })
    .expect("active target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        tab: None,
        pane: None,
        session: None,
    })
    .expect("window id target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Index { index: 0 }),
        tab: None,
        pane: None,
        session: None,
    })
    .expect("window index target is accepted");

    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Title {
            title: "window".to_owned(),
        }),
        tab: None,
        pane: None,
        session: None,
    })
    .expect("window title target is accepted");
}

#[test]
fn tab_create_rejects_concrete_targets() {
    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        pane: None,
        session: None,
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: None,
        pane: Some(PaneTarget::Id {
            id: PaneSelector("pane".to_owned()),
        }),
        session: None,
    })
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn tab_create_rejects_unsupported_selector_forms() {
    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Index { index: 0 }),
        pane: None,
        session: None,
    })
    .expect_err("indexed tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn capabilities_advertises_implemented_readonly_app_state_metadata_config_and_drive_actions() {
    let actions = capabilities();

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
        ActionKind::BlockList,
        ActionKind::BlockInspect,
        ActionKind::BlockOutput,
        ActionKind::InputGet,
        ActionKind::HistoryList,
        ActionKind::ThemeGet,
        ActionKind::KeybindingList,
        ActionKind::KeybindingGet,
        ActionKind::FileList,
        ActionKind::DriveList,
        ActionKind::DriveInspect,
        ActionKind::InstanceList,
        ActionKind::AppPing,
        ActionKind::AppVersion,
        ActionKind::AppFocus,
        ActionKind::WindowCreate,
        ActionKind::WindowFocus,
        ActionKind::WindowClose,
        ActionKind::TabCreate,
        ActionKind::TabActivate,
        ActionKind::TabMove,
        ActionKind::TabClose,
        ActionKind::TabRename,
        ActionKind::TabResetName,
        ActionKind::TabColorSet,
        ActionKind::TabColorClear,
        ActionKind::PaneSplit,
        ActionKind::PaneFocus,
        ActionKind::PaneNavigate,
        ActionKind::PaneResize,
        ActionKind::PaneMaximize,
        ActionKind::PaneUnmaximize,
        ActionKind::PaneClose,
        ActionKind::PaneRename,
        ActionKind::PaneResetName,
        ActionKind::SessionActivate,
        ActionKind::SessionPrevious,
        ActionKind::SessionNext,
        ActionKind::SessionReopenClosed,
        ActionKind::InputInsert,
        ActionKind::InputReplace,
        ActionKind::InputClear,
        ActionKind::InputModeSet,
        ActionKind::InputRun,
        ActionKind::ThemeSet,
        ActionKind::ThemeSystemSet,
        ActionKind::ThemeLightSet,
        ActionKind::ThemeDarkSet,
        ActionKind::AppearanceFontSizeIncrease,
        ActionKind::AppearanceFontSizeDecrease,
        ActionKind::AppearanceFontSizeReset,
        ActionKind::AppearanceZoomIncrease,
        ActionKind::AppearanceZoomDecrease,
        ActionKind::AppearanceZoomReset,
        ActionKind::SettingSet,
        ActionKind::SettingToggle,
        ActionKind::SurfaceSettingsOpen,
        ActionKind::SurfaceCommandPaletteOpen,
        ActionKind::SurfaceCommandSearchOpen,
        ActionKind::SurfaceWarpDriveOpen,
        ActionKind::SurfaceWarpDriveToggle,
        ActionKind::SurfaceResourceCenterToggle,
        ActionKind::SurfaceAiAssistantToggle,
        ActionKind::SurfaceCodeReviewToggle,
        ActionKind::SurfaceLeftPanelToggle,
        ActionKind::SurfaceRightPanelToggle,
        ActionKind::SurfaceVerticalTabsToggle,
        ActionKind::FileOpen,
        ActionKind::DriveOpen,
        ActionKind::DriveNotebookOpen,
        ActionKind::DriveEnvVarCollectionOpen,
        ActionKind::DriveObjectShareOpen,
        ActionKind::DriveObjectCreate,
        ActionKind::DriveObjectUpdate,
        ActionKind::DriveObjectDelete,
        ActionKind::DriveObjectInsert,
        ActionKind::DriveObjectShareToTeam,
        ActionKind::DriveWorkflowRun,
    ] {
        assert!(actions.contains(&action), "missing {}", action.as_str());
    }
}

#[test]
fn outside_warp_discovery_requires_everywhere_mode() {
    assert!(!outside_warp_control_enabled_for_settings(
        &settings_with_mode(LocalControlMode::Disabled)
    ));
    assert!(!outside_warp_control_enabled_for_settings(
        &settings_with_mode(LocalControlMode::EnabledWithinWarp)
    ));
    assert!(outside_warp_control_enabled_for_settings(
        &settings_with_mode(LocalControlMode::EnabledEverywhere)
    ));
}

#[test]
fn tab_create_requires_active_window() {
    let active = warpui::WindowId::from_usize(1);

    assert_eq!(
        require_active_window_id_for_action(Some(active), ActionKind::TabCreate).expect("active"),
        active
    );
    let err = require_active_window_id_for_action(None, ActionKind::TabCreate)
        .expect_err("missing active window");
    assert_eq!(err.code, ErrorCode::MissingTarget);
}

#[test]
fn active_window_errors_use_requested_action_name() {
    let err = require_active_window_id_for_action(None, ActionKind::TabActivate)
        .expect_err("missing active window");
    assert_eq!(err.code, ErrorCode::MissingTarget);
    assert!(err.message.contains("tab.activate"));
    assert!(!err.message.contains("tab.create"));
}

#[test]
fn feature_flag_disabled_denies_local_control() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(false);
    let err = ensure_feature_enabled().expect_err("feature flag disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn outside_warp_requires_everywhere_mode() {
    let settings = settings_with_mode(LocalControlMode::EnabledWithinWarp);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("outside-Warp local control is disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn enabled_within_warp_allows_inside_warp_context() {
    ensure_settings_allow_action(
        &settings_with_mode(LocalControlMode::EnabledWithinWarp),
        InvocationContext::InsideWarp,
        ActionKind::InputRun,
    )
    .expect("inside-Warp local control is enabled");
}

#[test]
fn outside_warp_authenticated_actions_are_execution_context_denied() {
    let settings = settings_with_mode(LocalControlMode::EnabledEverywhere);
    for action in [ActionKind::InputRun, ActionKind::DriveObjectCreate] {
        let err = ensure_settings_allow_action(&settings, InvocationContext::OutsideWarp, action)
            .expect_err("outside-Warp authenticated action is rejected");
        assert_eq!(err.code, ErrorCode::ExecutionContextNotAllowed);
    }
}

#[test]
fn disabled_mode_denies_inside_warp_context() {
    let settings = settings_with_mode(LocalControlMode::Disabled);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("inside-Warp local control is disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn enabled_everywhere_allows_outside_warp_context() {
    ensure_settings_allow_action(
        &settings_with_mode(LocalControlMode::EnabledEverywhere),
        InvocationContext::OutsideWarp,
        ActionKind::TabCreate,
    )
    .expect("outside-Warp local control is enabled");
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
