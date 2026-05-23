use crate::terminal::model::block::BlockMetadata;
use ::local_control::auth::CredentialGrant;
use ::local_control::protocol::ActionKind;
use ::local_control::protocol::{
    Action, AppFocusParams, AppSurfaceParams, AppearanceFontSizeParams, AppearanceSetParams,
    AppearanceZoomParams, BlockGetParams, BlockListParams, BlockTarget, ControlResponse,
    DriveCreateParams, DriveDeleteParams, DriveGetParams, DriveGetResult, DriveInsertParams,
    DriveListParams, DriveListResult, DriveMutationResult, DriveObjectSelector, DriveObjectType,
    DriveRunParams, DriveTarget, DriveUpdateParams, FileDeleteParams, FileMutationResult,
    FileOpenParams, FileSelector, FileTarget, FileWriteParams, HorizontalDirection,
    InputClearParams, InputInsertParams, InputMode, InputModeSetParams, InputReplaceParams,
    InputRunParams, InputStateResult, PaneCloseParams, PaneDirection, PaneFocusParams,
    PaneMaximizeParams, PaneNavigateParams, PaneResizeParams, PaneSelector, PaneSplitParams,
    PaneTarget, SessionSelector, SessionTarget, SettingSetParams, SettingToggleParams,
    SizeAdjustment, TabActivateParams, TabActivationTarget, TabCloseParams, TabCloseScope,
    TabMoveParams, TabMutationResult, TabRenameParams, TabSelector, TabTarget, TargetSelector,
    ThemeSetParams, WindowCloseParams, WindowCreateParams, WindowFocusParams, WindowSelector,
    WindowTarget,
};
use ::local_control::{
    ErrorCode, InstanceId, InvocationContext, PermissionCategory, RequestEnvelope,
};
use chrono::Duration;
use lsp::LspManagerModel;
use settings::Setting as _;
use std::fs;
use std::path::Path;
use warp_core::features::FeatureFlag;
use warp_core::session_id::SessionId;
use warpui::platform::WindowStyle;
use warpui::windowing::{state::ApplicationStage, WindowManager};
use warpui::{App, SingletonEntity, TypedActionView};

use super::allow_input_run_policy_for_test;
use super::{
    action_metadata_for_name, appearance_font_size_result, appearance_set_result,
    appearance_state_result, appearance_zoom_result, authenticated_user_subject_for_action,
    block_get_result_from_model, block_list_result_from_model, capabilities,
    ensure_feature_enabled, ensure_input_run_policy_allows, ensure_settings_allow_action,
    outside_warp_action_enabled_for_settings, rejected_setting_key, require_active_window_id,
    require_active_window_id_for_action, resolve_file_mutation_path,
    select_window_for_app_state_target, setting_get_result, setting_list_result,
    setting_set_result, setting_toggle_result, theme_list_result, theme_set_result,
    validate_action_params, validate_active_terminal_target, validate_app_focus_target,
    validate_block_get_target, validate_block_list_target, validate_drive_target,
    validate_file_mutation_target, validate_instance_metadata_read_target,
    validate_tab_create_target, validate_tab_rename_target, validate_terminal_read_target,
    validate_window_create_target, workspace_action_for_surface, LocalControlBridge,
};
use crate::ai::blocklist::{InputConfig, InputType};
use crate::ai::facts::manager::AIFactManager;
use crate::ai::mcp::{FileBasedMCPManager, FileMCPWatcher};
use crate::appearance::AppearanceManager;
use crate::auth::AuthStateProvider;
use crate::autoupdate::{AutoupdateState, RelaunchModel};
use crate::cloud_object::model::{
    actions::ObjectActions, persistence::CloudModel, view::CloudViewModel,
};
use crate::cloud_object::Owner;
#[cfg(enable_crash_recovery)]
use crate::crash_recovery::CrashRecovery;
use crate::default_terminal::DefaultTerminal;
use crate::drive::folders::{CloudFolder, CloudFolderModel};
use crate::env_vars::{
    CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVar, EnvVarCollection,
};
use crate::gpu_state::GPUState;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::{CloudNotebook, CloudNotebookModel};
use crate::projects::ProjectManagementModel;
#[cfg(not(target_family = "wasm"))]
use crate::remote_server::codebase_index_model::RemoteCodebaseIndexModel;
use crate::remote_server::manager::RemoteServerManager;
use crate::root_view::NewWorkspaceSource;
use crate::server::ids::{ClientId, SyncId};
use crate::settings::{
    AllowInsideWarpControl, AllowInsideWarpReadOnly, AllowInsideWarpReadWrite,
    AllowOutsideWarpControl, AllowOutsideWarpReadOnly, AllowOutsideWarpReadWrite,
    LocalControlSettings,
};
use crate::settings_view::pane_manager::SettingsPaneManager;
use crate::settings_view::DisplayCount;
#[cfg(feature = "local_tty")]
use crate::terminal::available_shells;
use crate::terminal::model::TerminalModel;
use crate::terminal::shared_session::manager::Manager as SharedSessionManager;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workflows::{workflow::Workflow, CloudWorkflow, CloudWorkflowModel};
use crate::workspace::{
    bonus_grant_notification_model::BonusGrantNotificationModel,
    cross_window_tab_drag::CrossWindowTabDrag, ToastStack as WorkspaceToastStack, Workspace,
    WorkspaceAction,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::GlobalResourceHandles;
use ai::project_context::model::ProjectContextModel;

fn settings_with_values(
    inside_enabled: bool,
    outside_enabled: bool,
    inside_read_only: bool,
    outside_read_only: bool,
    inside_read_write: bool,
    outside_read_write: bool,
) -> LocalControlSettings {
    LocalControlSettings {
        allow_inside_warp_control: AllowInsideWarpControl::new(Some(inside_enabled)),
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(outside_enabled)),
        allow_inside_warp_read_only: AllowInsideWarpReadOnly::new(Some(inside_read_only)),
        allow_outside_warp_read_only: AllowOutsideWarpReadOnly::new(Some(outside_read_only)),
        allow_inside_warp_read_write: AllowInsideWarpReadWrite::new(Some(inside_read_write)),
        allow_outside_warp_read_write: AllowOutsideWarpReadWrite::new(Some(outside_read_write)),
    }
}

#[test]
fn tab_rename_accepts_default_active_and_concrete_tab_targets() {
    validate_tab_rename_target(&TargetSelector::default()).expect("default target is accepted");

    validate_tab_rename_target(&TargetSelector {
        window: Some(WindowTarget::Active),
        tab: Some(TabTarget::Active),
        ..TargetSelector::default()
    })
    .expect("active target is accepted");

    validate_tab_rename_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        ..TargetSelector::default()
    })
    .expect("concrete tab target is accepted");
}

#[test]
fn tab_rename_rejects_unsupported_target_families() {
    for target in [
        TargetSelector {
            pane: Some(PaneTarget::Active),
            ..TargetSelector::default()
        },
        TargetSelector {
            session: Some(SessionTarget::Active),
            ..TargetSelector::default()
        },
        TargetSelector {
            file: Some(FileTarget::Path {
                path: "notes.txt".to_owned(),
            }),
            ..TargetSelector::default()
        },
        TargetSelector {
            drive: Some(DriveTarget::Id {
                object_type: DriveObjectType::Notebook,
                id: DriveObjectSelector("notebook".to_owned()),
            }),
            ..TargetSelector::default()
        },
    ] {
        let err = validate_tab_rename_target(&target).expect_err("unsupported target is rejected");
        assert_eq!(err.code, ErrorCode::InvalidSelector);
    }
}

fn settings_with_outside_warp(
    outside_control: bool,
    outside_read_write: bool,
) -> LocalControlSettings {
    settings_with_values(true, outside_control, true, false, true, outside_read_write)
}

fn settings_with_outside_warp_read_only(
    outside_control: bool,
    outside_read_only: bool,
) -> LocalControlSettings {
    settings_with_values(true, outside_control, true, outside_read_only, true, false)
}

fn settings_with_outside_warp_permissions(
    outside_control: bool,
    outside_read_only: bool,
    outside_read_write: bool,
) -> LocalControlSettings {
    settings_with_values(
        true,
        outside_control,
        true,
        outside_read_only,
        true,
        outside_read_write,
    )
}

fn grant_for(action: ActionKind) -> CredentialGrant {
    CredentialGrant::new(
        InstanceId("test-instance".to_owned()),
        action,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    )
}

fn request_with_target(action: ActionKind, target: TargetSelector) -> RequestEnvelope {
    let mut request = RequestEnvelope::new(Action::new(action));
    request.target = target;
    request
}

fn action_for_app_state_mutation(action: ActionKind) -> Action {
    match action {
        ActionKind::TabActivate => Action::new(action),
        ActionKind::TabMove => Action::with_params(
            action,
            TabMoveParams {
                direction: HorizontalDirection::Left,
            },
        )
        .expect("tab.move params serialize"),
        ActionKind::TabClose => Action::new(action),
        ActionKind::PaneSplit => Action::with_params(
            action,
            PaneSplitParams {
                direction: PaneDirection::Right,
                profile: None,
            },
        )
        .expect("pane.split params serialize"),
        ActionKind::PaneFocus => Action::new(action),
        ActionKind::PaneNavigate => Action::with_params(
            action,
            PaneNavigateParams {
                direction: PaneDirection::Left,
            },
        )
        .expect("pane.navigate params serialize"),
        ActionKind::PaneClose => Action::new(action),
        ActionKind::PaneMaximize => Action::new(action),
        ActionKind::PaneResize => Action::with_params(
            action,
            PaneResizeParams {
                direction: PaneDirection::Right,
                amount: Some(1),
            },
        )
        .expect("pane.resize params serialize"),
        _ => panic!("unexpected app-state mutation action"),
    }
}

fn request_with_action_and_target(action: Action, target: TargetSelector) -> RequestEnvelope {
    let mut request = RequestEnvelope::new(action);
    request.target = target;
    request
}

fn owned_app_state_actions() -> [ActionKind; 9] {
    [
        ActionKind::TabActivate,
        ActionKind::TabMove,
        ActionKind::TabClose,
        ActionKind::PaneSplit,
        ActionKind::PaneFocus,
        ActionKind::PaneNavigate,
        ActionKind::PaneClose,
        ActionKind::PaneMaximize,
        ActionKind::PaneResize,
    ]
}
fn request_with_params<T: serde::Serialize>(action: ActionKind, params: T) -> RequestEnvelope {
    RequestEnvelope::new(Action::with_params(action, params).expect("params serialize"))
}

fn initialize_drive_app(app: &mut App, logged_in: bool) {
    initialize_settings_for_tests(app);
    if logged_in {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    } else {
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
    }
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(LocalControlBridge::new);
}
fn initialize_file_mutation_app(app: &mut App, root: &Path, logged_in: bool) {
    initialize_settings_for_tests(app);
    if logged_in {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    } else {
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
    }
    let root = root.to_path_buf();
    app.add_singleton_model(move |ctx| {
        let mut model = ProjectManagementModel::new(Vec::new(), None, ctx);
        model.upsert_project(root, ctx);
        model
    });
    app.add_singleton_model(LocalControlBridge::new);
}

fn initialize_app_state_app(app: &mut App, logged_in: bool) {
    initialize_settings_for_tests(app);
    if logged_in {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    } else {
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
    }
    app.add_singleton_model(LocalControlBridge::new);
}

fn create_workflow(app: &mut App, name: &str, command: &str) -> String {
    CloudModel::handle(app).update(app, |cloud_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let uid = sync_id.uid();
        cloud_model.create_object(
            sync_id,
            CloudWorkflow::new_local(
                CloudWorkflowModel::new(Workflow::new(name, command)),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        uid
    })
}

fn create_notebook(app: &mut App, title: &str, data: &str) -> String {
    CloudModel::handle(app).update(app, |cloud_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let uid = sync_id.uid();
        cloud_model.create_object(
            sync_id,
            CloudNotebook::new_local(
                CloudNotebookModel {
                    title: title.to_owned(),
                    data: data.to_owned(),
                    ..CloudNotebookModel::default()
                },
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        uid
    })
}

fn create_environment(app: &mut App, title: &str) -> String {
    CloudModel::handle(app).update(app, |cloud_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let uid = sync_id.uid();
        cloud_model.create_object(
            sync_id,
            CloudEnvVarCollection::new_local(
                CloudEnvVarCollectionModel::new(EnvVarCollection::new(
                    Some(title.to_owned()),
                    None,
                    vec![EnvVar::new("PORT".to_owned(), "4000".to_owned(), None)],
                )),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        uid
    })
}

fn create_folder(app: &mut App, name: &str) -> String {
    CloudModel::handle(app).update(app, |cloud_model, ctx| {
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let uid = sync_id.uid();
        cloud_model.create_object(
            sync_id,
            CloudFolder::new_local(
                CloudFolderModel::new(name, false),
                Owner::mock_current_user(),
                None,
                client_id,
            ),
            ctx,
        );
        uid
    })
}

fn authenticated_grant(
    action: ActionKind,
    ctx: &mut warpui::ModelContext<LocalControlBridge>,
) -> CredentialGrant {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        action,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = authenticated_user_subject_for_action(action, ctx)
        .expect("authenticated subject check succeeds");
    grant
}

fn spoofed_authenticated_grant(action: ActionKind) -> CredentialGrant {
    let mut grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        action,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = Some("spoofed-user".to_owned());
    grant
}

fn response_error_code(response: ::local_control::ResponseEnvelope) -> ErrorCode {
    let ControlResponse::Error { error } = response.response else {
        panic!("expected error response");
    };
    error.code
}

fn response_drive_mutation(response: ::local_control::ResponseEnvelope) -> DriveMutationResult {
    let ControlResponse::Ok { data } = response.response else {
        panic!("expected ok response");
    };
    serde_json::from_value(data).expect("drive mutation result decodes")
}
fn response_ok_data(response: ::local_control::ResponseEnvelope) -> serde_json::Value {
    match response.response {
        ControlResponse::Ok { data } => data,
        ControlResponse::Error { error } => panic!("expected ok response, got {error}"),
    }
}

fn with_local_control_bridge(
    test: impl FnOnce(&mut LocalControlBridge, &mut warpui::ModelContext<LocalControlBridge>) + 'static,
) {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(LocalControlBridge::new);
        LocalControlBridge::handle(&app).update(&mut app, test);
    });
}

#[test]
fn tab_move_success_moves_target_tab_right() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let (window_id, workspace) = initialize_app_with_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::AddTerminalTab {
                    hide_homepage: false,
                },
                ctx,
            );
        });
        let original_order = workspace.read(&app, |workspace, _| {
            workspace
                .tab_views()
                .map(|pane_group| pane_group.id())
                .collect::<Vec<_>>()
        });
        let request = request_with_action_and_target(
            Action::with_params(
                ActionKind::TabMove,
                TabMoveParams {
                    direction: HorizontalDirection::Right,
                },
            )
            .expect("tab.move params serialize"),
            TargetSelector {
                tab: Some(TabTarget::Index { index: 0 }),
                ..window_target(window_id)
            },
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response =
                bridge.handle_request(request, authenticated_grant(ActionKind::TabMove, ctx), ctx);
            let result: TabMutationResult = serde_json::from_value(ok_response_data(response))
                .expect("tab mutation result decodes");
            let new_order = workspace.read(ctx, |workspace, _| {
                workspace
                    .tab_views()
                    .map(|pane_group| pane_group.id())
                    .collect::<Vec<_>>()
            });
            assert_eq!(new_order[1], original_order[0]);
            assert_eq!(result.tab_id, original_order[0].to_string());
            assert_eq!(result.window_id, window_id.to_string());
        });
    });
}

#[test]
fn pane_split_and_focus_success_updates_target_pane() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let (window_id, workspace) = initialize_app_with_workspace(&mut app);
        let split_request = request_with_action_and_target(
            Action::with_params(
                ActionKind::PaneSplit,
                PaneSplitParams {
                    direction: PaneDirection::Right,
                    profile: None,
                },
            )
            .expect("pane.split params serialize"),
            window_target(window_id),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                split_request,
                authenticated_grant(ActionKind::PaneSplit, ctx),
                ctx,
            );
            let split_result: ::local_control::protocol::PaneMutationResult =
                serde_json::from_value(ok_response_data(response))
                    .expect("pane mutation result decodes");
            let pane_group = workspace.read(ctx, |workspace, _| {
                workspace.active_tab_pane_group().clone()
            });
            let pane_ids = pane_group.read(ctx, |pane_group, _| pane_group.visible_pane_ids());
            assert_eq!(pane_ids.len(), 2);
            assert!(pane_ids
                .iter()
                .any(|pane_id| pane_id.to_string() == split_result.pane_id));

            let target_pane_id = pane_ids[0];
            let focus_request = request_with_target(
                ActionKind::PaneFocus,
                TargetSelector {
                    pane: Some(PaneTarget::Index { index: 0 }),
                    ..window_target(window_id)
                },
            );
            let response = bridge.handle_request(
                focus_request,
                authenticated_grant(ActionKind::PaneFocus, ctx),
                ctx,
            );
            let focus_result: ::local_control::protocol::PaneMutationResult =
                serde_json::from_value(ok_response_data(response))
                    .expect("pane mutation result decodes");
            let focused_pane_id =
                pane_group.read(ctx, |pane_group, ctx| pane_group.focused_pane_id(ctx));
            assert_eq!(focused_pane_id, target_pane_id);
            assert_eq!(focus_result.pane_id, target_pane_id.to_string());
            assert_eq!(focus_result.tab_id, split_result.tab_id);
        });
    });
}

#[test]
fn owned_app_state_actions_require_mutate_app_state_permission() {
    let settings_without_app_state_mutation =
        settings_with_values(true, true, true, true, false, false);

    for action in owned_app_state_actions() {
        assert_eq!(
            action.metadata().implementation_status,
            ::local_control::ActionImplementationStatus::Implemented
        );
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::MutateAppState
        );
        assert!(action.metadata().requires_authenticated_user);
        let err = ensure_settings_allow_action(
            &settings_without_app_state_mutation,
            InvocationContext::InsideWarp,
            action,
        )
        .expect_err("app-state mutation permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn tab_app_state_mutations_reject_stale_tab_ids() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let (window_id, _) = initialize_app_with_workspace(&mut app);
        for action in [
            ActionKind::TabActivate,
            ActionKind::TabMove,
            ActionKind::TabClose,
        ] {
            let request = request_with_action_and_target(
                action_for_app_state_mutation(action),
                TargetSelector {
                    tab: Some(TabTarget::Id {
                        id: TabSelector("stale-tab".to_owned()),
                    }),
                    ..window_target(window_id)
                },
            );
            LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
                let response =
                    bridge.handle_request(request, authenticated_grant(action, ctx), ctx);
                assert_eq!(response_error_code(response), ErrorCode::StaleTarget);
            });
        }
    });
}

#[test]
fn pane_app_state_mutations_reject_stale_pane_ids() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let (window_id, _) = initialize_app_with_workspace(&mut app);
        for action in [
            ActionKind::PaneSplit,
            ActionKind::PaneFocus,
            ActionKind::PaneNavigate,
            ActionKind::PaneClose,
            ActionKind::PaneMaximize,
            ActionKind::PaneResize,
        ] {
            let request = request_with_action_and_target(
                action_for_app_state_mutation(action),
                TargetSelector {
                    pane: Some(PaneTarget::Id {
                        id: PaneSelector("stale-pane".to_owned()),
                    }),
                    ..window_target(window_id)
                },
            );
            LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
                let response =
                    bridge.handle_request(request, authenticated_grant(action, ctx), ctx);
                assert_eq!(response_error_code(response), ErrorCode::StaleTarget);
            });
        }
    });
}

#[test]
fn app_state_mutations_report_target_state_conflicts() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let (window_id, _) = initialize_app_with_workspace(&mut app);
        let cases = vec![
            request_with_action_and_target(
                Action::with_params(
                    ActionKind::TabMove,
                    TabMoveParams {
                        direction: HorizontalDirection::Left,
                    },
                )
                .expect("tab.move params serialize"),
                TargetSelector {
                    tab: Some(TabTarget::Index { index: 0 }),
                    ..window_target(window_id)
                },
            ),
            request_with_action_and_target(
                Action::with_params(
                    ActionKind::TabClose,
                    TabCloseParams {
                        scope: TabCloseScope::Others,
                        force: false,
                    },
                )
                .expect("tab.close params serialize"),
                TargetSelector {
                    tab: Some(TabTarget::Index { index: 0 }),
                    ..window_target(window_id)
                },
            ),
            request_with_action_and_target(
                Action::with_params(
                    ActionKind::PaneNavigate,
                    PaneNavigateParams {
                        direction: PaneDirection::Left,
                    },
                )
                .expect("pane.navigate params serialize"),
                window_target(window_id),
            ),
            request_with_action_and_target(
                Action::new(ActionKind::PaneMaximize),
                window_target(window_id),
            ),
            request_with_action_and_target(
                Action::with_params(
                    ActionKind::PaneResize,
                    PaneResizeParams {
                        direction: PaneDirection::Right,
                        amount: Some(1),
                    },
                )
                .expect("pane.resize params serialize"),
                window_target(window_id),
            ),
        ];
        for request in cases {
            let action = request.action.kind;
            LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
                let response =
                    bridge.handle_request(request, authenticated_grant(action, ctx), ctx);
                assert_eq!(
                    response_error_code(response),
                    ErrorCode::TargetStateConflict
                );
            });
        }
    });
}

fn initialize_app_with_workspace(
    app: &mut App,
) -> (warpui::WindowId, warpui::ViewHandle<Workspace>) {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(AppearanceManager::new);
    app.add_singleton_model(|ctx| {
        AutoupdateState::new(crate::server::server_api::ServerApiProvider::as_ref(ctx).get())
    });
    app.add_singleton_model(|_| RelaunchModel::new());
    app.add_singleton_model(|_| GPUState::new());
    app.add_singleton_model(|_| DisplayCount::mock());
    app.add_singleton_model(DefaultTerminal::new);
    #[cfg(feature = "local_tty")]
    available_shells::register(app);
    app.add_singleton_model(RemoteServerManager::new);
    #[cfg(not(target_family = "wasm"))]
    app.add_singleton_model(RemoteCodebaseIndexModel::new);
    app.add_singleton_model(|_| LspManagerModel::new());
    app.add_singleton_model(|ctx| ProjectContextModel::new_from_persisted(vec![], ctx));
    app.add_singleton_model(FileMCPWatcher::new);
    app.add_singleton_model(|_| FileBasedMCPManager::default());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(|_| SettingsPaneManager::new());
    app.add_singleton_model(|_| AIFactManager::new());
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(|_| WorkspaceToastStack);
    app.add_singleton_model(SharedSessionManager::new);
    app.add_singleton_model(BonusGrantNotificationModel::new);
    app.add_singleton_model(|_| CrossWindowTabDrag::new());
    #[cfg(enable_crash_recovery)]
    CrashRecovery::register_for_test(app);
    app.add_singleton_model(LocalControlBridge::new);
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        Workspace::new(
            global_resource_handles,
            None,
            NewWorkspaceSource::Empty {
                previous_active_window: None,
                shell: None,
            },
            ctx,
        )
    })
}

fn window_target(window_id: warpui::WindowId) -> TargetSelector {
    TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector(window_id.to_string()),
        }),
        ..TargetSelector::default()
    }
}

fn ok_response_data(response: ::local_control::ResponseEnvelope) -> serde_json::Value {
    let ControlResponse::Ok { data } = response.response else {
        panic!("expected ok response");
    };
    data
}

#[test]
fn tab_activate_success_activates_target_tab() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let (window_id, workspace) = initialize_app_with_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::AddTerminalTab {
                    hide_homepage: false,
                },
                ctx,
            );
        });
        let request = request_with_target(
            ActionKind::TabActivate,
            TargetSelector {
                tab: Some(TabTarget::Index { index: 0 }),
                ..window_target(window_id)
            },
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                request,
                authenticated_grant(ActionKind::TabActivate, ctx),
                ctx,
            );
            let data = ok_response_data(response);
            let result: TabMutationResult =
                serde_json::from_value(data).expect("tab mutation result decodes");
            let active_tab_id = workspace.read(ctx, |workspace, _| {
                assert_eq!(workspace.active_tab_index(), 0);
                workspace.active_tab_pane_group().id().to_string()
            });
            assert_eq!(result.tab_id, active_tab_id);
            assert_eq!(result.window_id, window_id.to_string());
        });
    });
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
        tab: None,
        pane: None,
        ..TargetSelector::default()
    })
    .expect_err("concrete window target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        pane: None,
        ..TargetSelector::default()
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: None,
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
        tab: None,
        pane: None,
        ..TargetSelector::default()
    })
    .expect_err("indexed window target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Index { index: 0 }),
        pane: None,
        ..TargetSelector::default()
    })
    .expect_err("indexed tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn capabilities_advertises_implemented_actions() {
    assert_eq!(
        capabilities(),
        vec![
            ActionKind::InstanceList,
            ActionKind::AppPing,
            ActionKind::AppInspect,
            ActionKind::AppVersion,
            ActionKind::AppActive,
            ActionKind::ActionList,
            ActionKind::ActionGet,
            ActionKind::AppFocus,
            ActionKind::AppSettingsOpen,
            ActionKind::AppCommandPaletteOpen,
            ActionKind::AppCommandSearchOpen,
            ActionKind::AppWarpDriveOpen,
            ActionKind::AppWarpDriveToggle,
            ActionKind::AppResourceCenterToggle,
            ActionKind::AppAiAssistantToggle,
            ActionKind::AppCodeReviewToggle,
            ActionKind::AppVerticalTabsToggle,
            ActionKind::WindowList,
            ActionKind::WindowCreate,
            ActionKind::WindowFocus,
            ActionKind::WindowClose,
            ActionKind::TabList,
            ActionKind::TabCreate,
            ActionKind::TabActivate,
            ActionKind::TabMove,
            ActionKind::TabRename,
            ActionKind::TabClose,
            ActionKind::PaneList,
            ActionKind::PaneSplit,
            ActionKind::PaneFocus,
            ActionKind::PaneNavigate,
            ActionKind::PaneClose,
            ActionKind::PaneMaximize,
            ActionKind::PaneResize,
            ActionKind::PaneSessionPrevious,
            ActionKind::PaneSessionNext,
            ActionKind::SessionList,
            ActionKind::BlockList,
            ActionKind::BlockGet,
            ActionKind::InputGet,
            ActionKind::InputInsert,
            ActionKind::InputReplace,
            ActionKind::InputClear,
            ActionKind::InputModeSet,
            ActionKind::InputRun,
            ActionKind::HistoryList,
            ActionKind::ThemeList,
            ActionKind::ThemeSet,
            ActionKind::AppearanceGet,
            ActionKind::AppearanceSet,
            ActionKind::AppearanceFontSize,
            ActionKind::AppearanceZoom,
            ActionKind::SettingGet,
            ActionKind::SettingList,
            ActionKind::SettingSet,
            ActionKind::SettingToggle,
            ActionKind::FileList,
            ActionKind::FileWrite,
            ActionKind::FileDelete,
            ActionKind::ProjectActive,
            ActionKind::ProjectList,
            ActionKind::DriveList,
            ActionKind::DriveGet,
            ActionKind::DriveCreate,
            ActionKind::DriveUpdate,
            ActionKind::DriveDelete,
            ActionKind::DriveRun,
            ActionKind::DriveInsert,
        ]
    );
}

#[test]
fn metadata_handlers_return_successful_empty_metadata_without_windows() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let bridge = app.add_model(LocalControlBridge::new);

        for action in [
            ActionKind::AppActive,
            ActionKind::AppInspect,
            ActionKind::AppVersion,
            ActionKind::WindowList,
            ActionKind::TabList,
            ActionKind::PaneList,
            ActionKind::SessionList,
        ] {
            let response = bridge.update(&mut app, |bridge, ctx| {
                bridge.handle_request(
                    RequestEnvelope::new(Action::new(action)),
                    grant_for(action),
                    ctx,
                )
            });
            match response.response {
                ControlResponse::Ok { data } => {
                    assert_eq!(data["action"], action.as_str());
                }
                ControlResponse::Error { error } => {
                    panic!("{} returned {error}", action.as_str());
                }
            }
        }
    });
}

#[test]
fn metadata_list_handlers_reject_stale_and_unsupported_selectors() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let bridge = app.add_model(LocalControlBridge::new);

        let cases = [
            (
                ActionKind::WindowList,
                TargetSelector {
                    tab: Some(TabTarget::Active),
                    ..TargetSelector::default()
                },
                ErrorCode::InvalidSelector,
            ),
            (
                ActionKind::WindowList,
                TargetSelector {
                    window: Some(WindowTarget::Id {
                        id: WindowSelector("stale-window".to_owned()),
                    }),
                    ..TargetSelector::default()
                },
                ErrorCode::StaleTarget,
            ),
            (
                ActionKind::TabList,
                TargetSelector {
                    tab: Some(TabTarget::Title {
                        title: "unsupported".to_owned(),
                    }),
                    ..TargetSelector::default()
                },
                ErrorCode::InvalidSelector,
            ),
            (
                ActionKind::PaneList,
                TargetSelector {
                    pane: Some(PaneTarget::Id {
                        id: PaneSelector("stale-pane".to_owned()),
                    }),
                    ..TargetSelector::default()
                },
                ErrorCode::StaleTarget,
            ),
            (
                ActionKind::SessionList,
                TargetSelector {
                    session: Some(SessionTarget::Id {
                        id: SessionSelector("stale-session".to_owned()),
                    }),
                    ..TargetSelector::default()
                },
                ErrorCode::StaleTarget,
            ),
            (
                ActionKind::SessionList,
                TargetSelector {
                    block: Some(BlockTarget::Active),
                    ..TargetSelector::default()
                },
                ErrorCode::InvalidSelector,
            ),
        ];

        for (action, target, code) in cases {
            let response = bridge.update(&mut app, |bridge, ctx| {
                bridge.handle_request(request_with_target(action, target), grant_for(action), ctx)
            });
            assert_eq!(response_error_code(response), code);
        }
    });
}

#[test]
fn terminal_reads_accept_default_and_active_targets() {
    for action in [ActionKind::InputGet, ActionKind::HistoryList] {
        validate_terminal_read_target(action, &TargetSelector::default())
            .expect("default target is accepted");

        validate_terminal_read_target(
            action,
            &TargetSelector {
                window: Some(WindowTarget::Active),
                tab: Some(TabTarget::Active),
                pane: Some(PaneTarget::Active),
                session: Some(SessionTarget::Active),
                ..TargetSelector::default()
            },
        )
        .expect("active target is accepted");
    }
}

#[test]
fn terminal_reads_reject_stale_concrete_targets() {
    let err = validate_terminal_read_target(
        ActionKind::InputGet,
        &TargetSelector {
            window: Some(WindowTarget::Id {
                id: WindowSelector("window".to_owned()),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("concrete window target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_terminal_read_target(
        ActionKind::InputGet,
        &TargetSelector {
            pane: Some(PaneTarget::Id {
                id: PaneSelector("pane".to_owned()),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_terminal_read_target(
        ActionKind::HistoryList,
        &TargetSelector {
            session: Some(SessionTarget::Id {
                id: SessionSelector("session".to_owned()),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("concrete session target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn terminal_reads_reject_unsupported_selector_forms() {
    let err = validate_terminal_read_target(
        ActionKind::InputGet,
        &TargetSelector {
            tab: Some(TabTarget::Index { index: 0 }),
            ..TargetSelector::default()
        },
    )
    .expect_err("indexed tab target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_terminal_read_target(
        ActionKind::HistoryList,
        &TargetSelector {
            file: Some(FileTarget::Path {
                path: "../secret".to_owned(),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("file target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn input_mutation_targets_accept_default_and_active_targets() {
    for action in [
        ActionKind::InputInsert,
        ActionKind::InputReplace,
        ActionKind::InputClear,
        ActionKind::InputModeSet,
    ] {
        validate_terminal_read_target(action, &TargetSelector::default())
            .expect("default target is accepted");

        validate_terminal_read_target(
            action,
            &TargetSelector {
                window: Some(WindowTarget::Active),
                tab: Some(TabTarget::Active),
                pane: Some(PaneTarget::Active),
                session: Some(SessionTarget::Active),
                ..TargetSelector::default()
            },
        )
        .expect("active target is accepted");
    }
}

#[test]
fn session_cycle_targets_accept_default_and_active_targets() {
    for action in [ActionKind::PaneSessionPrevious, ActionKind::PaneSessionNext] {
        validate_active_terminal_target(&TargetSelector::default(), action)
            .expect("default target is accepted");

        validate_active_terminal_target(
            &TargetSelector {
                window: Some(WindowTarget::Active),
                tab: Some(TabTarget::Active),
                pane: Some(PaneTarget::Active),
                ..TargetSelector::default()
            },
            action,
        )
        .expect("active target is accepted");
    }
}

#[test]
fn input_mutation_targets_reject_stale_and_unsupported_targets() {
    let err = validate_terminal_read_target(
        ActionKind::InputInsert,
        &TargetSelector {
            window: Some(WindowTarget::Id {
                id: WindowSelector("window".to_owned()),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("concrete window target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_terminal_read_target(
        ActionKind::InputReplace,
        &TargetSelector {
            session: Some(SessionTarget::Id {
                id: SessionSelector("session".to_owned()),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("concrete session target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_terminal_read_target(
        ActionKind::InputModeSet,
        &TargetSelector {
            block: Some(BlockTarget::Active),
            ..TargetSelector::default()
        },
    )
    .expect_err("block target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn session_cycle_targets_reject_stale_and_unsupported_targets() {
    let err = validate_active_terminal_target(
        &TargetSelector {
            pane: Some(PaneTarget::Id {
                id: PaneSelector("pane".to_owned()),
            }),
            ..TargetSelector::default()
        },
        ActionKind::PaneSessionNext,
    )
    .expect_err("concrete pane target is rejected");
    assert_eq!(err.code, ErrorCode::StaleTarget);

    let err = validate_active_terminal_target(
        &TargetSelector {
            file: Some(FileTarget::Path {
                path: "notes.txt".to_owned(),
            }),
            ..TargetSelector::default()
        },
        ActionKind::PaneSessionPrevious,
    )
    .expect_err("file target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_active_terminal_target(
        &TargetSelector {
            session: Some(SessionTarget::Active),
            ..TargetSelector::default()
        },
        ActionKind::PaneSessionNext,
    )
    .expect_err("session target is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn file_and_project_metadata_reads_reject_target_selectors() {
    validate_instance_metadata_read_target(ActionKind::FileList, &TargetSelector::default())
        .expect("default target is accepted");

    let err = validate_instance_metadata_read_target(
        ActionKind::FileList,
        &TargetSelector {
            file: Some(FileTarget::Path {
                path: "../secret".to_owned(),
            }),
            ..TargetSelector::default()
        },
    )
    .expect_err("file path selector is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_instance_metadata_read_target(
        ActionKind::ProjectList,
        &TargetSelector {
            window: Some(WindowTarget::Active),
            ..TargetSelector::default()
        },
    )
    .expect_err("project target selector is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
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
    assert!(!outside_warp_action_enabled_for_settings(
        &settings_with_outside_warp_permissions(true, false, true),
        ActionKind::WindowList
    ));
    assert!(outside_warp_action_enabled_for_settings(
        &settings_with_outside_warp_permissions(true, true, false),
        ActionKind::WindowList
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
fn terminal_reads_require_active_window_with_action_specific_error() {
    let active = warpui::WindowId::from_usize(1);

    assert_eq!(
        require_active_window_id_for_action(Some(active), ActionKind::InputGet).expect("active"),
        active
    );
    let err = require_active_window_id_for_action(None, ActionKind::HistoryList)
        .expect_err("missing active window");
    assert_eq!(err.code, ErrorCode::MissingTarget);
    assert!(err.message.contains("history.list"));
}

#[test]
fn feature_flag_disabled_denies_local_control() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(false);
    let err = ensure_feature_enabled().expect_err("feature flag disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn disabled_context_denies_before_granular_permission() {
    let settings = settings_with_values(false, true, true, true, true, true);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("inside-Warp parent context is disabled");
    assert_eq!(err.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn disabled_granular_permission_denies_with_insufficient_permissions() {
    let settings = settings_with_values(true, true, true, true, false, true);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect_err("read-write permission is disabled");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}
#[test]
fn app_state_mutations_require_read_write_permission() {
    let settings = settings_with_values(true, true, true, true, false, false);

    for action in [
        ActionKind::AppFocus,
        ActionKind::AppSettingsOpen,
        ActionKind::AppCommandPaletteOpen,
        ActionKind::AppCommandSearchOpen,
        ActionKind::AppWarpDriveOpen,
        ActionKind::AppWarpDriveToggle,
        ActionKind::AppResourceCenterToggle,
        ActionKind::AppAiAssistantToggle,
        ActionKind::AppCodeReviewToggle,
        ActionKind::AppVerticalTabsToggle,
        ActionKind::WindowCreate,
        ActionKind::WindowFocus,
        ActionKind::WindowClose,
    ] {
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::MutateAppState
        );
        let err = ensure_settings_allow_action(&settings, InvocationContext::InsideWarp, action)
            .expect_err("app-state mutation permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn app_state_mutation_grants_require_authenticated_user_and_action_scope() {
    let grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::AppFocus,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );

    let err = grant
        .verify_for_action(ActionKind::AppFocus)
        .expect_err("app.focus requires authenticated user grant");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserRequired);

    let mut grant = grant;
    grant.authenticated_user.subject = Some("user".to_owned());
    let err = grant
        .verify_for_action(ActionKind::WindowFocus)
        .expect_err("app.focus credential cannot focus a window");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn metadata_read_actions_require_read_permission() {
    let settings = settings_with_values(true, true, false, true, true, true);

    for action in [
        ActionKind::ActionList,
        ActionKind::WindowList,
        ActionKind::TabList,
        ActionKind::PaneList,
        ActionKind::SessionList,
        ActionKind::ThemeList,
        ActionKind::AppearanceGet,
        ActionKind::SettingGet,
        ActionKind::SettingList,
        ActionKind::FileList,
        ActionKind::ProjectActive,
        ActionKind::ProjectList,
        ActionKind::DriveList,
    ] {
        let err = ensure_settings_allow_action(&settings, InvocationContext::InsideWarp, action)
            .expect_err("read permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn underlying_data_read_actions_require_read_permission() {
    let settings = settings_with_values(true, true, false, true, true, true);

    for action in [
        ActionKind::BlockList,
        ActionKind::BlockGet,
        ActionKind::InputGet,
        ActionKind::HistoryList,
        ActionKind::DriveGet,
    ] {
        let err = ensure_settings_allow_action(&settings, InvocationContext::InsideWarp, action)
            .expect_err("read permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn metadata_scoped_credential_cannot_invoke_input_or_history_reads() {
    let grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::ActionList,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );

    for action in [ActionKind::InputGet, ActionKind::HistoryList] {
        let err = grant
            .verify_for_action(action)
            .expect_err("metadata-scoped credential cannot read underlying data");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn metadata_actions_require_metadata_permission_not_app_state_mutation_permission() {
    let metadata_without_mutation = settings_with_values(true, true, true, true, false, false);
    let mutation_without_metadata = settings_with_values(true, true, false, false, true, true);

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
        ActionKind::ThemeList,
        ActionKind::AppearanceGet,
        ActionKind::SettingGet,
        ActionKind::SettingList,
        ActionKind::FileList,
        ActionKind::ProjectActive,
        ActionKind::ProjectList,
        ActionKind::DriveList,
    ] {
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::ReadMetadata
        );
        ensure_settings_allow_action(
            &metadata_without_mutation,
            InvocationContext::InsideWarp,
            action,
        )
        .expect("metadata read permission allows metadata action");
        let err = ensure_settings_allow_action(
            &mutation_without_metadata,
            InvocationContext::InsideWarp,
            action,
        )
        .expect_err("metadata action is denied without metadata read permission");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }

    assert_eq!(
        ActionKind::TabCreate.metadata().permission_category,
        PermissionCategory::MutateAppState
    );
    ensure_settings_allow_action(
        &mutation_without_metadata,
        InvocationContext::InsideWarp,
        ActionKind::TabCreate,
    )
    .expect("app-state mutation permission allows tab.create");

    for action in [
        ActionKind::BlockList,
        ActionKind::BlockGet,
        ActionKind::InputGet,
        ActionKind::HistoryList,
        ActionKind::DriveGet,
    ] {
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::ReadUnderlyingData
        );
    }
}

#[test]
fn metadata_reads_require_read_only_permission() {
    let settings = settings_with_outside_warp_read_only(true, false);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::OutsideWarp,
        ActionKind::SettingGet,
    )
    .expect_err("read-only permission is disabled");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
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
fn action_metadata_lookup_reports_implemented_status_for_window_create() {
    let metadata = action_metadata_for_name("window.create").expect("allowlisted action");

    assert_eq!(metadata.kind, ActionKind::WindowCreate);
    assert_eq!(
        metadata.implementation_status,
        ::local_control::ActionImplementationStatus::Implemented
    );
}

#[test]
fn app_state_targets_reject_unsupported_and_stale_selectors() {
    let err = validate_app_focus_target(&TargetSelector {
        window: Some(WindowTarget::Active),
        ..TargetSelector::default()
    })
    .expect_err("app.focus rejects target selectors");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_window_create_target(
        &TargetSelector {
            tab: Some(TabTarget::Active),
            ..TargetSelector::default()
        },
        &WindowCreateParams::default(),
    )
    .expect_err("window.create rejects target selectors");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = validate_window_create_target(
        &TargetSelector::default(),
        &WindowCreateParams {
            profile: Some("Default".to_owned()),
        },
    )
    .expect_err("window.create profile selection is unsupported");
    assert_eq!(err.code, ErrorCode::UnsupportedAction);

    with_local_control_bridge(|_, ctx| {
        let err = select_window_for_app_state_target(
            ActionKind::WindowFocus,
            &TargetSelector::default(),
            ctx,
        )
        .expect_err("window.focus requires an active window");
        assert_eq!(err.code, ErrorCode::MissingTarget);

        let err = select_window_for_app_state_target(
            ActionKind::WindowFocus,
            &TargetSelector {
                window: Some(WindowTarget::Id {
                    id: WindowSelector("stale-window".to_owned()),
                }),
                ..TargetSelector::default()
            },
            ctx,
        )
        .expect_err("window.focus rejects stale window id");
        assert_eq!(err.code, ErrorCode::StaleTarget);

        let err = select_window_for_app_state_target(
            ActionKind::WindowClose,
            &TargetSelector {
                pane: Some(PaneTarget::Active),
                ..TargetSelector::default()
            },
            ctx,
        )
        .expect_err("window.close rejects pane selectors");
        assert_eq!(err.code, ErrorCode::InvalidSelector);
    });
}

#[test]
fn app_surface_actions_validate_params_and_map_to_allowlisted_workspace_actions() {
    let settings_action = workspace_action_for_surface(
        ActionKind::AppSettingsOpen,
        AppSurfaceParams {
            query: Some("font".to_owned()),
            page: Some("appearance".to_owned()),
        },
    )
    .expect("settings surface maps to workspace action");
    let WorkspaceAction::ShowSettingsPageWithSearch {
        search_query,
        section,
    } = settings_action
    else {
        panic!("expected settings search action");
    };
    assert_eq!(search_query, "font");
    assert!(section.is_some());

    let palette_action = workspace_action_for_surface(
        ActionKind::AppCommandPaletteOpen,
        AppSurfaceParams {
            query: Some("new tab".to_owned()),
            page: None,
        },
    )
    .expect("command palette maps to workspace action");
    let WorkspaceAction::OpenPalette { query, .. } = palette_action else {
        panic!("expected command palette action");
    };
    assert_eq!(query.as_deref(), Some("new tab"));

    let drive_action =
        workspace_action_for_surface(ActionKind::AppWarpDriveToggle, AppSurfaceParams::default())
            .expect("warp drive toggle maps to workspace action");
    assert!(matches!(drive_action, WorkspaceAction::ToggleWarpDrive));

    let err = workspace_action_for_surface(
        ActionKind::AppCommandPaletteOpen,
        AppSurfaceParams {
            query: None,
            page: Some("commands".to_owned()),
        },
    )
    .expect_err("command palette rejects page");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    let err = workspace_action_for_surface(
        ActionKind::AppWarpDriveToggle,
        AppSurfaceParams {
            query: Some("workflows".to_owned()),
            page: None,
        },
    )
    .expect_err("toggle surface rejects params");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn app_focus_handler_requires_authenticated_user_and_can_succeed_without_windows() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_state_app(&mut app, true);
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                RequestEnvelope::new(Action::new(ActionKind::AppFocus)),
                authenticated_grant(ActionKind::AppFocus, ctx),
                ctx,
            );
            let ControlResponse::Ok { data } = response.response else {
                panic!("expected ok response");
            };
            assert_eq!(data["action"], ActionKind::AppFocus.as_str());
            assert_eq!(data["focused"], true);
            assert_eq!(data["window_id"], serde_json::Value::Null);
        });
    });

    App::test((), |mut app| async move {
        initialize_app_state_app(&mut app, false);
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                RequestEnvelope::new(Action::new(ActionKind::AppFocus)),
                spoofed_authenticated_grant(ActionKind::AppFocus),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::AuthenticatedUserUnavailable
            );
        });
    });
}

#[test]
fn action_get_rejects_unknown_action_names() {
    let err = validate_action_params(&Action {
        kind: ActionKind::ActionGet,
        params: serde_json::json!({ "action": "unknown.run" }),
    })
    .expect_err("unknown action is rejected");
    assert_eq!(err.code, ErrorCode::NotAllowlisted);
}

#[test]
fn mutating_stub_actions_validate_typed_params() {
    let actions = [
        Action::with_params(ActionKind::AppFocus, AppFocusParams::default())
            .expect("params serialize"),
        Action::with_params(ActionKind::AppSettingsOpen, AppSurfaceParams::default())
            .expect("params serialize"),
        Action::with_params(
            ActionKind::WindowCreate,
            WindowCreateParams {
                profile: Some("Default".to_owned()),
            },
        )
        .expect("params serialize"),
        Action::with_params(ActionKind::WindowFocus, WindowFocusParams::default())
            .expect("params serialize"),
        Action::with_params(ActionKind::WindowClose, WindowCloseParams { force: true })
            .expect("params serialize"),
        Action::with_params(
            ActionKind::TabActivate,
            TabActivateParams {
                relative: Some(TabActivationTarget::Next),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::TabMove,
            TabMoveParams {
                direction: HorizontalDirection::Left,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::TabRename,
            TabRenameParams {
                title: Some("build".to_owned()),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::TabClose,
            TabCloseParams {
                scope: TabCloseScope::Others,
                force: true,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::PaneSplit,
            PaneSplitParams {
                direction: PaneDirection::Right,
                profile: None,
            },
        )
        .expect("params serialize"),
        Action::with_params(ActionKind::PaneFocus, PaneFocusParams::default())
            .expect("params serialize"),
        Action::with_params(
            ActionKind::PaneNavigate,
            PaneNavigateParams {
                direction: PaneDirection::Down,
            },
        )
        .expect("params serialize"),
        Action::with_params(ActionKind::PaneClose, PaneCloseParams { force: true })
            .expect("params serialize"),
        Action::with_params(
            ActionKind::PaneMaximize,
            PaneMaximizeParams {
                enabled: Some(true),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::PaneResize,
            PaneResizeParams {
                direction: PaneDirection::Up,
                amount: Some(8),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::InputInsert,
            InputInsertParams {
                text: "cargo check".to_owned(),
                replace: true,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::InputReplace,
            InputReplaceParams {
                text: "cargo test".to_owned(),
            },
        )
        .expect("params serialize"),
        Action::with_params(ActionKind::InputClear, InputClearParams::default())
            .expect("params serialize"),
        Action::with_params(
            ActionKind::InputModeSet,
            InputModeSetParams {
                mode: InputMode::Agent,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::InputRun,
            InputRunParams {
                command: "cargo check".to_owned(),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::ThemeSet,
            ThemeSetParams {
                name: "Warp Dark".to_owned(),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::AppearanceSet,
            AppearanceSetParams {
                theme: Some("Warp Dark".to_owned()),
                follow_system_theme: None,
                light_theme: None,
                dark_theme: None,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::AppearanceFontSize,
            AppearanceFontSizeParams {
                adjustment: SizeAdjustment::Increase,
                value: None,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::AppearanceZoom,
            AppearanceZoomParams {
                adjustment: SizeAdjustment::Set,
                value: Some(120),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::SettingSet,
            SettingSetParams {
                key: "appearance.theme".to_owned(),
                value: serde_json::json!("Warp Dark"),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::SettingToggle,
            SettingToggleParams {
                key: "appearance.follow_system".to_owned(),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::FileOpen,
            FileOpenParams {
                path: "src/main.rs".to_owned(),
                line: Some(12),
                new_window: false,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::FileWrite,
            FileWriteParams {
                path: "notes.txt".to_owned(),
                contents: "hello".to_owned(),
                create: true,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::FileDelete,
            FileDeleteParams {
                path: "notes.txt".to_owned(),
                recursive: false,
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::DriveCreate,
            DriveCreateParams {
                object_type: DriveObjectType::Workflow,
                name: "build".to_owned(),
                content: serde_json::json!({ "command": "cargo check" }),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::DriveUpdate,
            DriveUpdateParams {
                object_type: DriveObjectType::Notebook,
                id: "notebook_123".to_owned(),
                content: serde_json::json!({ "title": "notes" }),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::DriveDelete,
            DriveDeleteParams {
                object_type: DriveObjectType::Prompt,
                id: "prompt_123".to_owned(),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::DriveRun,
            DriveRunParams {
                object_type: DriveObjectType::Workflow,
                id: "workflow_123".to_owned(),
            },
        )
        .expect("params serialize"),
        Action::with_params(
            ActionKind::DriveInsert,
            DriveInsertParams {
                object_type: DriveObjectType::Notebook,
                id: "notebook_123".to_owned(),
            },
        )
        .expect("params serialize"),
    ];

    for action in actions {
        validate_action_params(&action).expect("mutating params are accepted");
    }
}

#[test]
fn mutating_permissions_keep_app_metadata_and_underlying_data_separate() {
    assert_eq!(
        ActionKind::TabActivate.metadata().permission_category,
        PermissionCategory::MutateAppState
    );
    assert_eq!(
        ActionKind::TabRename.metadata().permission_category,
        PermissionCategory::MutateMetadataConfiguration
    );
    assert_eq!(
        ActionKind::PaneSessionPrevious
            .metadata()
            .permission_category,
        PermissionCategory::MutateAppState
    );
    assert_eq!(
        ActionKind::PaneSessionNext.metadata().permission_category,
        PermissionCategory::MutateAppState
    );
    assert_eq!(
        ActionKind::SettingSet.metadata().permission_category,
        PermissionCategory::MutateMetadataConfiguration
    );
    for action in [
        ActionKind::InputInsert,
        ActionKind::InputReplace,
        ActionKind::InputClear,
        ActionKind::InputModeSet,
        ActionKind::InputRun,
    ] {
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::MutateUnderlyingData
        );
    }
    assert_eq!(
        ActionKind::InputRun.metadata().permission_category,
        PermissionCategory::MutateUnderlyingData
    );
    assert_eq!(
        ActionKind::FileWrite.metadata().permission_category,
        PermissionCategory::MutateUnderlyingData
    );
    assert_eq!(
        ActionKind::FileDelete.metadata().permission_category,
        PermissionCategory::MutateUnderlyingData
    );

    let mut grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::InputRun,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.permission_category = PermissionCategory::MutateAppState;
    grant.authenticated_user.subject = Some("user".to_owned());

    let err = grant
        .verify_for_action(ActionKind::InputRun)
        .expect_err("app-state mutation category does not satisfy command execution");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);

    let mut grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::FileWrite,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.permission_category = PermissionCategory::MutateAppState;
    grant.authenticated_user.subject = Some("user".to_owned());

    let err = grant
        .verify_for_action(ActionKind::FileWrite)
        .expect_err("app-state mutation category does not satisfy file writes");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);

    let mut grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::TabRename,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.permission_category = PermissionCategory::MutateAppState;
    grant.authenticated_user.subject = Some("user".to_owned());

    let err = grant
        .verify_for_action(ActionKind::TabRename)
        .expect_err("app-state mutation category does not satisfy metadata mutation");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);

    let mut grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::InputInsert,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.permission_category = PermissionCategory::MutateAppState;
    grant.authenticated_user.subject = Some("user".to_owned());

    let err = grant
        .verify_for_action(ActionKind::InputInsert)
        .expect_err("app-state mutation category does not satisfy input mutation");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn input_mutations_require_underlying_data_mutation_permission() {
    let settings = settings_with_values(true, true, true, true, true, false);

    for action in [
        ActionKind::InputInsert,
        ActionKind::InputReplace,
        ActionKind::InputClear,
        ActionKind::InputModeSet,
    ] {
        let err = ensure_settings_allow_action(&settings, InvocationContext::OutsideWarp, action)
            .expect_err("underlying-data mutation permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);

        let mut grant = CredentialGrant::new(
            InstanceId("instance".to_owned()),
            action,
            InvocationContext::InsideWarp,
            Duration::minutes(5),
        );
        grant.permission_category = PermissionCategory::MutateAppState;
        grant.authenticated_user.subject = Some("user".to_owned());
        let err = grant
            .verify_for_action(action)
            .expect_err("app-state permission does not satisfy input mutation");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn session_cycle_actions_require_app_state_mutation_permission() {
    let settings = settings_with_values(true, true, true, true, false, true);

    for action in [ActionKind::PaneSessionPrevious, ActionKind::PaneSessionNext] {
        let err = ensure_settings_allow_action(&settings, InvocationContext::InsideWarp, action)
            .expect_err("app-state mutation permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);

        let mut grant = CredentialGrant::new(
            InstanceId("instance".to_owned()),
            action,
            InvocationContext::InsideWarp,
            Duration::minutes(5),
        );
        grant.permission_category = PermissionCategory::MutateUnderlyingData;
        grant.authenticated_user.subject = Some("user".to_owned());
        let err = grant
            .verify_for_action(action)
            .expect_err("underlying-data mutation permission does not satisfy session cycling");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn input_mutation_handlers_change_buffer_and_mode_without_executing() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(LocalControlBridge::new);

        let terminal = add_window_with_terminal(&mut app, None);
        let session_id = SessionId::from(123);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model
                    .block_list_mut()
                    .active_block_for_test()
                    .set_session_id(session_id);
            }
            let window_id = ctx.window_id();
            WindowManager::handle(ctx).update(ctx, |state, ctx| {
                state.overwrite_for_test(ApplicationStage::Active, Some(window_id));
                ctx.notify();
            });
            ctx.windows().show_window_and_focus_app(window_id);
            view.set_active_block_metadata_for_test(
                BlockMetadata::new(Some(session_id), None),
                ctx,
            );
        });

        let initial_block_count = terminal.read(&app, |view, _| {
            let model = view.model.lock();
            model.block_list().blocks().len()
        });

        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let insert_response = bridge.handle_request(
                request_with_params(
                    ActionKind::InputInsert,
                    InputInsertParams {
                        text: "cargo".to_owned(),
                        replace: false,
                    },
                ),
                authenticated_grant(ActionKind::InputInsert, ctx),
                ctx,
            );
            let inserted: InputStateResult =
                serde_json::from_value(response_ok_data(insert_response))
                    .expect("input.insert result decodes");
            assert_eq!(inserted.session_id, session_id.as_u64().to_string());
            assert_eq!(inserted.text, "cargo");

            let append_response = bridge.handle_request(
                request_with_params(
                    ActionKind::InputInsert,
                    InputInsertParams {
                        text: " check".to_owned(),
                        replace: false,
                    },
                ),
                authenticated_grant(ActionKind::InputInsert, ctx),
                ctx,
            );
            let appended: InputStateResult =
                serde_json::from_value(response_ok_data(append_response))
                    .expect("input.insert append result decodes");
            assert_eq!(appended.text, "cargo check");

            let replace_response = bridge.handle_request(
                request_with_params(
                    ActionKind::InputReplace,
                    InputReplaceParams {
                        text: "cargo test".to_owned(),
                    },
                ),
                authenticated_grant(ActionKind::InputReplace, ctx),
                ctx,
            );
            let replaced: InputStateResult =
                serde_json::from_value(response_ok_data(replace_response))
                    .expect("input.replace result decodes");
            assert_eq!(replaced.text, "cargo test");

            let mode_response = bridge.handle_request(
                request_with_params(
                    ActionKind::InputModeSet,
                    InputModeSetParams {
                        mode: InputMode::Agent,
                    },
                ),
                authenticated_grant(ActionKind::InputModeSet, ctx),
                ctx,
            );
            let mode_result: InputStateResult =
                serde_json::from_value(response_ok_data(mode_response))
                    .expect("input.mode.set result decodes");
            assert_eq!(mode_result.text, "cargo test");

            let clear_response = bridge.handle_request(
                request_with_params(ActionKind::InputClear, InputClearParams::default()),
                authenticated_grant(ActionKind::InputClear, ctx),
                ctx,
            );
            let cleared: InputStateResult =
                serde_json::from_value(response_ok_data(clear_response))
                    .expect("input.clear result decodes");
            assert_eq!(cleared.text, "");
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input_config(ctx),
                InputConfig {
                    input_type: InputType::AI,
                    is_locked: true,
                }
            );
            let model = view.model.lock();
            assert_eq!(model.block_list().blocks().len(), initial_block_count);
            assert_eq!(model.block_list().active_block().command_to_string(), "");
        });
    })
}

#[test]
fn action_list_rejects_malformed_params() {
    let err = validate_action_params(&Action {
        kind: ActionKind::ActionList,
        params: serde_json::json!({ "all": true }),
    })
    .expect_err("action.list params must be empty");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn app_target_metadata_reads_reject_malformed_params() {
    for action in [
        ActionKind::AppVersion,
        ActionKind::AppActive,
        ActionKind::AppInspect,
        ActionKind::WindowList,
        ActionKind::TabList,
        ActionKind::PaneList,
        ActionKind::SessionList,
    ] {
        let err = validate_action_params(&Action {
            kind: action,
            params: serde_json::json!({ "unexpected": true }),
        })
        .expect_err("app target metadata read params must be empty");
        assert_eq!(err.code, ErrorCode::InvalidParams);

        validate_action_params(&Action {
            kind: action,
            params: serde_json::json!({}),
        })
        .expect("empty app target metadata read params are accepted");
    }
}

#[test]
fn input_and_history_reject_malformed_params() {
    let err = validate_action_params(&Action {
        kind: ActionKind::InputGet,
        params: serde_json::json!({ "text": true }),
    })
    .expect_err("input.get params must be empty");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    validate_action_params(&Action {
        kind: ActionKind::InputGet,
        params: serde_json::json!({}),
    })
    .expect("empty input.get params are accepted");

    validate_action_params(&Action {
        kind: ActionKind::HistoryList,
        params: serde_json::json!({ "limit": 5 }),
    })
    .expect("history.list limit is accepted");

    let err = validate_action_params(&Action {
        kind: ActionKind::HistoryList,
        params: serde_json::json!({ "command": true }),
    })
    .expect_err("unexpected history.list params are rejected");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn input_run_rejects_empty_command_params() {
    let err = validate_action_params(
        &Action::with_params(
            ActionKind::InputRun,
            InputRunParams {
                command: "  \t  ".to_owned(),
            },
        )
        .expect("input.run params serialize"),
    )
    .expect_err("empty command is rejected");
    assert_eq!(err.code, ErrorCode::InvalidParams);
}

#[test]
fn tab_rename_rejects_malformed_params() {
    let err = validate_action_params(
        &Action::with_params(
            ActionKind::TabRename,
            TabRenameParams {
                title: Some("  \t  ".to_owned()),
            },
        )
        .expect("tab.rename params serialize"),
    )
    .expect_err("blank title is rejected");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    validate_action_params(
        &Action::with_params(ActionKind::TabRename, TabRenameParams { title: None })
            .expect("tab.rename reset params serialize"),
    )
    .expect("reset params are accepted");
}
#[test]
fn input_run_policy_gate_fails_closed_and_allows_test_override() {
    let action = Action::with_params(
        ActionKind::InputRun,
        InputRunParams {
            command: "echo hi".to_owned(),
        },
    )
    .expect("input.run params serialize");
    let mut grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::InputRun,
        InvocationContext::InsideWarp,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = Some("user".to_owned());

    let err = ensure_input_run_policy_allows(&grant, &action)
        .expect_err("input.run policy fails closed by default");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);

    let guard = allow_input_run_policy_for_test();
    ensure_input_run_policy_allows(&grant, &action).expect("test policy override allows input.run");
    drop(guard);
}

#[test]
fn input_run_denials_happen_before_selector_resolution() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let request = RequestEnvelope {
            target: TargetSelector {
                window: Some(WindowTarget::Id {
                    id: WindowSelector("stale-window".to_owned()),
                }),
                ..TargetSelector::default()
            },
            ..RequestEnvelope::new(
                Action::with_params(
                    ActionKind::InputRun,
                    InputRunParams {
                        command: "echo hi".to_owned(),
                    },
                )
                .expect("input.run params serialize"),
            )
        };
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let mut wrong_permission_grant = authenticated_grant(ActionKind::InputRun, ctx);
            wrong_permission_grant.permission_category = PermissionCategory::MutateAppState;
            let response = bridge.handle_request(request.clone(), wrong_permission_grant, ctx);
            assert_eq!(
                response_error_code(response),
                ErrorCode::InsufficientPermissions
            );

            let response = bridge.handle_request(
                request.clone(),
                spoofed_authenticated_grant(ActionKind::InputRun),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::AuthenticatedUserUnavailable
            );

            let response =
                bridge.handle_request(request, authenticated_grant(ActionKind::InputRun, ctx), ctx);
            assert_eq!(
                response_error_code(response),
                ErrorCode::InsufficientPermissions
            );
        });
    })
}

#[test]
fn input_run_reaches_target_resolution_only_with_explicit_policy_gate() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::InputRun,
                InputRunParams {
                    command: "echo hi".to_owned(),
                },
            )
            .expect("input.run params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let guard = allow_input_run_policy_for_test();
            let response =
                bridge.handle_request(request, authenticated_grant(ActionKind::InputRun, ctx), ctx);
            assert_eq!(response_error_code(response), ErrorCode::MissingTarget);
            drop(guard);
        });
    })
}

#[test]
fn file_and_project_metadata_reads_reject_malformed_params() {
    for action in [
        ActionKind::FileList,
        ActionKind::ProjectActive,
        ActionKind::ProjectList,
    ] {
        let err = validate_action_params(&Action {
            kind: action,
            params: serde_json::json!({ "unexpected": true }),
        })
        .expect_err("metadata read params must be empty");
        assert_eq!(err.code, ErrorCode::InvalidParams);

        validate_action_params(&Action {
            kind: action,
            params: serde_json::json!({}),
        })
        .expect("empty metadata read params are accepted");
    }
}

#[test]
fn block_reads_require_underlying_data_permission() {
    let settings = settings_with_values(true, true, false, false, true, true);

    let err = ensure_settings_allow_action(
        &settings,
        InvocationContext::InsideWarp,
        ActionKind::BlockList,
    )
    .expect_err("underlying data read permission is disabled");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn file_mutations_require_underlying_data_permission() {
    let settings = settings_with_values(true, true, true, true, false, false);

    for action in [ActionKind::FileWrite, ActionKind::FileDelete] {
        assert_eq!(
            action.metadata().permission_category,
            PermissionCategory::MutateUnderlyingData
        );
        let err = ensure_settings_allow_action(&settings, InvocationContext::InsideWarp, action)
            .expect_err("underlying data mutation permission is disabled");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}

#[test]
fn file_mutation_grant_requires_authenticated_user_subject() {
    let grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::FileWrite,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );

    let err = grant
        .verify_for_action(ActionKind::FileWrite)
        .expect_err("file.write requires authenticated user grant");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserRequired);
}

#[test]
fn file_mutations_require_true_logged_in_user_before_path_resolution() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let tempdir = tempfile::tempdir().expect("tempdir is created");
        initialize_file_mutation_app(&mut app, tempdir.path(), false);
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::FileWrite,
                FileWriteParams {
                    path: "../secret".to_owned(),
                    contents: "secret".to_owned(),
                    create: true,
                },
            )
            .expect("file.write params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                request,
                spoofed_authenticated_grant(ActionKind::FileWrite),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::AuthenticatedUserUnavailable
            );
        });
    })
}

#[test]
fn file_mutation_path_safety_rejects_traversal_outside_roots_and_ambiguous_relative_paths() {
    let tempdir = tempfile::tempdir().expect("tempdir is created");
    let root = tempdir.path().join("workspace");
    let other_root = tempdir.path().join("other-workspace");
    let outside = tempdir.path().join("outside.txt");
    fs::create_dir(&root).expect("root is created");
    fs::create_dir(&other_root).expect("other root is created");
    fs::write(&outside, "outside").expect("outside file is written");
    let roots = vec![root.canonicalize().expect("root canonicalizes")];

    let err = resolve_file_mutation_path(ActionKind::FileWrite, "../secret", &roots, true)
        .expect_err("parent traversal is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let err = resolve_file_mutation_path(
        ActionKind::FileWrite,
        &outside.display().to_string(),
        &roots,
        false,
    )
    .expect_err("absolute path outside root is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);

    let multiple_roots = vec![
        root.canonicalize().expect("root canonicalizes"),
        other_root.canonicalize().expect("other root canonicalizes"),
    ];
    let err =
        resolve_file_mutation_path(ActionKind::FileWrite, "relative.txt", &multiple_roots, true)
            .expect_err("relative path is ambiguous with multiple roots");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn file_mutation_targets_reject_unsupported_and_conflicting_selectors() {
    let err = validate_file_mutation_target(
        ActionKind::FileWrite,
        &TargetSelector {
            file: Some(FileTarget::Id {
                id: FileSelector("file-id".to_owned()),
            }),
            ..TargetSelector::default()
        },
        "notes.txt",
    )
    .expect_err("file id selector is unsupported");
    assert_eq!(err.code, ErrorCode::UnsupportedAction);

    let err = validate_file_mutation_target(
        ActionKind::FileDelete,
        &TargetSelector {
            file: Some(FileTarget::Path {
                path: "other.txt".to_owned(),
            }),
            ..TargetSelector::default()
        },
        "notes.txt",
    )
    .expect_err("mismatched file path target is rejected");
    assert_eq!(err.code, ErrorCode::TargetStateConflict);
}

#[test]
fn file_delete_rejects_recursive_and_directory_targets() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let tempdir = tempfile::tempdir().expect("tempdir is created");
        initialize_file_mutation_app(&mut app, tempdir.path(), true);
        let directory = tempdir.path().join("nested");
        fs::create_dir(&directory).expect("directory is created");

        let recursive_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::FileDelete,
                FileDeleteParams {
                    path: directory.display().to_string(),
                    recursive: true,
                },
            )
            .expect("file.delete params serialize"),
        );
        let directory_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::FileDelete,
                FileDeleteParams {
                    path: directory.display().to_string(),
                    recursive: false,
                },
            )
            .expect("file.delete params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                recursive_request,
                authenticated_grant(ActionKind::FileDelete, ctx),
                ctx,
            );
            assert_eq!(response_error_code(response), ErrorCode::UnsupportedAction);

            let response = bridge.handle_request(
                directory_request,
                authenticated_grant(ActionKind::FileDelete, ctx),
                ctx,
            );
            assert_eq!(response_error_code(response), ErrorCode::UnsupportedAction);
        });
    })
}

#[test]
fn file_write_create_and_delete_succeed_for_disposable_project_files() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        let tempdir = tempfile::tempdir().expect("tempdir is created");
        initialize_file_mutation_app(&mut app, tempdir.path(), true);
        let existing = tempdir.path().join("existing.txt");
        fs::write(&existing, "old").expect("existing file is written");
        let created = tempdir.path().join("created.txt");

        let write_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::FileWrite,
                FileWriteParams {
                    path: existing.display().to_string(),
                    contents: "new".to_owned(),
                    create: false,
                },
            )
            .expect("file.write params serialize"),
        );
        let create_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::FileWrite,
                FileWriteParams {
                    path: created.display().to_string(),
                    contents: "created".to_owned(),
                    create: true,
                },
            )
            .expect("file.write params serialize"),
        );
        let delete_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::FileDelete,
                FileDeleteParams {
                    path: created.display().to_string(),
                    recursive: false,
                },
            )
            .expect("file.delete params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                write_request,
                authenticated_grant(ActionKind::FileWrite, ctx),
                ctx,
            );
            let ControlResponse::Ok { data } = response.response else {
                panic!("expected file.write ok response");
            };
            let result: FileMutationResult =
                serde_json::from_value(data).expect("file mutation result decodes");
            let existing_path = existing
                .canonicalize()
                .expect("existing file canonicalizes")
                .display()
                .to_string();
            assert_eq!(result.path, existing_path);
            assert_eq!(
                fs::read_to_string(&existing).expect("existing file is read"),
                "new"
            );

            let response = bridge.handle_request(
                create_request,
                authenticated_grant(ActionKind::FileWrite, ctx),
                ctx,
            );
            let ControlResponse::Ok { data } = response.response else {
                panic!("expected file.write create ok response");
            };
            let result: FileMutationResult =
                serde_json::from_value(data).expect("file mutation result decodes");
            let created_path = created
                .canonicalize()
                .expect("created file canonicalizes")
                .display()
                .to_string();
            assert_eq!(result.path, created_path);
            assert_eq!(
                fs::read_to_string(&created).expect("created file is read"),
                "created"
            );

            let response = bridge.handle_request(
                delete_request,
                authenticated_grant(ActionKind::FileDelete, ctx),
                ctx,
            );
            let ControlResponse::Ok { data } = response.response else {
                panic!("expected file.delete ok response");
            };
            let result: FileMutationResult =
                serde_json::from_value(data).expect("file mutation result decodes");
            assert_eq!(result.path, created_path);
            assert!(!created.exists());
        });
    })
}

#[test]
fn metadata_read_grant_cannot_read_blocks() {
    let grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::AppPing,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );

    let err = grant
        .verify_for_action(ActionKind::BlockList)
        .expect_err("metadata credential cannot read terminal data");
    assert_eq!(err.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn block_read_grant_requires_authenticated_user_subject() {
    let grant = CredentialGrant::new(
        InstanceId("instance".to_owned()),
        ActionKind::BlockGet,
        InvocationContext::OutsideWarp,
        Duration::minutes(5),
    );

    let err = grant
        .verify_for_action(ActionKind::BlockGet)
        .expect_err("block.get requires authenticated user grant");
    assert_eq!(err.code, ErrorCode::AuthenticatedUserRequired);
}

#[test]
fn block_read_targets_accept_default_and_active_session() {
    validate_block_list_target(&TargetSelector::default()).expect("default target is accepted");
    validate_block_get_target(&TargetSelector {
        session: Some(SessionTarget::Active),
        ..TargetSelector::default()
    })
    .expect("active session target is accepted");
}

#[test]
fn block_list_rejects_block_selector() {
    let err = validate_block_list_target(&TargetSelector {
        block: Some(::local_control::protocol::BlockTarget::Id {
            id: ::local_control::protocol::BlockSelector("block".to_owned()),
        }),
        ..TargetSelector::default()
    })
    .expect_err("block.list does not accept block selectors");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn block_read_rejects_stale_session_targets() {
    let model = TerminalModel::mock(None, None);

    let err = block_list_result_from_model(
        &model,
        SessionId::from(42),
        true,
        BlockListParams::default(),
    )
    .expect_err("explicit session id is stale");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn block_get_rejects_stale_block_targets() {
    let model = TerminalModel::mock(None, None);

    let err = block_get_result_from_model(&model, SessionId::from(0), "missing-block")
        .expect_err("block id is stale");
    assert_eq!(err.code, ErrorCode::StaleTarget);
}

#[test]
fn block_list_and_get_return_active_session_block_output() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_block("echo hi", "hello from block");
    let session_id = SessionId::from(7);
    let mut block_id = None;

    for block in model.block_list_mut().blocks_mut() {
        if block.command_to_string() == "echo hi" {
            block.set_session_id(session_id);
            block_id = Some(block.id().to_string());
        }
    }

    let Some(block_id) = block_id else {
        panic!("expected simulated block id");
    };
    let list = block_list_result_from_model(
        &model,
        session_id,
        false,
        BlockListParams { limit: Some(1) },
    )
    .expect("block list succeeds");
    assert_eq!(list.blocks.len(), 1);
    assert_eq!(list.blocks[0].block_id, block_id);
    assert_eq!(list.blocks[0].command.as_deref(), Some("echo hi"));

    let params = BlockGetParams {
        block_id: block_id.clone(),
    };
    let block = block_get_result_from_model(&model, session_id, &params.block_id)
        .expect("block get succeeds");
    assert_eq!(block.block.block_id, block_id);
    assert_eq!(block.output.as_deref(), Some("hello from block"));
}

#[test]
fn drive_actions_validate_params_and_targets() {
    validate_action_params(
        &Action::with_params(ActionKind::DriveList, DriveListParams::default())
            .expect("drive list params serialize"),
    )
    .expect("drive.list params are accepted");

    let err = validate_action_params(
        &Action::with_params(
            ActionKind::DriveGet,
            DriveGetParams {
                object_type: DriveObjectType::Workflow,
                id: String::new(),
            },
        )
        .expect("drive get params serialize"),
    )
    .expect_err("empty drive object id is rejected");
    assert_eq!(err.code, ErrorCode::InvalidParams);

    let err = validate_drive_target(
        &TargetSelector {
            window: Some(WindowTarget::Active),
            ..TargetSelector::default()
        },
        ActionKind::DriveList,
    )
    .expect_err("window selector is rejected");
    assert_eq!(err.code, ErrorCode::InvalidSelector);
}

#[test]
fn drive_list_requires_true_logged_in_user() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, false);
        let request = RequestEnvelope::new(
            Action::with_params(ActionKind::DriveList, DriveListParams::default())
                .expect("drive.list params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                request,
                spoofed_authenticated_grant(ActionKind::DriveList),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::AuthenticatedUserUnavailable
            );
        });
    })
}

#[test]
fn drive_list_returns_authenticated_metadata_without_content() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        create_workflow(&mut app, "build", "cargo check");
        create_notebook(&mut app, "notes", "# Notes");
        create_folder(&mut app, "folder");
        let request = RequestEnvelope::new(
            Action::with_params(ActionKind::DriveList, DriveListParams::default())
                .expect("drive.list params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                request,
                authenticated_grant(ActionKind::DriveList, ctx),
                ctx,
            );
            let ControlResponse::Ok { data } = response.response else {
                panic!("expected ok response");
            };
            let result: DriveListResult =
                serde_json::from_value(data.clone()).expect("drive list result decodes");
            assert_eq!(result.objects.len(), 2);
            assert_eq!(result.objects[0].object_type, DriveObjectType::Workflow);
            assert_eq!(result.objects[0].name, "build");
            assert_eq!(result.objects[1].object_type, DriveObjectType::Notebook);
            assert_eq!(result.objects[1].name, "notes");
            assert!(data["objects"][0].get("content").is_none());
            assert!(data["objects"][1].get("content").is_none());
        });
    })
}

#[test]
fn drive_get_returns_authenticated_underlying_content() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let workflow_id = create_workflow(&mut app, "build", "cargo check");
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveGet,
                DriveGetParams {
                    object_type: DriveObjectType::Workflow,
                    id: workflow_id,
                },
            )
            .expect("drive.get params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response =
                bridge.handle_request(request, authenticated_grant(ActionKind::DriveGet, ctx), ctx);
            let ControlResponse::Ok { data } = response.response else {
                panic!("expected ok response");
            };
            let result: DriveGetResult =
                serde_json::from_value(data).expect("drive get result decodes");
            assert_eq!(result.object.object_type, DriveObjectType::Workflow);
            assert_eq!(result.object.name, "build");
            assert_eq!(result.content["command"], "cargo check");
        });
    })
}

#[test]
fn drive_metadata_grant_cannot_read_underlying_content() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let workflow_id = create_workflow(&mut app, "build", "cargo check");
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveGet,
                DriveGetParams {
                    object_type: DriveObjectType::Workflow,
                    id: workflow_id,
                },
            )
            .expect("drive.get params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                request,
                authenticated_grant(ActionKind::DriveList, ctx),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::InsufficientPermissions
            );
        });
    })
}

#[test]
fn drive_get_rejects_unsupported_or_mismatched_objects() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let folder_id = create_folder(&mut app, "folder");
        let workflow_id = create_workflow(&mut app, "build", "cargo check");
        let unsupported_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveGet,
                DriveGetParams {
                    object_type: DriveObjectType::Workflow,
                    id: folder_id,
                },
            )
            .expect("drive.get params serialize"),
        );
        let mismatched_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveGet,
                DriveGetParams {
                    object_type: DriveObjectType::Notebook,
                    id: workflow_id,
                },
            )
            .expect("drive.get params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                unsupported_request,
                authenticated_grant(ActionKind::DriveGet, ctx),
                ctx,
            );
            assert_eq!(response_error_code(response), ErrorCode::UnsupportedAction);

            let response = bridge.handle_request(
                mismatched_request,
                authenticated_grant(ActionKind::DriveGet, ctx),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::TargetStateConflict
            );
        });
    })
}

#[test]
fn drive_mutations_require_true_logged_in_user() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, false);
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveCreate,
                DriveCreateParams {
                    object_type: DriveObjectType::Workflow,
                    name: "build".to_owned(),
                    content: serde_json::json!({ "command": "cargo check" }),
                },
            )
            .expect("drive.create params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                request,
                spoofed_authenticated_grant(ActionKind::DriveCreate),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::AuthenticatedUserUnavailable
            );
        });
    })
}

#[test]
fn drive_mutations_require_underlying_data_mutation_permission() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveCreate,
                DriveCreateParams {
                    object_type: DriveObjectType::Notebook,
                    name: "notes".to_owned(),
                    content: serde_json::json!({ "data": "# Notes" }),
                },
            )
            .expect("drive.create params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response =
                bridge.handle_request(request, authenticated_grant(ActionKind::DriveGet, ctx), ctx);
            assert_eq!(
                response_error_code(response),
                ErrorCode::InsufficientPermissions
            );
        });
    })
}

#[test]
fn drive_mutations_reject_unsupported_targets_and_objects() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let folder_id = create_folder(&mut app, "folder");
        let notebook_id = create_notebook(&mut app, "notes", "# Notes");
        let name_target_request = {
            let mut request = RequestEnvelope::new(
                Action::with_params(
                    ActionKind::DriveCreate,
                    DriveCreateParams {
                        object_type: DriveObjectType::Workflow,
                        name: "build".to_owned(),
                        content: serde_json::json!({ "command": "cargo check" }),
                    },
                )
                .expect("drive.create params serialize"),
            );
            request.target = TargetSelector {
                drive: Some(DriveTarget::Name {
                    object_type: DriveObjectType::Workflow,
                    name: "build".to_owned(),
                }),
                ..TargetSelector::default()
            };
            request
        };
        let mismatched_target_request = {
            let mut request = RequestEnvelope::new(
                Action::with_params(
                    ActionKind::DriveUpdate,
                    DriveUpdateParams {
                        object_type: DriveObjectType::Notebook,
                        id: notebook_id.clone(),
                        content: serde_json::json!({ "data": "updated" }),
                    },
                )
                .expect("drive.update params serialize"),
            );
            request.target = TargetSelector {
                drive: Some(DriveTarget::Id {
                    object_type: DriveObjectType::Workflow,
                    id: DriveObjectSelector(notebook_id),
                }),
                ..TargetSelector::default()
            };
            request
        };
        let unsupported_object_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveDelete,
                DriveDeleteParams {
                    object_type: DriveObjectType::Workflow,
                    id: folder_id,
                },
            )
            .expect("drive.delete params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                name_target_request,
                authenticated_grant(ActionKind::DriveCreate, ctx),
                ctx,
            );
            assert_eq!(response_error_code(response), ErrorCode::UnsupportedAction);

            let response = bridge.handle_request(
                mismatched_target_request,
                authenticated_grant(ActionKind::DriveUpdate, ctx),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::TargetStateConflict
            );

            let response = bridge.handle_request(
                unsupported_object_request,
                authenticated_grant(ActionKind::DriveDelete, ctx),
                ctx,
            );
            assert_eq!(response_error_code(response), ErrorCode::UnsupportedAction);
        });
    })
}

#[test]
fn drive_create_returns_safe_success_for_allowlisted_object_types() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let workflow = response_drive_mutation(
                bridge.handle_request(
                    RequestEnvelope::new(
                        Action::with_params(
                            ActionKind::DriveCreate,
                            DriveCreateParams {
                                object_type: DriveObjectType::Workflow,
                                name: "build".to_owned(),
                                content: serde_json::json!({ "command": "cargo check" }),
                            },
                        )
                        .expect("drive.create workflow params serialize"),
                    ),
                    authenticated_grant(ActionKind::DriveCreate, ctx),
                    ctx,
                ),
            );
            assert_eq!(workflow.object.object_type, DriveObjectType::Workflow);
            assert_eq!(workflow.object.name, "build");

            let notebook = response_drive_mutation(
                bridge.handle_request(
                    RequestEnvelope::new(
                        Action::with_params(
                            ActionKind::DriveCreate,
                            DriveCreateParams {
                                object_type: DriveObjectType::Notebook,
                                name: "notes".to_owned(),
                                content: serde_json::json!({ "data": "# Notes" }),
                            },
                        )
                        .expect("drive.create notebook params serialize"),
                    ),
                    authenticated_grant(ActionKind::DriveCreate, ctx),
                    ctx,
                ),
            );
            assert_eq!(notebook.object.object_type, DriveObjectType::Notebook);
            assert_eq!(notebook.object.name, "notes");

            let environment = response_drive_mutation(
                bridge.handle_request(
                    RequestEnvelope::new(
                        Action::with_params(
                            ActionKind::DriveCreate,
                            DriveCreateParams {
                                object_type: DriveObjectType::Environment,
                                name: "dev".to_owned(),
                                content: serde_json::json!({
                                    "vars": [{ "name": "PORT", "value": { "Constant": "4000" } }]
                                }),
                            },
                        )
                        .expect("drive.create environment params serialize"),
                    ),
                    authenticated_grant(ActionKind::DriveCreate, ctx),
                    ctx,
                ),
            );
            assert_eq!(environment.object.object_type, DriveObjectType::Environment);
            assert_eq!(environment.object.name, "dev");
        });
    })
}

#[test]
fn drive_update_and_delete_return_safe_success() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let notebook_id = create_notebook(&mut app, "notes", "# Notes");
        let environment_id = create_environment(&mut app, "dev");
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let update = response_drive_mutation(bridge.handle_request(
                RequestEnvelope::new(
                    Action::with_params(
                        ActionKind::DriveUpdate,
                        DriveUpdateParams {
                            object_type: DriveObjectType::Notebook,
                            id: notebook_id.clone(),
                            content: serde_json::json!({ "title": "updated", "data": "changed" }),
                        },
                    )
                    .expect("drive.update params serialize"),
                ),
                authenticated_grant(ActionKind::DriveUpdate, ctx),
                ctx,
            ));
            assert_eq!(update.object.name, "updated");

            let delete = response_drive_mutation(
                bridge.handle_request(
                    RequestEnvelope::new(
                        Action::with_params(
                            ActionKind::DriveDelete,
                            DriveDeleteParams {
                                object_type: DriveObjectType::Environment,
                                id: environment_id.clone(),
                            },
                        )
                        .expect("drive.delete params serialize"),
                    ),
                    authenticated_grant(ActionKind::DriveDelete, ctx),
                    ctx,
                ),
            );
            assert_eq!(delete.object.id, environment_id);
            assert_eq!(delete.object.object_type, DriveObjectType::Environment);
        });
    })
}

#[test]
fn drive_run_and_insert_fail_closed_without_policy_approval() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let workflow_id = create_workflow(&mut app, "build", "cargo check");
        let notebook_id = create_notebook(&mut app, "notes", "# Notes");
        let run_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveRun,
                DriveRunParams {
                    object_type: DriveObjectType::Workflow,
                    id: workflow_id,
                },
            )
            .expect("drive.run params serialize"),
        );
        let insert_request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::DriveInsert,
                DriveInsertParams {
                    object_type: DriveObjectType::Notebook,
                    id: notebook_id,
                },
            )
            .expect("drive.insert params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let response = bridge.handle_request(
                run_request,
                authenticated_grant(ActionKind::DriveRun, ctx),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::ExecutionContextNotAllowed
            );

            let response = bridge.handle_request(
                insert_request,
                authenticated_grant(ActionKind::DriveInsert, ctx),
                ctx,
            );
            assert_eq!(
                response_error_code(response),
                ErrorCode::ExecutionContextNotAllowed
            );
        });
    })
}

#[test]
fn read_only_settings_and_appearance_handlers_return_allowlisted_metadata() {
    with_local_control_bridge(|_, ctx| {
        let themes = theme_list_result(ctx).expect("themes are listed");
        assert!(themes.themes.iter().any(|theme| theme.name == "Dark"));

        let appearance = appearance_state_result(ctx).expect("appearance is readable");
        assert_eq!(appearance.theme.as_deref(), Some("Dark"));
        assert_eq!(appearance.light_theme.as_deref(), Some("Light"));
        assert_eq!(appearance.dark_theme.as_deref(), Some("Dark"));
        assert_eq!(appearance.ui_zoom_percent, Some(100));

        let settings = setting_list_result(ctx).expect("settings are listed");
        assert!(settings
            .settings
            .iter()
            .any(|setting| setting.key == "appearance.themes.system_theme"));

        let setting = setting_get_result("appearance.themes.system_theme", ctx)
            .expect("allowlisted setting is readable");
        assert_eq!(setting.setting.value, serde_json::json!(false));
        assert_eq!(setting.setting.value_type, "bool");
    });
}

#[test]
fn settings_and_appearance_mutation_handlers_update_allowlisted_settings() {
    with_local_control_bridge(|_, ctx| {
        let result = theme_set_result(
            ThemeSetParams {
                name: "Light".to_owned(),
            },
            ctx,
        )
        .expect("theme.set succeeds");
        assert!(result.changed);
        let theme =
            setting_get_result("appearance.themes.theme", ctx).expect("theme setting is readable");
        assert_eq!(theme.setting.value, serde_json::json!("Light"));
        let system_theme = setting_get_result("appearance.themes.system_theme", ctx)
            .expect("system theme setting is readable");
        assert_eq!(system_theme.setting.value, serde_json::json!(false));

        let result = appearance_set_result(
            AppearanceSetParams {
                theme: None,
                follow_system_theme: Some(true),
                light_theme: Some("Light".to_owned()),
                dark_theme: Some("Dark".to_owned()),
            },
            ctx,
        )
        .expect("appearance.set succeeds");
        assert!(result.changed);
        let appearance = appearance_state_result(ctx).expect("appearance is readable");
        assert!(appearance.follow_system_theme);
        assert_eq!(appearance.light_theme.as_deref(), Some("Light"));
        assert_eq!(appearance.dark_theme.as_deref(), Some("Dark"));

        let result = appearance_font_size_result(
            AppearanceFontSizeParams {
                adjustment: SizeAdjustment::Set,
                value: Some(14),
            },
            ctx,
        )
        .expect("font size set succeeds");
        assert!(result.changed);
        let font_size = setting_get_result("appearance.text.font_size", ctx)
            .expect("font size setting is readable");
        assert_eq!(font_size.setting.value, serde_json::json!(14.0));

        let result = appearance_zoom_result(
            AppearanceZoomParams {
                adjustment: SizeAdjustment::Set,
                value: Some(125),
            },
            ctx,
        )
        .expect("zoom set succeeds");
        assert!(result.changed);
        let zoom = setting_get_result("appearance.window.zoom_level", ctx)
            .expect("zoom setting is readable");
        assert_eq!(zoom.setting.value, serde_json::json!(125));

        let result = setting_set_result(
            SettingSetParams {
                key: "appearance.text.font_name".to_owned(),
                value: serde_json::json!("JetBrains Mono"),
            },
            ctx,
        )
        .expect("font name setting set succeeds");
        assert_eq!(result.setting.value, serde_json::json!("JetBrains Mono"));

        let before = setting_get_result("terminal.input.syntax_highlighting", ctx)
            .expect("syntax setting is readable")
            .setting
            .value;
        let Some(before) = before.as_bool() else {
            panic!("expected boolean syntax highlighting setting");
        };
        let result = setting_toggle_result(
            SettingToggleParams {
                key: "terminal.input.syntax_highlighting".to_owned(),
            },
            ctx,
        )
        .expect("syntax highlighting toggle succeeds");
        assert_eq!(result.setting.value, serde_json::json!(!before));
    });
}

#[test]
fn settings_and_appearance_mutations_reject_private_unknown_and_invalid_values() {
    with_local_control_bridge(|_, ctx| {
        let err = setting_set_result(
            SettingSetParams {
                key: "terminal.input.not_real".to_owned(),
                value: serde_json::json!(true),
            },
            ctx,
        )
        .expect_err("unknown setting is rejected");
        assert_eq!(err.code, ErrorCode::NotAllowlisted);

        let err = setting_set_result(
            SettingSetParams {
                key: "local_control.allow_outside_warp_control".to_owned(),
                value: serde_json::json!(true),
            },
            ctx,
        )
        .expect_err("private setting is rejected");
        assert_eq!(err.code, ErrorCode::NotAllowlisted);

        let err = appearance_zoom_result(
            AppearanceZoomParams {
                adjustment: SizeAdjustment::Set,
                value: Some(123),
            },
            ctx,
        )
        .expect_err("unsupported zoom level is rejected");
        assert_eq!(err.code, ErrorCode::InvalidParams);

        let err = setting_set_result(
            SettingSetParams {
                key: "terminal.input.syntax_highlighting".to_owned(),
                value: serde_json::json!("yes"),
            },
            ctx,
        )
        .expect_err("bool setting rejects string value");
        assert_eq!(err.code, ErrorCode::InvalidParams);

        let err = setting_toggle_result(
            SettingToggleParams {
                key: "appearance.text.font_name".to_owned(),
            },
            ctx,
        )
        .expect_err("non-bool setting cannot be toggled");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    });
}

#[test]
fn settings_mutations_require_metadata_configuration_permission() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_drive_app(&mut app, true);
        let request = RequestEnvelope::new(
            Action::with_params(
                ActionKind::SettingSet,
                SettingSetParams {
                    key: "terminal.input.syntax_highlighting".to_owned(),
                    value: serde_json::json!(false),
                },
            )
            .expect("setting.set params serialize"),
        );
        LocalControlBridge::handle(&app).update(&mut app, |bridge, ctx| {
            let mut grant = authenticated_grant(ActionKind::SettingSet, ctx);
            grant.permission_category = PermissionCategory::MutateAppState;
            let response = bridge.handle_request(request, grant, ctx);
            assert_eq!(
                response_error_code(response),
                ErrorCode::InsufficientPermissions
            );
        });
    })
}

#[test]
fn setting_get_rejects_unknown_and_private_settings() {
    with_local_control_bridge(|_, ctx| {
        let err = setting_get_result("appearance.secrets.token", ctx)
            .expect_err("unknown settings are rejected");
        assert_eq!(err.code, ErrorCode::NotAllowlisted);

        let err = setting_get_result("local_control.allow_outside_warp_control", ctx)
            .expect_err("private settings are rejected");
        assert_eq!(err.code, ErrorCode::NotAllowlisted);
        assert!(err.message.contains("private or sensitive"));
    });
}

#[test]
fn rejected_setting_key_distinguishes_private_settings() {
    let private_err = rejected_setting_key("terminal.input.inline_menu_custom_content_heights");
    assert_eq!(private_err.code, ErrorCode::NotAllowlisted);
    assert!(private_err.message.contains("private or sensitive"));

    let unknown_err = rejected_setting_key("terminal.input.not_real");
    assert_eq!(unknown_err.code, ErrorCode::NotAllowlisted);
    assert!(unknown_err.message.contains("not an allowlisted"));
}
