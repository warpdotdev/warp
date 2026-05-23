use crate::auth::AuthStateProvider;
use crate::cloud_object::{
    model::{generic_string_model::GenericStringObjectId, persistence::CloudModel},
    CloudObject, GenericStringObjectFormat, JsonObjectType, ObjectType, Owner,
};
use crate::code::view::CodeView;
use crate::env_vars::{CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVarCollection};
use crate::features::FeatureFlag;
use crate::notebooks::{CloudNotebook, CloudNotebookModel, NotebookId};
use crate::palette::PaletteMode;
use crate::pane_group::{Direction as PaneGroupDirection, PaneGroup, PaneGroupAction, PaneId};
use crate::projects::ProjectManagementModel;
use crate::root_view;
use crate::server::ids::{ClientId, SyncId};
use crate::server::telemetry::PaletteSource;
use crate::terminal::input::Input;
use crate::terminal::model::session::SessionId;
use crate::terminal::model::TerminalModel;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::settings::{
    AccessibilitySettings, FontSettings, InputSettings, LocalControlInvocationContext,
    LocalControlPermissionCategory, LocalControlSettings, ThemeSettings,
};
use crate::settings_view::SettingsSection;
use crate::terminal::view::TerminalView;
use crate::terminal::History;
use crate::themes::theme::{SelectedSystemThemes, ThemeKind};
use crate::user_config::WarpConfig;
use crate::window_settings::ZoomLevel;
use crate::workflows::{workflow::Workflow, CloudWorkflow, CloudWorkflowModel, WorkflowId};
use crate::workspace::{ActiveSession, CommandSearchOptions, InitContent};
use crate::WindowSettings;
use ::local_control::auth::{CredentialGrant, CredentialRequest, ScopedCredential};
use ::local_control::protocol::{
    ActionGetParams, ActiveTargetChain, AppFocusParams, AppSurfaceParams, AppearanceFontSizeParams,
    AppearanceMutationResult, AppearanceSetParams, AppearanceStateResult, AppearanceZoomParams,
    BlockGetParams, BlockGetResult, BlockListParams, BlockListResult, BlockSummary,
    DriveCreateParams, DriveDeleteParams, DriveGetParams, DriveGetResult, DriveInsertParams,
    DriveListParams, DriveListResult, DriveMutationResult, DriveObjectSummary,
    DriveObjectType as ControlDriveObjectType, DriveRunParams, DriveTarget, DriveUpdateParams,
    FileDeleteParams, FileListResult, FileMutationResult, FileOpenParams, FileSummary, FileTarget,
    FileWriteParams, HistoryEntrySummary, HistoryListParams, HistoryListResult,
    HorizontalDirection, InputClearParams, InputInsertParams, InputMode, InputModeSetParams,
    InputReplaceParams, InputRunParams, InputStateResult, PaneCloseParams, PaneDirection,
    PaneFocusParams, PaneMaximizeParams, PaneMutationResult, PaneNavigateParams, PaneResizeParams,
    PaneSplitParams, PaneTarget, ProjectActiveResult, ProjectListResult, ProjectSummary,
    SessionTarget, SettingGetParams, SettingGetResult, SettingListResult, SettingMutationResult,
    SettingSetParams, SettingSummary, SettingToggleParams, SizeAdjustment, TabActivateParams,
    TabActivationTarget, TabCloseParams, TabCloseScope, TabMoveParams, TabMutationResult,
    TabRenameParams, TabTarget, TargetSelector, ThemeListResult, ThemeSetParams, ThemeSummary,
    WindowCloseParams, WindowCreateParams, WindowFocusParams, WindowTarget,
};
use ::local_control::{
    ActionKind, AuthToken, ControlEndpoint, ControlError, ControlResponse, ErrorCode,
    ErrorResponseEnvelope, InstanceId, InstanceRecord, RegisteredInstance, RequestEnvelope,
    ResponseEnvelope, PROTOCOL_VERSION,
};
use ::local_control::{InvocationContext, PermissionCategory};
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Duration;
use serde_json::json;
use serde_json::Value;
use settings::Setting as _;
use warp_core::channel::ChannelState;
use warpui::accessibility::AccessibilityVerbosity;
use warpui::platform::TerminationMode;
use warpui::{
    Entity, ModelContext, ModelSpawner, SingletonEntity, TypedActionView, ViewHandle, WindowId,
};

use crate::workspace::{Workspace, WorkspaceAction};
#[cfg(test)]
static TEST_ALLOW_INPUT_RUN_POLICY: AtomicBool = AtomicBool::new(false);

struct ResolvedTerminalTarget {
    terminal_view: ViewHandle<TerminalView>,
}

#[derive(Clone)]
struct TabEntry {
    window_id: WindowId,
    index: usize,
    workspace_active_tab_index: usize,
    pane_group: ViewHandle<PaneGroup>,
}

#[derive(Clone)]
struct PaneEntry {
    tab_id: String,
    index: usize,
    pane_group: ViewHandle<PaneGroup>,
    pane_id: PaneId,
}

#[derive(Clone)]
struct ControlServerState {
    bridge_spawner: ModelSpawner<LocalControlBridge>,
    instance_id: InstanceId,
    credentials: Arc<Mutex<HashMap<String, CredentialGrant>>>,
}

pub struct LocalControlServer {
    _runtime: Option<tokio::runtime::Runtime>,
    _registered_instance: Option<RegisteredInstance>,
}

impl Entity for LocalControlServer {
    type Event = ();
}

impl SingletonEntity for LocalControlServer {}

impl LocalControlServer {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        if !warp_control_cli_enabled() {
            return Self {
                _runtime: None,
                _registered_instance: None,
            };
        }
        match Self::start(ctx) {
            Ok(server) => server,
            Err(error) => {
                log::warn!("Failed to start local-control server: {error:#}");
                Self {
                    _runtime: None,
                    _registered_instance: None,
                }
            }
        }
    }

    fn start(ctx: &mut ModelContext<Self>) -> Result<Self, ControlError> {
        ensure_feature_enabled()?;
        if !outside_warp_any_implemented_action_enabled(ctx) {
            return Err(ControlError::new(
                ErrorCode::LocalControlDisabled,
                "outside-Warp local control is disabled",
            ));
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .build()
            .map_err(|err| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to create local-control runtime",
                    err.to_string(),
                )
            })?;
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))))
            .map_err(|err| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to bind local-control listener",
                    err.to_string(),
                )
            })?;
        let port = listener.local_addr().map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to read local-control listener address",
                err.to_string(),
            )
        })?;
        let outside_warp_control_enabled = LocalControlSettings::as_ref(ctx)
            .is_context_enabled(LocalControlInvocationContext::OutsideWarp);
        let endpoint =
            outside_warp_control_enabled.then_some(ControlEndpoint::localhost(port.port()));
        let record = InstanceRecord::for_current_process(
            endpoint,
            ChannelState::channel().to_string(),
            ChannelState::app_id().to_string(),
            ChannelState::app_version().map(str::to_owned),
            ActionKind::implemented_metadata(),
        );
        let instance_id = record.instance_id.clone();
        let bridge_spawner = LocalControlBridge::handle(ctx).update(ctx, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            ctx.spawner()
        });
        let registered_instance = RegisteredInstance::register(record)?;
        let state = ControlServerState {
            bridge_spawner,
            instance_id,
            credentials: Arc::default(),
        };
        let router = Router::new()
            .route("/v1/control", post(handle_control_request))
            .route("/v1/control/credentials", post(handle_credential_request))
            .with_state(state);
        runtime.spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                log::warn!("local-control listener stopped: {err:#}");
            }
        });
        Ok(Self {
            _runtime: Some(runtime),
            _registered_instance: Some(registered_instance),
        })
    }
}

pub struct LocalControlBridge {
    instance_id: Option<InstanceId>,
}

impl Entity for LocalControlBridge {
    type Event = ();
}

impl SingletonEntity for LocalControlBridge {}

impl LocalControlBridge {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self { instance_id: None }
    }

    fn set_instance_id(&mut self, instance_id: InstanceId) {
        self.instance_id = Some(instance_id);
    }

    fn handle_request(
        &mut self,
        request: RequestEnvelope,
        grant: CredentialGrant,
        ctx: &mut ModelContext<Self>,
    ) -> ResponseEnvelope {
        if let Err(error) = ensure_feature_enabled() {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if request.protocol_version != PROTOCOL_VERSION {
            return ResponseEnvelope::error(
                request.request_id,
                ControlError::new(
                    ErrorCode::ProtocolVersionUnsupported,
                    format!("unsupported protocol version {}", request.protocol_version),
                ),
            );
        }
        if let Err(error) = validate_action_params(&request.action) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = grant.verify_for_action(request.action.kind) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if !request.action.kind.is_implemented() {
            return ResponseEnvelope::error(
                request.request_id,
                ControlError::new(
                    ErrorCode::UnsupportedAction,
                    format!(
                        "{} is not implemented by this local-control bridge",
                        request.action.kind.as_str()
                    ),
                ),
            );
        }
        match request.action.kind {
            ActionKind::InstanceList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, self.instance_metadata())
            }
            ActionKind::AppPing => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, self.ping_metadata())
            }
            ActionKind::AppVersion => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, self.version_metadata())
            }
            ActionKind::AppActive => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, self.active_metadata(ctx))
            }
            ActionKind::AppInspect => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, self.inspect_metadata(ctx))
            }
            ActionKind::ActionList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, self.action_list_metadata())
            }
            ActionKind::ActionGet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.action_get_metadata(&request.action) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.window_list_metadata(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.tab_list_metadata(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ThemeList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match theme_list_result(ctx).and_then(to_control_data) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceGet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match appearance_state_result(ctx).and_then(to_control_data) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingGet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<SettingGetParams>()
                    .and_then(|params| setting_get_result(&params.key, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match setting_list_result(ctx).and_then(to_control_data) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ThemeSet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<ThemeSetParams>()
                    .and_then(|params| theme_set_result(params, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceSet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<AppearanceSetParams>()
                    .and_then(|params| appearance_set_result(params, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceFontSize => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<AppearanceFontSizeParams>()
                    .and_then(|params| appearance_font_size_result(params, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceZoom => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<AppearanceZoomParams>()
                    .and_then(|params| appearance_zoom_result(params, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingSet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<SettingSetParams>()
                    .and_then(|params| setting_set_result(params, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingToggle => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<SettingToggleParams>()
                    .and_then(|params| setting_toggle_result(params, ctx))
                    .and_then(to_control_data)
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::FileList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.list_open_files(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.pane_list_metadata(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SessionList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.session_list_metadata(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ProjectActive => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.active_project(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ProjectList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.list_projects(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputGet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.get_input_state(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::HistoryList => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<HistoryListParams>()
                    .and_then(|params| self.list_history(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppFocus => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.focus_app(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowCreate => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<WindowCreateParams>()
                    .and_then(|params| self.create_window(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowFocus => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.focus_window(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowClose => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<WindowCloseParams>()
                    .and_then(|params| self.close_window(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppSettingsOpen
            | ActionKind::AppCommandPaletteOpen
            | ActionKind::AppCommandSearchOpen
            | ActionKind::AppWarpDriveOpen
            | ActionKind::AppWarpDriveToggle
            | ActionKind::AppResourceCenterToggle
            | ActionKind::AppAiAssistantToggle
            | ActionKind::AppCodeReviewToggle
            | ActionKind::AppVerticalTabsToggle => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<AppSurfaceParams>()
                    .and_then(|params| {
                        self.open_or_toggle_surface(
                            request.action.kind,
                            &request.target,
                            params,
                            ctx,
                        )
                    }) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabCreate => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.create_terminal_tab(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabActivate => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<TabActivateParams>()
                    .and_then(|params| self.activate_tab(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabMove => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<TabMoveParams>()
                    .and_then(|params| self.move_tab(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabRename => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.rename_tab(
                    &request.target,
                    request.action.params_as::<TabRenameParams>(),
                    ctx,
                ) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabClose => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<TabCloseParams>()
                    .and_then(|params| self.close_tab(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneSplit => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<PaneSplitParams>()
                    .and_then(|params| self.split_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneFocus => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.focus_pane(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneNavigate => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<PaneNavigateParams>()
                    .and_then(|params| self.navigate_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneClose => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<PaneCloseParams>()
                    .and_then(|_| self.close_pane(&request.target, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneMaximize => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<PaneMaximizeParams>()
                    .and_then(|params| self.maximize_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneResize => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<PaneResizeParams>()
                    .and_then(|params| self.resize_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputRun => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) = ensure_input_run_policy_allows(&grant, &request.action) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<InputRunParams>()
                    .and_then(|params| self.run_input_command(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockList => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.list_blocks(&request.target, request.action.params_as(), ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockGet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.get_block(&request.target, request.action.params_as(), ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveList => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.list_drive_objects(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveGet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.get_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneSessionPrevious | ActionKind::PaneSessionNext => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.cycle_session(request.action.kind, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputInsert => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<InputInsertParams>()
                    .and_then(|params| self.insert_input(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputReplace => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<InputReplaceParams>()
                    .and_then(|params| self.replace_input(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputClear => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<InputClearParams>()
                    .and_then(|params| {
                        let InputClearParams {} = params;
                        self.clear_input(&request.target, ctx)
                    }) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputModeSet => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<InputModeSetParams>()
                    .and_then(|params| self.set_input_mode(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveCreate => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.create_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveUpdate => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.update_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveDelete => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.delete_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveRun | ActionKind::DriveInsert => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.execute_drive_action_with_policy(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::FileWrite => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.write_file(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::FileDelete => {
                if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match self.delete_file(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            action => ResponseEnvelope::error(
                request.request_id,
                ControlError::new(
                    ErrorCode::UnsupportedAction,
                    format!(
                        "{} is not implemented by this local-control bridge",
                        action.as_str()
                    ),
                ),
            ),
        }
    }

    fn run_input_command(
        &mut self,
        target: &TargetSelector,
        params: InputRunParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::InputRun, target, ctx)?;
        let session_id = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.active_block_session_id())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "input.run requires a target terminal session",
                )
            })?;
        resolved.terminal_view.update(ctx, |terminal, ctx| {
            terminal.execute_command_or_set_pending(&params.command, ctx);
        });
        Ok(json!({
            "action": ActionKind::InputRun.as_str(),
            "submitted": true,
            "session_id": session_id.as_u64().to_string(),
        }))
    }
    fn create_terminal_tab(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_tab_create_target(target)?;
        let window_id = target_window_id(ctx)?;
        let workspace = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|workspaces| workspaces.into_iter().next())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "tab.create requires a workspace in the target window",
                )
            })?;
        let (previous_tab_count, tab_count, active_tab_index) =
            workspace.update(ctx, |workspace, ctx| {
                let previous_tab_count = workspace.tab_count();
                workspace.handle_action(
                    &WorkspaceAction::AddTerminalTab {
                        hide_homepage: false,
                    },
                    ctx,
                );
                (
                    previous_tab_count,
                    workspace.tab_count(),
                    workspace.active_tab_index(),
                )
            });
        Ok(json!({
            "action": ActionKind::TabCreate.as_str(),
            "created": true,
            "instance_id": self.instance_id.as_ref().map(|id| id.0.as_str()),
            "window": {
                "selector": "active",
                "id": window_id.to_string(),
            },
            "tab": {
                "previous_count": previous_tab_count,
                "count": tab_count,
                "active_index": active_tab_index,
            },
        }))
    }

    fn activate_tab(
        &mut self,
        target: &TargetSelector,
        params: TabActivateParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let (window_id, tab_id) = if let Some(relative) = params.relative {
            reject_concrete_tab_selector_for_relative_activation(target)?;
            let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabActivate, ctx)?;
            let workspace = workspace_for_window(ActionKind::TabActivate, entry.window_id, ctx)?;
            workspace.update(ctx, |workspace, ctx| {
                let action = match relative {
                    TabActivationTarget::Previous => WorkspaceAction::ActivatePrevTab,
                    TabActivationTarget::Next => WorkspaceAction::ActivateNextTab,
                    TabActivationTarget::Last => WorkspaceAction::ActivateLastTab,
                };
                workspace.handle_action(&action, ctx);
                let pane_group = workspace.active_tab_pane_group();
                (entry.window_id, pane_group.id().to_string())
            })
        } else {
            let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabActivate, ctx)?;
            let workspace = workspace_for_window(ActionKind::TabActivate, entry.window_id, ctx)?;
            workspace.update(ctx, |workspace, ctx| {
                workspace.handle_action(&WorkspaceAction::ActivateTab(entry.index), ctx);
                (entry.window_id, entry.pane_group.id().to_string())
            })
        };
        to_control_data(TabMutationResult {
            tab_id,
            window_id: window_id.to_string(),
        })
    }

    fn move_tab(
        &mut self,
        target: &TargetSelector,
        params: TabMoveParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabMove, ctx)?;
        let workspace = workspace_for_window(ActionKind::TabMove, entry.window_id, ctx)?;
        let tab_count = workspace.read(ctx, |workspace, _| workspace.tab_count());
        match params.direction {
            HorizontalDirection::Left if entry.index == 0 => {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "tab.move cannot move the leftmost tab further left",
                ));
            }
            HorizontalDirection::Right if entry.index + 1 >= tab_count => {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "tab.move cannot move the rightmost tab further right",
                ));
            }
            _ => {}
        }
        let tab_id = entry.pane_group.id().to_string();
        workspace.update(ctx, |workspace, ctx| {
            let action = match params.direction {
                HorizontalDirection::Left => WorkspaceAction::MoveTabLeft(entry.index),
                HorizontalDirection::Right => WorkspaceAction::MoveTabRight(entry.index),
            };
            workspace.handle_action(&action, ctx);
        });
        to_control_data(TabMutationResult {
            tab_id,
            window_id: entry.window_id.to_string(),
        })
    }

    fn close_tab(
        &mut self,
        target: &TargetSelector,
        params: TabCloseParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabClose, ctx)?;
        let workspace = workspace_for_window(ActionKind::TabClose, entry.window_id, ctx)?;
        let tab_count = workspace.read(ctx, |workspace, _| workspace.tab_count());
        match params.scope {
            TabCloseScope::Others if tab_count <= 1 => {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "tab.close others requires at least one other tab",
                ));
            }
            TabCloseScope::Right if entry.index + 1 >= tab_count => {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "tab.close right requires at least one tab to the right",
                ));
            }
            _ => {}
        }
        let tab_id = entry.pane_group.id().to_string();
        workspace.update(ctx, |workspace, ctx| {
            let action = match params.scope {
                TabCloseScope::Target => WorkspaceAction::CloseTab(entry.index),
                TabCloseScope::Others => WorkspaceAction::CloseOtherTabs(entry.index),
                TabCloseScope::Right => WorkspaceAction::CloseTabsRight(entry.index),
            };
            workspace.handle_action(&action, ctx);
        });
        to_control_data(TabMutationResult {
            tab_id,
            window_id: entry.window_id.to_string(),
        })
    }

    fn split_pane(
        &mut self,
        target: &TargetSelector,
        params: PaneSplitParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        if params.profile.is_some() {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "pane.split profile selection is not implemented by this local-control bridge",
            ));
        }
        let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneSplit, ctx)?;
        let direction = pane_direction(params.direction);
        let new_pane_id = entry.pane_group.update(ctx, |pane_group, ctx| {
            let existing_ids = pane_group.visible_pane_ids();
            pane_group.focus_pane_by_id(entry.pane_id, ctx);
            pane_group.handle_action(&PaneGroupAction::Add(direction), ctx);
            pane_group
                .visible_pane_ids()
                .into_iter()
                .find(|pane_id| !existing_ids.contains(pane_id))
        });
        let pane_id = new_pane_id.ok_or_else(|| {
            ControlError::new(
                ErrorCode::TargetStateConflict,
                "pane.split did not create a new pane in the target pane group",
            )
        })?;
        to_control_data(PaneMutationResult {
            pane_id: pane_id.to_string(),
            tab_id: entry.tab_id,
        })
    }

    fn focus_pane(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneFocus, ctx)?;
        entry.pane_group.update(ctx, |pane_group, ctx| {
            pane_group.focus_pane_by_id(entry.pane_id, ctx);
        });
        to_control_data(PaneMutationResult {
            pane_id: entry.pane_id.to_string(),
            tab_id: entry.tab_id,
        })
    }

    fn navigate_pane(
        &mut self,
        target: &TargetSelector,
        params: PaneNavigateParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneNavigate, ctx)?;
        let action = match params.direction {
            PaneDirection::Left => PaneGroupAction::NavigateLeft,
            PaneDirection::Right => PaneGroupAction::NavigateRight,
            PaneDirection::Up => PaneGroupAction::NavigateUp,
            PaneDirection::Down => PaneGroupAction::NavigateDown,
        };
        let focused_pane_id = entry.pane_group.update(ctx, |pane_group, ctx| {
            pane_group.focus_pane_by_id(entry.pane_id, ctx);
            pane_group.handle_action(&action, ctx);
            pane_group.focused_pane_id(ctx)
        });
        if focused_pane_id == entry.pane_id {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "pane.navigate could not find a pane in the requested direction",
            ));
        }
        to_control_data(PaneMutationResult {
            pane_id: focused_pane_id.to_string(),
            tab_id: entry.tab_id,
        })
    }

    fn close_pane(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneClose, ctx)?;
        let pane_id = entry.pane_id.to_string();
        entry.pane_group.update(ctx, |pane_group, ctx| {
            pane_group.close_pane(entry.pane_id, ctx);
        });
        to_control_data(PaneMutationResult {
            pane_id,
            tab_id: entry.tab_id,
        })
    }

    fn maximize_pane(
        &mut self,
        target: &TargetSelector,
        params: PaneMaximizeParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneMaximize, ctx)?;
        let pane_id = entry.pane_id.to_string();
        entry.pane_group.update(ctx, |pane_group, ctx| {
            if pane_group.pane_count() <= 1 {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "pane.maximize requires a split pane group",
                ));
            }
            pane_group.focus_pane_by_id(entry.pane_id, ctx);
            let currently_enabled = pane_group.is_focused_pane_maximized(ctx);
            if params
                .enabled
                .is_none_or(|enabled| enabled != currently_enabled)
            {
                pane_group.handle_action(&PaneGroupAction::ToggleMaximizePane, ctx);
            }
            Ok(())
        })?;
        to_control_data(PaneMutationResult {
            pane_id,
            tab_id: entry.tab_id,
        })
    }

    fn resize_pane(
        &mut self,
        target: &TargetSelector,
        params: PaneResizeParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneResize, ctx)?;
        if params.amount == Some(0) {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "pane.resize amount must be greater than zero",
            ));
        }
        let action = match params.direction {
            PaneDirection::Left => PaneGroupAction::ResizeLeft,
            PaneDirection::Right => PaneGroupAction::ResizeRight,
            PaneDirection::Up => PaneGroupAction::ResizeUp,
            PaneDirection::Down => PaneGroupAction::ResizeDown,
        };
        entry.pane_group.update(ctx, |pane_group, ctx| {
            if pane_group.pane_count() <= 1 {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "pane.resize requires a split pane group",
                ));
            }
            pane_group.focus_pane_by_id(entry.pane_id, ctx);
            let repeat_count = params.amount.unwrap_or(1);
            for _ in 0..repeat_count {
                pane_group.handle_action(&action, ctx);
            }
            Ok(())
        })?;
        to_control_data(PaneMutationResult {
            pane_id: entry.pane_id.to_string(),
            tab_id: entry.tab_id,
        })
    }

    fn rename_tab(
        &mut self,
        target: &TargetSelector,
        params: Result<TabRenameParams, ControlError>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_tab_rename_target(target)?;
        let params = params?;
        let next_title = params.title.as_deref().map(str::trim).map(str::to_owned);
        let entry = select_single_tab_for_mutation(target, ActionKind::TabRename, ctx)?;
        let tab_id = entry.pane_group.id().to_string();
        let window_id = entry.window_id.to_string();
        let previous_title = entry
            .pane_group
            .read(ctx, |pane_group, ctx| pane_group.custom_title(ctx));
        let changed = previous_title != next_title;
        if changed {
            let pane_group = entry.pane_group.clone();
            let index = entry.index;
            let title_for_update = next_title.clone();
            let workspace = workspace_for_window(ActionKind::TabRename, entry.window_id, ctx)?;
            workspace.update(ctx, move |workspace, ctx| match title_for_update {
                Some(title) if index == workspace.active_tab_index() => {
                    workspace.handle_action(&WorkspaceAction::SetActiveTabName(title), ctx);
                }
                Some(title) => {
                    pane_group.update(ctx, |pane_group, ctx| {
                        pane_group.set_title(&title, ctx);
                    });
                    ctx.dispatch_global_action("workspace:save_app", ());
                    ctx.notify();
                }
                None => workspace.handle_action(&WorkspaceAction::ResetTabName(index), ctx),
            });
        }
        Ok(json!({
            "action": ActionKind::TabRename.as_str(),
            "changed": changed,
            "instance_id": self.instance_id.as_ref().map(|id| id.0.as_str()),
            "window_id": window_id,
            "tab_id": tab_id,
            "title": next_title,
        }))
    }
    fn focus_app(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_app_focus_target(target)?;
        let window_id = ctx.windows().activate_app();
        Ok(json!({
            "action": ActionKind::AppFocus.as_str(),
            "focused": true,
            "window_id": window_id.map(|id| id.to_string()),
        }))
    }

    fn create_window(
        &mut self,
        target: &TargetSelector,
        params: WindowCreateParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_window_create_target(target, &params)?;
        let (window_id, _) = root_view::open_new_window_get_handles(None, ctx);
        ctx.windows().show_window_and_focus_app(window_id);
        Ok(json!({
            "action": ActionKind::WindowCreate.as_str(),
            "created": true,
            "window_id": window_id.to_string(),
        }))
    }

    fn focus_window(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let window_id = select_window_for_app_state_target(ActionKind::WindowFocus, target, ctx)?;
        ctx.windows().show_window_and_focus_app(window_id);
        Ok(json!({
            "action": ActionKind::WindowFocus.as_str(),
            "focused": true,
            "window_id": window_id.to_string(),
        }))
    }

    fn close_window(
        &mut self,
        target: &TargetSelector,
        params: WindowCloseParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let window_id = select_window_for_app_state_target(ActionKind::WindowClose, target, ctx)?;
        let termination_mode = if params.force {
            TerminationMode::ForceTerminate
        } else {
            TerminationMode::Cancellable
        };
        ctx.windows().close_window(window_id, termination_mode);
        Ok(json!({
            "action": ActionKind::WindowClose.as_str(),
            "closed": true,
            "force": params.force,
            "window_id": window_id.to_string(),
        }))
    }

    fn open_or_toggle_surface(
        &mut self,
        action: ActionKind,
        target: &TargetSelector,
        params: AppSurfaceParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let window_id = select_window_for_app_state_target(action, target, ctx)?;
        let workspace = workspace_for_window(action, window_id, ctx)?;
        let workspace_action = workspace_action_for_surface(action, params)?;
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_action(&workspace_action, ctx);
        });
        ctx.windows().show_window_and_focus_app(window_id);
        Ok(json!({
            "action": action.as_str(),
            "handled": true,
            "window_id": window_id.to_string(),
        }))
    }

    fn cycle_session(
        &mut self,
        action: ActionKind,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_active_terminal_target(target, action)?;
        let window_id = require_active_window_id_for_action(ctx.windows().active_window(), action)?;
        let workspace = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|workspaces| workspaces.into_iter().next())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    format!(
                        "{} requires a workspace in the target window",
                        action.as_str()
                    ),
                )
            })?;
        let (previous_tab_index, active_tab_index, tab_count) =
            workspace.update(ctx, |workspace, ctx| {
                let previous_tab_index = workspace.active_tab_index();
                let workspace_action = match action {
                    ActionKind::PaneSessionPrevious => WorkspaceAction::CyclePrevSession,
                    ActionKind::PaneSessionNext => WorkspaceAction::CycleNextSession,
                    _ => {
                        return (
                            previous_tab_index,
                            workspace.active_tab_index(),
                            workspace.tab_count(),
                        );
                    }
                };
                workspace.handle_action(&workspace_action, ctx);
                (
                    previous_tab_index,
                    workspace.active_tab_index(),
                    workspace.tab_count(),
                )
            });
        Ok(json!({
            "action": action.as_str(),
            "window_id": window_id.to_string(),
            "previous_tab_index": previous_tab_index,
            "active_tab_index": active_tab_index,
            "tab_count": tab_count,
        }))
    }

    fn insert_input(
        &mut self,
        target: &TargetSelector,
        params: InputInsertParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::InputInsert, target, ctx)?;
        let session_id =
            active_session_id_for_terminal(&resolved.terminal_view, ActionKind::InputInsert, ctx)?;
        let input = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.input().clone());
        if params.replace {
            input.update(ctx, |input, ctx| {
                input.replace_buffer_content(&params.text, ctx);
            });
        } else {
            input.update(ctx, |input, ctx| {
                input.append_to_buffer(&params.text, ctx);
            });
        }
        input_state_result_for_input(input, session_id, ctx).and_then(to_control_data)
    }

    fn replace_input(
        &mut self,
        target: &TargetSelector,
        params: InputReplaceParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::InputReplace, target, ctx)?;
        let session_id =
            active_session_id_for_terminal(&resolved.terminal_view, ActionKind::InputReplace, ctx)?;
        let input = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.input().clone());
        input.update(ctx, |input, ctx| {
            input.replace_buffer_content(&params.text, ctx);
        });
        input_state_result_for_input(input, session_id, ctx).and_then(to_control_data)
    }

    fn clear_input(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::InputClear, target, ctx)?;
        let session_id =
            active_session_id_for_terminal(&resolved.terminal_view, ActionKind::InputClear, ctx)?;
        let input = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.input().clone());
        input.update(ctx, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
        });
        input_state_result_for_input(input, session_id, ctx).and_then(to_control_data)
    }

    fn set_input_mode(
        &mut self,
        target: &TargetSelector,
        params: InputModeSetParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::InputModeSet, target, ctx)?;
        let session_id =
            active_session_id_for_terminal(&resolved.terminal_view, ActionKind::InputModeSet, ctx)?;
        let input = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.input().clone());
        input.update(ctx, |input, ctx| match params.mode {
            InputMode::Terminal => input.set_input_mode_terminal(false, ctx),
            InputMode::Agent => input.set_input_mode_agent(false, ctx),
        });
        input_state_result_for_input(input, session_id, ctx).and_then(to_control_data)
    }

    fn list_open_files(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_instance_metadata_read_target(ActionKind::FileList, target)?;
        to_control_data(FileListResult {
            files: open_file_summaries(ctx),
        })
    }

    fn write_file(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let params = request.action.params_as::<FileWriteParams>()?;
        validate_file_mutation_target(ActionKind::FileWrite, &request.target, &params.path)?;
        let roots = file_mutation_roots(ctx)?;
        let path = resolve_file_mutation_path(ActionKind::FileWrite, &params.path, &roots, true)?;
        if !params.create && !path.exists() {
            return Err(ControlError::new(
                ErrorCode::StaleTarget,
                "file.write cannot resolve the requested file path",
            ));
        }
        if path.exists() && !path.is_file() {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "file.write only supports writing files",
            ));
        }
        fs::write(&path, params.contents).map_err(|err| {
            ControlError::with_details(
                ErrorCode::TargetStateConflict,
                "file.write failed to write the requested file",
                err.to_string(),
            )
        })?;
        to_control_data(FileMutationResult {
            path: path.display().to_string(),
            tab_id: None,
        })
    }

    fn delete_file(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let params = request.action.params_as::<FileDeleteParams>()?;
        validate_file_mutation_target(ActionKind::FileDelete, &request.target, &params.path)?;
        if params.recursive {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "file.delete does not support recursive directory deletion",
            ));
        }
        let roots = file_mutation_roots(ctx)?;
        let path = resolve_file_mutation_path(ActionKind::FileDelete, &params.path, &roots, false)?;
        if !path.is_file() {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "file.delete only supports deleting files",
            ));
        }
        fs::remove_file(&path).map_err(|err| {
            ControlError::with_details(
                ErrorCode::TargetStateConflict,
                "file.delete failed to delete the requested file",
                err.to_string(),
            )
        })?;
        to_control_data(FileMutationResult {
            path: path.display().to_string(),
            tab_id: None,
        })
    }

    fn active_project(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_instance_metadata_read_target(ActionKind::ProjectActive, target)?;
        to_control_data(ProjectActiveResult {
            project: active_project_summary(ctx),
        })
    }

    fn list_projects(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_instance_metadata_read_target(ActionKind::ProjectList, target)?;
        to_control_data(ProjectListResult {
            projects: project_summaries(ctx),
        })
    }

    fn get_input_state(
        &mut self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::InputGet, target, ctx)?;
        let session_id = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.active_block_session_id())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "input.get requires a target terminal session",
                )
            })?;
        let input = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.input().clone());
        let (text, cursor_offset) = input.read(ctx, |input, ctx| {
            let cursor_offset = input
                .editor()
                .as_ref(ctx)
                .start_byte_index_of_first_selection(ctx)
                .as_usize();
            (input.buffer_text(ctx), cursor_offset)
        });
        let cursor_offset = u32::try_from(cursor_offset).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "input cursor offset is too large to encode",
                err.to_string(),
            )
        })?;
        to_control_data(InputStateResult {
            session_id: session_id.as_u64().to_string(),
            text,
            cursor_offset,
        })
    }

    fn list_history(
        &mut self,
        target: &TargetSelector,
        params: HistoryListParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let resolved = resolve_terminal_read_target(ActionKind::HistoryList, target, ctx)?;
        let session_id = resolved
            .terminal_view
            .read(ctx, |terminal, _| terminal.active_block_session_id())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "history.list requires a target terminal session",
                )
            })?;
        let commands = History::as_ref(ctx)
            .is_queryable(&session_id)
            .then(|| {
                History::as_ref(ctx)
                    .commands_shared(session_id)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let start_index = params
            .limit
            .and_then(|limit| usize::try_from(limit).ok())
            .map(|limit| commands.len().saturating_sub(limit))
            .unwrap_or_default();
        let entries = commands
            .iter()
            .enumerate()
            .skip(start_index)
            .map(|(index, entry)| HistoryEntrySummary {
                entry_id: format!("history:{}:{index}", session_id.as_u64()),
                command: entry.command.clone(),
                cwd: entry.pwd.clone(),
            })
            .collect();
        to_control_data(HistoryListResult { entries })
    }

    fn list_drive_objects(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_drive_target(&request.target, request.action.kind)?;
        let params = request.action.params_as::<DriveListParams>()?;
        let cloud_model = CloudModel::as_ref(ctx);
        let mut objects = cloud_model
            .cloud_objects()
            .filter_map(|object| drive_object_summary(object.as_ref()))
            .filter(|summary| {
                params
                    .object_type
                    .is_none_or(|object_type| summary.object_type == object_type)
            })
            .collect::<Vec<_>>();
        objects.sort_by(|a, b| {
            drive_object_type_rank(a.object_type)
                .cmp(&drive_object_type_rank(b.object_type))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        });
        serde_json::to_value(DriveListResult { objects }).map_err(json_response_error)
    }

    fn get_drive_object(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_drive_target(&request.target, request.action.kind)?;
        let params = request.action.params_as::<DriveGetParams>()?;
        if let Some(DriveTarget::Id { object_type, id }) = request.target.drive.as_ref() {
            if *object_type != params.object_type || id.0 != params.id {
                return Err(ControlError::new(
                    ErrorCode::TargetStateConflict,
                    "drive.get target selector does not match the requested Drive object",
                ));
            }
        }
        let cloud_model = CloudModel::as_ref(ctx);
        let object = cloud_model.get_by_uid(&params.id).ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "drive.get could not resolve the requested Drive object id",
            )
        })?;
        drive_object_get_result(object, params.object_type)
            .and_then(|result| serde_json::to_value(result).map_err(json_response_error))
    }

    fn create_drive_object(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_drive_target(&request.target, request.action.kind)?;
        let params = request.action.params_as::<DriveCreateParams>()?;
        if params.name.is_empty() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "drive.create requires a non-empty Drive object name",
            ));
        }
        let client_id = ClientId::new();
        let sync_id = SyncId::ClientId(client_id);
        let owner = authenticated_user_owner(ctx)?;
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match params.object_type {
            ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => {
                let workflow =
                    workflow_from_drive_content(params.object_type, &params.name, params.content)?;
                cloud_model.create_object(
                    sync_id,
                    CloudWorkflow::new_local(
                        CloudWorkflowModel::new(workflow),
                        owner,
                        None,
                        client_id,
                    ),
                    ctx,
                );
                Ok(())
            }
            ControlDriveObjectType::Notebook => {
                let notebook = notebook_from_drive_content(&params.name, params.content, None)?;
                cloud_model.create_object(
                    sync_id,
                    CloudNotebook::new_local(notebook, owner, None, client_id),
                    ctx,
                );
                Ok(())
            }
            ControlDriveObjectType::Environment => {
                let env_vars = env_vars_from_drive_content(&params.name, params.content)?;
                cloud_model.create_object(
                    sync_id,
                    CloudEnvVarCollection::new_local(
                        CloudEnvVarCollectionModel::new(env_vars),
                        owner,
                        None,
                        client_id,
                    ),
                    ctx,
                );
                Ok(())
            }
        })?;
        let cloud_model = CloudModel::as_ref(ctx);
        let object = cloud_model.get_by_uid(&sync_id.uid()).ok_or_else(|| {
            ControlError::new(
                ErrorCode::Internal,
                "drive.create could not resolve the created Drive object",
            )
        })?;
        drive_mutation_result(object, params.object_type)
    }

    fn update_drive_object(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_drive_target(&request.target, request.action.kind)?;
        let params = request.action.params_as::<DriveUpdateParams>()?;
        validate_drive_request_id(&params.id, request.action.kind)?;
        validate_drive_target_matches_params(
            &request.target,
            params.object_type,
            &params.id,
            request.action.kind,
        )?;
        let (sync_id, existing_notebook) = {
            let cloud_model = CloudModel::as_ref(ctx);
            let object = drive_object_for_mutation(
                cloud_model,
                params.object_type,
                &params.id,
                request.action.kind,
            )?;
            (
                object.sync_id(),
                object
                    .as_any()
                    .downcast_ref::<CloudNotebook>()
                    .map(|notebook| notebook.model().clone()),
            )
        };
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match params.object_type {
            ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => {
                let workflow =
                    workflow_from_drive_content(params.object_type, "", params.content.clone())?;
                cloud_model.update_object_from_edit::<WorkflowId, CloudWorkflowModel>(
                    CloudWorkflowModel::new(workflow),
                    sync_id,
                    ctx,
                );
                Ok(())
            }
            ControlDriveObjectType::Notebook => {
                let notebook =
                    notebook_from_drive_content("", params.content.clone(), existing_notebook)?;
                cloud_model.update_object_from_edit::<NotebookId, CloudNotebookModel>(
                    notebook, sync_id, ctx,
                );
                Ok(())
            }
            ControlDriveObjectType::Environment => {
                let env_vars = env_vars_from_drive_content("", params.content.clone())?;
                cloud_model
                    .update_object_from_edit::<GenericStringObjectId, CloudEnvVarCollectionModel>(
                        CloudEnvVarCollectionModel::new(env_vars),
                        sync_id,
                        ctx,
                    );
                Ok(())
            }
        })?;
        let cloud_model = CloudModel::as_ref(ctx);
        let object = drive_object_for_mutation(
            cloud_model,
            params.object_type,
            &params.id,
            request.action.kind,
        )?;
        drive_mutation_result(object, params.object_type)
    }

    fn delete_drive_object(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_drive_target(&request.target, request.action.kind)?;
        let params = request.action.params_as::<DriveDeleteParams>()?;
        validate_drive_request_id(&params.id, request.action.kind)?;
        validate_drive_target_matches_params(
            &request.target,
            params.object_type,
            &params.id,
            request.action.kind,
        )?;
        let (sync_id, summary) = {
            let cloud_model = CloudModel::as_ref(ctx);
            let object = drive_object_for_mutation(
                cloud_model,
                params.object_type,
                &params.id,
                request.action.kind,
            )?;
            let summary = drive_object_summary(object).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "drive.delete does not support this Drive object type",
                )
            })?;
            (object.sync_id(), summary)
        };
        CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
            cloud_model.delete_object(sync_id, ctx);
        });
        to_control_data(DriveMutationResult {
            object: summary,
            execution_id: None,
        })
    }

    fn execute_drive_action_with_policy(
        &mut self,
        request: &RequestEnvelope,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_drive_target(&request.target, request.action.kind)?;
        match request.action.kind {
            ActionKind::DriveRun => {
                let params = request.action.params_as::<DriveRunParams>()?;
                validate_drive_request_id(&params.id, request.action.kind)?;
                validate_drive_target_matches_params(
                    &request.target,
                    params.object_type,
                    &params.id,
                    request.action.kind,
                )?;
                if params.object_type != ControlDriveObjectType::Workflow {
                    return Err(ControlError::new(
                        ErrorCode::UnsupportedAction,
                        "drive.run only supports workflow objects",
                    ));
                }
                ensure_drive_execution_policy_approved(request.action.kind)?;
                let cloud_model = CloudModel::as_ref(ctx);
                let object = drive_object_for_mutation(
                    cloud_model,
                    params.object_type,
                    &params.id,
                    request.action.kind,
                )?;
                drive_mutation_result(object, params.object_type)
            }
            ActionKind::DriveInsert => {
                let params = request.action.params_as::<DriveInsertParams>()?;
                validate_drive_request_id(&params.id, request.action.kind)?;
                validate_drive_target_matches_params(
                    &request.target,
                    params.object_type,
                    &params.id,
                    request.action.kind,
                )?;
                if params.object_type != ControlDriveObjectType::Notebook {
                    return Err(ControlError::new(
                        ErrorCode::UnsupportedAction,
                        "drive.insert only supports notebook objects",
                    ));
                }
                ensure_drive_execution_policy_approved(request.action.kind)?;
                let cloud_model = CloudModel::as_ref(ctx);
                let object = drive_object_for_mutation(
                    cloud_model,
                    params.object_type,
                    &params.id,
                    request.action.kind,
                )?;
                drive_mutation_result(object, params.object_type)
            }
            action => Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                format!("{} is not a Drive execution action", action.as_str()),
            )),
        }
    }
    fn list_blocks(
        &self,
        target: &TargetSelector,
        params: Result<BlockListParams, ControlError>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_block_list_target(target)?;
        let params = params?;
        let terminal = target_terminal_view(ctx)?;
        let result = terminal.read(ctx, |view, _| {
            let session_id = resolve_session_selector(
                target.session.as_ref(),
                view.active_block_session_id(),
                ActionKind::BlockList,
            )?;
            let model = view.model.lock();
            block_list_result_from_model(&model, session_id, target.session.is_some(), params)
        })?;
        serde_json::to_value(result).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to serialize block.list result",
                err.to_string(),
            )
        })
    }

    fn get_block(
        &self,
        target: &TargetSelector,
        params: Result<BlockGetParams, ControlError>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        validate_block_get_target(target)?;
        let params = params?;
        let terminal = target_terminal_view(ctx)?;
        let result = terminal.read(ctx, |view, _| {
            let session_id = resolve_session_selector(
                target.session.as_ref(),
                view.active_block_session_id(),
                ActionKind::BlockGet,
            )?;
            let model = view.model.lock();
            block_get_result_from_model(&model, session_id, &params.block_id)
        })?;
        serde_json::to_value(result).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to serialize block.get result",
                err.to_string(),
            )
        })
    }

    fn instance_metadata(&self) -> serde_json::Value {
        json!({
            "action": ActionKind::InstanceList.as_str(),
            "instance_id": self.instance_id.as_ref().map(|id| id.0.as_str()),
            "pid": std::process::id(),
            "channel": ChannelState::channel().to_string(),
            "app_id": ChannelState::app_id().to_string(),
            "app_version": ChannelState::app_version(),
            "protocol_version": PROTOCOL_VERSION,
            "actions": ActionKind::implemented_metadata(),
        })
    }

    fn ping_metadata(&self) -> serde_json::Value {
        json!({
            "action": ActionKind::AppPing.as_str(),
            "ok": true,
            "instance_id": self.instance_id.as_ref().map(|id| id.0.as_str()),
            "protocol_version": PROTOCOL_VERSION,
        })
    }

    fn version_metadata(&self) -> serde_json::Value {
        json!({
            "action": ActionKind::AppVersion.as_str(),
            "instance_id": self.instance_id.as_ref().map(|id| id.0.as_str()),
            "protocol_version": PROTOCOL_VERSION,
            "channel": ChannelState::channel().to_string(),
            "app_id": ChannelState::app_id().to_string(),
            "app_version": ChannelState::app_version(),
        })
    }

    fn active_metadata(&self, ctx: &mut ModelContext<Self>) -> serde_json::Value {
        json!({
            "action": ActionKind::AppActive.as_str(),
            "active": self.active_chain(ctx),
        })
    }

    fn inspect_metadata(&self, ctx: &mut ModelContext<Self>) -> serde_json::Value {
        json!({
            "action": ActionKind::AppInspect.as_str(),
            "instance_id": self.instance_id.as_ref().map(|id| id.0.as_str()),
            "version": {
                "protocol_version": PROTOCOL_VERSION,
                "channel": ChannelState::channel().to_string(),
                "app_id": ChannelState::app_id().to_string(),
                "app_version": ChannelState::app_version(),
            },
            "active": self.active_chain(ctx),
            "actions": ActionKind::implemented_metadata(),
        })
    }

    fn active_chain(&self, ctx: &mut ModelContext<Self>) -> ActiveTargetChain {
        let instance_id = self.instance_id.as_ref().map(|id| id.0.clone());
        let active_window = ctx.windows().active_window();
        let Some(window_id) = active_window else {
            return ActiveTargetChain {
                instance_id,
                window_id: None,
                tab_id: None,
                pane_id: None,
                session_id: None,
            };
        };
        let window_id_string = window_id.to_string();
        let workspace = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|workspaces| workspaces.into_iter().next());
        let Some(workspace) = workspace else {
            return ActiveTargetChain {
                instance_id,
                window_id: Some(window_id_string),
                tab_id: None,
                pane_id: None,
                session_id: None,
            };
        };
        let (tab_id, pane_id, session_id) = workspace.read(ctx, |workspace, ctx| {
            let pane_group = workspace.active_tab_pane_group();
            let pane_group_ref = pane_group.as_ref(ctx);
            let pane_id = pane_group_ref.focused_pane_id(ctx);
            let session_id = pane_group_ref
                .active_session_id(ctx)
                .map(|session_id| PaneId::from(session_id).to_string());
            (
                Some(pane_group.id().to_string()),
                Some(pane_id.to_string()),
                session_id,
            )
        });
        ActiveTargetChain {
            instance_id,
            window_id: Some(window_id_string),
            tab_id,
            pane_id,
            session_id,
        }
    }

    fn window_list_metadata(
        &self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        let window_ids = select_window_ids(target, false, ActionKind::WindowList, ctx)?;
        let active_window = ctx.windows().active_window();
        let windows = window_ids
            .into_iter()
            .map(|window_id| {
                let title = ctx
                    .views_of_type::<Workspace>(window_id)
                    .and_then(|workspaces| workspaces.into_iter().next())
                    .map(|workspace| {
                        workspace.read(ctx, |workspace, ctx| {
                            workspace
                                .active_tab_pane_group()
                                .as_ref(ctx)
                                .display_title(ctx)
                        })
                    });
                json!({
                    "window_id": window_id.to_string(),
                    "is_active": Some(window_id) == active_window,
                    "title": title,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "action": ActionKind::WindowList.as_str(),
            "windows": windows,
        }))
    }

    fn tab_list_metadata(
        &self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        reject_target_families(
            ActionKind::TabList,
            target.pane.is_some()
                || target.session.is_some()
                || target.block.is_some()
                || target.file.is_some()
                || target.drive.is_some(),
            "pane, session, block, file, or drive selectors",
        )?;
        let tabs = select_tab_entries(target, ActionKind::TabList, ctx)?
            .into_iter()
            .map(|entry| {
                let title = entry
                    .pane_group
                    .read(ctx, |pane_group, ctx| pane_group.display_title(ctx));
                json!({
                    "tab_id": entry.pane_group.id().to_string(),
                    "window_id": entry.window_id.to_string(),
                    "index": entry.index as u32,
                    "is_active": entry.index == entry.workspace_active_tab_index,
                    "title": title,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "action": ActionKind::TabList.as_str(),
            "tabs": tabs,
        }))
    }

    fn pane_list_metadata(
        &self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        reject_target_families(
            ActionKind::PaneList,
            target.session.is_some()
                || target.block.is_some()
                || target.file.is_some()
                || target.drive.is_some(),
            "session, block, file, or drive selectors",
        )?;
        let panes = select_pane_entries(target, ActionKind::PaneList, ctx)?
            .into_iter()
            .map(|entry| {
                let (is_active, has_terminal_session) =
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        (
                            pane_group.focused_pane_id(ctx) == entry.pane_id,
                            pane_group
                                .terminal_view_from_pane_id(entry.pane_id, ctx)
                                .is_some(),
                        )
                    });
                json!({
                    "pane_id": entry.pane_id.to_string(),
                    "tab_id": entry.tab_id,
                    "index": entry.index as u32,
                    "is_active": is_active,
                    "has_terminal_session": has_terminal_session,
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "action": ActionKind::PaneList.as_str(),
            "panes": panes,
        }))
    }

    fn session_list_metadata(
        &self,
        target: &TargetSelector,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, ControlError> {
        reject_target_families(
            ActionKind::SessionList,
            target.block.is_some() || target.file.is_some() || target.drive.is_some(),
            "block, file, or drive selectors",
        )?;
        let session_target = target.session.as_ref();
        let session_id_filter = matches!(session_target, Some(SessionTarget::Id { .. }));
        let sessions = select_pane_entries(target, ActionKind::SessionList, ctx)?
            .into_iter()
            .filter_map(|entry| {
                let (is_active, has_terminal_session) =
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        (
                            pane_group.active_session_id(ctx).map(PaneId::from)
                                == Some(entry.pane_id),
                            pane_group
                                .terminal_view_from_pane_id(entry.pane_id, ctx)
                                .is_some(),
                        )
                    });
                if !has_terminal_session {
                    return None;
                }
                let session_id = entry.pane_id.to_string();
                let matches_session = match session_target {
                    None => true,
                    Some(SessionTarget::Active) => is_active,
                    Some(SessionTarget::Id { id }) => id.0 == session_id,
                };
                matches_session.then(|| {
                    json!({
                        "session_id": session_id,
                        "pane_id": entry.pane_id.to_string(),
                        "is_active": is_active,
                    })
                })
            })
            .collect::<Vec<_>>();
        if session_id_filter && sessions.is_empty() {
            return Err(ControlError::new(
                ErrorCode::StaleTarget,
                "session.list cannot resolve the requested session id",
            ));
        }
        Ok(json!({
            "action": ActionKind::SessionList.as_str(),
            "sessions": sessions,
        }))
    }

    fn action_list_metadata(&self) -> serde_json::Value {
        json!({
            "action": ActionKind::ActionList.as_str(),
            "actions": ActionKind::implemented_metadata(),
        })
    }

    fn action_get_metadata(
        &self,
        action: &::local_control::Action,
    ) -> Result<serde_json::Value, ControlError> {
        let params = action.params_as::<ActionGetParams>()?;
        let metadata = action_metadata_for_name(&params.action)?;
        Ok(json!({
            "action": ActionKind::ActionGet.as_str(),
            "requested_action": params.action,
            "metadata": metadata,
        }))
    }
}

const ALLOWLISTED_SETTING_KEYS: &[&str] = &[
    "accessibility.accessibility_verbosity",
    "appearance.text.font_name",
    "appearance.text.font_size",
    "appearance.themes.dark_theme",
    "appearance.themes.light_theme",
    "appearance.themes.system_theme",
    "appearance.themes.theme",
    "appearance.window.zoom_level",
    "terminal.input.error_underlining_enabled",
    "terminal.input.syntax_highlighting",
];

const PRIVATE_OR_SENSITIVE_SETTING_KEYS: &[&str] = &[
    "local_control.allow_inside_warp_control",
    "local_control.allow_inside_warp_read_only",
    "local_control.allow_inside_warp_read_write",
    "local_control.allow_outside_warp_control",
    "local_control.allow_outside_warp_read_only",
    "local_control.allow_outside_warp_read_write",
    "terminal.input.autosuggestion_accepted_count",
    "terminal.input.inline_menu_custom_content_heights",
    "terminal.input.workflows_box_expanded",
];

fn theme_list_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ThemeListResult, ControlError> {
    let current_theme = active_theme_kind(ThemeSettings::as_ref(ctx), ctx);
    let mut themes = WarpConfig::as_ref(ctx)
        .theme_config()
        .theme_items()
        .map(|(kind, _)| ThemeSummary {
            name: public_theme_name(kind),
            is_current: *kind == current_theme,
        })
        .collect::<Vec<_>>();
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ThemeListResult { themes })
}

fn appearance_state_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<AppearanceStateResult, ControlError> {
    let theme_settings = ThemeSettings::as_ref(ctx);
    let font_settings = FontSettings::as_ref(ctx);
    let window_settings = WindowSettings::as_ref(ctx);
    let system_themes = theme_settings.selected_system_themes.value();
    Ok(AppearanceStateResult {
        theme: Some(public_theme_name(theme_settings.theme_kind.value())),
        follow_system_theme: *theme_settings.use_system_theme.value(),
        light_theme: Some(public_theme_name(&system_themes.light)),
        dark_theme: Some(public_theme_name(&system_themes.dark)),
        font_size: rounded_u32(*font_settings.monospace_font_size.value()),
        ui_zoom_percent: Some(u32::from(*window_settings.zoom_level.value())),
    })
}

fn setting_list_result(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingListResult, ControlError> {
    let settings = ALLOWLISTED_SETTING_KEYS
        .iter()
        .map(|key| setting_summary_for_key(key, ctx))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SettingListResult { settings })
}

fn setting_get_result(
    key: &str,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingGetResult, ControlError> {
    Ok(SettingGetResult {
        setting: setting_summary_for_key(key, ctx)?,
    })
}

fn theme_set_result(
    params: ThemeSetParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<AppearanceMutationResult, ControlError> {
    let theme = theme_kind_for_name(&params.name, ctx)?;
    let changed = ThemeSettings::handle(ctx)
        .update(ctx, |theme_settings, ctx| {
            let changed = *theme_settings.use_system_theme.value()
                || *theme_settings.theme_kind.value() != theme;
            theme_settings.use_system_theme.set_value(false, ctx)?;
            theme_settings.theme_kind.set_value(theme, ctx)?;
            Ok::<_, anyhow::Error>(changed)
        })
        .map_err(|err| settings_write_error(ActionKind::ThemeSet, err))?;
    Ok(AppearanceMutationResult { changed })
}

fn appearance_set_result(
    params: AppearanceSetParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<AppearanceMutationResult, ControlError> {
    if params.theme.is_none()
        && params.follow_system_theme.is_none()
        && params.light_theme.is_none()
        && params.dark_theme.is_none()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "appearance.set requires at least one appearance field",
        ));
    }
    let theme = params
        .theme
        .as_deref()
        .map(|name| theme_kind_for_name(name, ctx))
        .transpose()?;
    let light_theme = params
        .light_theme
        .as_deref()
        .map(|name| theme_kind_for_name(name, ctx))
        .transpose()?;
    let dark_theme = params
        .dark_theme
        .as_deref()
        .map(|name| theme_kind_for_name(name, ctx))
        .transpose()?;
    let changed = ThemeSettings::handle(ctx)
        .update(ctx, |theme_settings, ctx| {
            let mut changed = false;
            if let Some(follow_system_theme) = params.follow_system_theme {
                changed |= *theme_settings.use_system_theme.value() != follow_system_theme;
                theme_settings
                    .use_system_theme
                    .set_value(follow_system_theme, ctx)?;
            }
            if let Some(theme) = theme {
                changed |= *theme_settings.use_system_theme.value();
                changed |= *theme_settings.theme_kind.value() != theme;
                theme_settings.use_system_theme.set_value(false, ctx)?;
                theme_settings.theme_kind.set_value(theme, ctx)?;
            }
            if light_theme.is_some() || dark_theme.is_some() {
                let current = theme_settings.selected_system_themes.value().clone();
                let next = SelectedSystemThemes {
                    light: light_theme.unwrap_or_else(|| current.light.clone()),
                    dark: dark_theme.unwrap_or_else(|| current.dark.clone()),
                };
                changed |= current != next;
                theme_settings.selected_system_themes.set_value(next, ctx)?;
            }
            Ok::<_, anyhow::Error>(changed)
        })
        .map_err(|err| settings_write_error(ActionKind::AppearanceSet, err))?;
    Ok(AppearanceMutationResult { changed })
}

fn appearance_font_size_result(
    params: AppearanceFontSizeParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<AppearanceMutationResult, ControlError> {
    let current = *FontSettings::as_ref(ctx).monospace_font_size.value();
    let next = match params.adjustment {
        SizeAdjustment::Increase => (current + 1.0).clamp(5.0, 25.0),
        SizeAdjustment::Decrease => (current - 1.0).clamp(5.0, 25.0),
        SizeAdjustment::Reset => crate::settings::MonospaceFontSize::default_value(),
        SizeAdjustment::Set => {
            let value = params.value.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "appearance.font_size set requires a value",
                )
            })?;
            valid_font_size(value)?
        }
    };
    let changed = current != next;
    FontSettings::handle(ctx)
        .update(ctx, |font_settings, ctx| {
            font_settings.monospace_font_size.set_value(next, ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::AppearanceFontSize, err))?;
    Ok(AppearanceMutationResult { changed })
}

fn appearance_zoom_result(
    params: AppearanceZoomParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<AppearanceMutationResult, ControlError> {
    let current = *WindowSettings::as_ref(ctx).zoom_level.value();
    let next = match params.adjustment {
        SizeAdjustment::Increase => adjacent_zoom_level(current, true),
        SizeAdjustment::Decrease => adjacent_zoom_level(current, false),
        SizeAdjustment::Reset => ZoomLevel::default_value(),
        SizeAdjustment::Set => {
            let value = params.value.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "appearance.zoom set requires a value",
                )
            })?;
            valid_zoom_level(value)?
        }
    };
    let changed = current != next;
    WindowSettings::handle(ctx)
        .update(ctx, |window_settings, ctx| {
            window_settings.zoom_level.set_value(next, ctx)
        })
        .map_err(|err| settings_write_error(ActionKind::AppearanceZoom, err))?;
    Ok(AppearanceMutationResult { changed })
}

fn setting_set_result(
    params: SettingSetParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingMutationResult, ControlError> {
    set_allowlisted_setting(&params.key, params.value, ctx)?;
    Ok(SettingMutationResult {
        setting: setting_summary_for_key(&params.key, ctx)?,
    })
}

fn setting_toggle_result(
    params: SettingToggleParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingMutationResult, ControlError> {
    let current = setting_summary_for_key(&params.key, ctx)?;
    let Some(value) = current.value.as_bool() else {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} is not a boolean setting and cannot be toggled",
                params.key
            ),
        ));
    };
    set_allowlisted_setting(&params.key, json!(!value), ctx)?;
    Ok(SettingMutationResult {
        setting: setting_summary_for_key(&params.key, ctx)?,
    })
}

fn setting_summary_for_key(
    key: &str,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SettingSummary, ControlError> {
    let theme_settings = ThemeSettings::as_ref(ctx);
    let font_settings = FontSettings::as_ref(ctx);
    let input_settings = InputSettings::as_ref(ctx);
    let accessibility_settings = AccessibilitySettings::as_ref(ctx);
    let window_settings = WindowSettings::as_ref(ctx);
    match key {
        "appearance.themes.theme" => Ok(setting_summary(
            key,
            json!(public_theme_name(theme_settings.theme_kind.value())),
            "string",
        )),
        "appearance.themes.system_theme" => Ok(setting_summary(
            key,
            json!(*theme_settings.use_system_theme.value()),
            "bool",
        )),
        "appearance.themes.light_theme" => Ok(setting_summary(
            key,
            json!(public_theme_name(
                &theme_settings.selected_system_themes.value().light
            )),
            "string",
        )),
        "appearance.themes.dark_theme" => Ok(setting_summary(
            key,
            json!(public_theme_name(
                &theme_settings.selected_system_themes.value().dark
            )),
            "string",
        )),
        "appearance.text.font_name" => Ok(setting_summary(
            key,
            json!(font_settings.monospace_font_name.value()),
            "string",
        )),
        "appearance.text.font_size" => Ok(setting_summary(
            key,
            json!(*font_settings.monospace_font_size.value()),
            "number",
        )),
        "appearance.window.zoom_level" => Ok(setting_summary(
            key,
            json!(*window_settings.zoom_level.value()),
            "number",
        )),
        "terminal.input.syntax_highlighting" => Ok(setting_summary(
            key,
            json!(*input_settings.syntax_highlighting.value()),
            "bool",
        )),
        "terminal.input.error_underlining_enabled" => Ok(setting_summary(
            key,
            json!(*input_settings.error_underlining.value()),
            "bool",
        )),
        "accessibility.accessibility_verbosity" => Ok(setting_summary(
            key,
            json!(format!(
                "{:?}",
                accessibility_settings.a11y_verbosity.value()
            )),
            "string",
        )),
        _ => Err(rejected_setting_key(key)),
    }
}

fn set_allowlisted_setting(
    key: &str,
    value: Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    match key {
        "appearance.themes.theme" => theme_set_result(
            ThemeSetParams {
                name: string_setting_value(key, &value)?,
            },
            ctx,
        )
        .map(|_| ()),
        "appearance.themes.system_theme" => {
            let enabled = bool_setting_value(key, &value)?;
            ThemeSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.use_system_theme.set_value(enabled, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.themes.light_theme" => {
            let theme = theme_kind_for_name(&string_setting_value(key, &value)?, ctx)?;
            ThemeSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    let current = settings.selected_system_themes.value().clone();
                    settings.selected_system_themes.set_value(
                        SelectedSystemThemes {
                            light: theme,
                            dark: current.dark,
                        },
                        ctx,
                    )
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.themes.dark_theme" => {
            let theme = theme_kind_for_name(&string_setting_value(key, &value)?, ctx)?;
            ThemeSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    let current = settings.selected_system_themes.value().clone();
                    settings.selected_system_themes.set_value(
                        SelectedSystemThemes {
                            light: current.light,
                            dark: theme,
                        },
                        ctx,
                    )
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.text.font_name" => {
            let font_name = string_setting_value(key, &value)?;
            if font_name.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "appearance.text.font_name cannot be empty",
                ));
            }
            FontSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.monospace_font_name.set_value(font_name, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.text.font_size" => {
            let font_size = valid_font_size(u32_setting_value(key, &value)?)?;
            FontSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.monospace_font_size.set_value(font_size, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "appearance.window.zoom_level" => {
            let zoom_level = valid_zoom_level(u32_setting_value(key, &value)?)?;
            WindowSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.zoom_level.set_value(zoom_level, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "terminal.input.syntax_highlighting" => {
            let enabled = bool_setting_value(key, &value)?;
            InputSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.syntax_highlighting.set_value(enabled, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "terminal.input.error_underlining_enabled" => {
            let enabled = bool_setting_value(key, &value)?;
            InputSettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.error_underlining.set_value(enabled, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        "accessibility.accessibility_verbosity" => {
            let verbosity = accessibility_verbosity_value(key, &value)?;
            AccessibilitySettings::handle(ctx)
                .update(ctx, |settings, ctx| {
                    settings.a11y_verbosity.set_value(verbosity, ctx)
                })
                .map_err(|err| settings_write_error(ActionKind::SettingSet, err))
        }
        _ => Err(rejected_setting_key(key)),
    }
}

fn setting_summary(key: &str, value: Value, value_type: &str) -> SettingSummary {
    SettingSummary {
        key: key.to_owned(),
        value,
        value_type: value_type.to_owned(),
    }
}

fn rejected_setting_key(key: &str) -> ControlError {
    if PRIVATE_OR_SENSITIVE_SETTING_KEYS.contains(&key) {
        return ControlError::new(
            ErrorCode::NotAllowlisted,
            format!("{key} is private or sensitive and is not available through local control"),
        );
    }
    ControlError::new(
        ErrorCode::NotAllowlisted,
        format!("{key} is not an allowlisted local-control setting"),
    )
}

fn theme_kind_for_name(
    name: &str,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<ThemeKind, ControlError> {
    let matches = WarpConfig::as_ref(ctx)
        .theme_config()
        .theme_items()
        .filter_map(|(kind, _)| (public_theme_name(kind) == name).then_some(kind.clone()))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [theme] => Ok(theme.clone()),
        [] => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{name} is not an available theme"),
        )),
        _ => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{name} matches multiple themes"),
        )),
    }
}

fn valid_font_size(value: u32) -> Result<f32, ControlError> {
    if (5..=25).contains(&value) {
        return Ok(value as f32);
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        "font size must be between 5 and 25",
    ))
}

fn valid_zoom_level(value: u32) -> Result<u16, ControlError> {
    let value = u16::try_from(value).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "zoom level is outside the supported range",
            err.to_string(),
        )
    })?;
    if ZoomLevel::VALUES.contains(&value) {
        return Ok(value);
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        "zoom level must be one of the supported zoom percentages",
    ))
}

fn adjacent_zoom_level(current: u16, increase: bool) -> u16 {
    let current_index = ZoomLevel::VALUES
        .iter()
        .position(|zoom| *zoom == current)
        .unwrap_or_else(|| {
            ZoomLevel::VALUES
                .iter()
                .position(|zoom| *zoom == ZoomLevel::default_value())
                .unwrap_or(0)
        });
    let next_index = if increase {
        (current_index + 1).min(ZoomLevel::VALUES.len() - 1)
    } else {
        current_index.saturating_sub(1)
    };
    ZoomLevel::VALUES[next_index]
}

fn bool_setting_value(key: &str, value: &Value) -> Result<bool, ControlError> {
    value.as_bool().ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("{key} requires a boolean value"),
        )
    })
}

fn string_setting_value(key: &str, value: &Value) -> Result<String, ControlError> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("{key} requires a string value"),
        )
    })
}

fn u32_setting_value(key: &str, value: &Value) -> Result<u32, ControlError> {
    if let Some(value) = value.as_u64().and_then(|value| u32::try_from(value).ok()) {
        return Ok(value);
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!("{key} requires a non-negative integer value"),
    ))
}

fn accessibility_verbosity_value(
    key: &str,
    value: &Value,
) -> Result<AccessibilityVerbosity, ControlError> {
    match string_setting_value(key, value)?.as_str() {
        "Verbose" | "verbose" | "VERBOSE" => Ok(AccessibilityVerbosity::Verbose),
        "Concise" | "concise" | "CONCISE" => Ok(AccessibilityVerbosity::Concise),
        _ => Err(ControlError::new(
            ErrorCode::InvalidParams,
            "accessibility.accessibility_verbosity must be Verbose or Concise",
        )),
    }
}

fn settings_write_error(action: ActionKind, err: anyhow::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::Internal,
        format!("{} failed to update app settings", action.as_str()),
        err.to_string(),
    )
}
fn public_theme_name(theme: &ThemeKind) -> String {
    match theme {
        ThemeKind::Custom(custom) | ThemeKind::CustomBase16(custom) => custom.name(),
        ThemeKind::InMemory(_) => "In-memory theme".to_owned(),
        _ => theme.to_string(),
    }
}

fn active_theme_kind(
    theme_settings: &ThemeSettings,
    ctx: &ModelContext<LocalControlBridge>,
) -> ThemeKind {
    crate::settings::derived_theme_kind(theme_settings, ctx.system_theme())
}

fn rounded_u32(value: f32) -> Option<u32> {
    if value.is_finite() && value >= 0.0 && value <= u32::MAX as f32 {
        return Some(value.round() as u32);
    }
    None
}

async fn handle_credential_request(
    State(state): State<ControlServerState>,
    payload: Result<Json<CredentialRequest>, JsonRejection>,
) -> Response {
    if let Err(error) = ensure_feature_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let request = match payload {
        Ok(Json(request)) => request,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponseEnvelope::new(ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to decode local-control credential request",
                    err.to_string(),
                ))),
            )
                .into_response();
        }
    };
    if request.protocol_version != PROTOCOL_VERSION {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::ProtocolVersionUnsupported,
                format!("unsupported protocol version {}", request.protocol_version),
            ))),
        )
            .into_response();
    }
    let metadata = request.action.metadata();
    if !request.action.is_implemented() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::UnsupportedAction,
                format!(
                    "{} is not implemented by this local-control bridge",
                    request.action.as_str()
                ),
            ))),
        )
            .into_response();
    }
    if !metadata
        .allowed_invocation_contexts
        .contains(&request.invocation_context)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                format!(
                    "{} cannot run from the requested invocation context",
                    request.action.as_str()
                ),
            ))),
        )
            .into_response();
    }
    if let Err(error) = request.verify_execution_context_proof() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let authorization_check = state
        .bridge_spawner
        .spawn({
            let action = request.action;
            let invocation_context = request.invocation_context;
            move |_, ctx| {
                ensure_action_allowed(invocation_context, action, ctx)?;
                authenticated_user_subject_for_action(action, ctx)
            }
        })
        .await;
    let authenticated_subject = match authorization_check {
        Ok(Ok(subject)) => subject,
        Ok(Err(error)) => {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponseEnvelope::new(ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control app bridge is unavailable",
                ))),
            )
                .into_response();
        }
    };
    let auth_token = AuthToken::generate();
    let mut grant = CredentialGrant::new(
        state.instance_id.clone(),
        request.action,
        request.invocation_context,
        Duration::minutes(5),
    );
    grant.authenticated_user.subject = authenticated_subject;
    let mut credentials = match state.credentials.lock() {
        Ok(credentials) => credentials,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponseEnvelope::new(ControlError::new(
                    ErrorCode::Internal,
                    "local-control credential broker is unavailable",
                ))),
            )
                .into_response();
        }
    };
    credentials.insert(auth_token.secret().to_owned(), grant.clone());
    Json(ScopedCredential {
        bearer_token: auth_token.secret().to_owned(),
        grant,
    })
    .into_response()
}

async fn handle_control_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    payload: Result<Json<RequestEnvelope>, JsonRejection>,
) -> Response {
    if let Err(error) = ensure_feature_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let auth_token = match AuthToken::from_authorization_header(auth_header) {
        Ok(token) => token,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
    };
    let request = match payload {
        Ok(Json(request)) => request,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponseEnvelope::new(ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to decode local-control request",
                    err.to_string(),
                ))),
            )
                .into_response();
        }
    };
    let grant = match state.credentials.lock() {
        Ok(credentials) => credentials.get(auth_token.secret()).cloned(),
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponseEnvelope::new(ControlError::new(
                    ErrorCode::Internal,
                    "local-control credential broker is unavailable",
                ))),
            )
                .into_response();
        }
    };
    let Some(grant) = grant else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponseEnvelope::new(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential is invalid",
            ))),
        )
            .into_response();
    };
    let request_id = request.request_id;
    let response = match state
        .bridge_spawner
        .spawn(move |bridge, ctx| bridge.handle_request(request, grant, ctx))
        .await
    {
        Ok(response) => response,
        Err(_) => ResponseEnvelope::error(
            request_id,
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control app bridge is unavailable",
            ),
        ),
    };
    let status = match &response.response {
        ControlResponse::Ok { .. } => StatusCode::OK,
        ControlResponse::Error { .. } => StatusCode::BAD_REQUEST,
    };
    (status, Json(response)).into_response()
}

fn reject_target_families(
    action: ActionKind,
    rejected: bool,
    families: &str,
) -> Result<(), ControlError> {
    if rejected {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept {families}", action.as_str()),
        ));
    }
    Ok(())
}

fn validate_app_focus_target(target: &TargetSelector) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.block.is_some()
        || target.file.is_some()
        || target.drive.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "app.focus does not accept target selectors",
        ));
    }
    Ok(())
}

fn validate_window_create_target(
    target: &TargetSelector,
    params: &WindowCreateParams,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.block.is_some()
        || target.file.is_some()
        || target.drive.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "window.create does not accept target selectors",
        ));
    }
    if params.profile.is_some() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "window.create does not support selecting a profile yet",
        ));
    }
    Ok(())
}

fn select_window_for_app_state_target(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<WindowId, ControlError> {
    reject_target_families(
        action,
        target.tab.is_some()
            || target.pane.is_some()
            || target.session.is_some()
            || target.block.is_some()
            || target.file.is_some()
            || target.drive.is_some(),
        "tab, pane, session, block, file, or drive selectors",
    )?;
    let window_ids = select_window_ids(target, true, action, ctx)?;
    window_ids.into_iter().next().ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires an active Warp window", action.as_str()),
        )
    })
}

fn workspace_for_window(
    action: ActionKind,
    window_id: WindowId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!(
                    "{} requires a workspace in the target window",
                    action.as_str()
                ),
            )
        })
}

fn workspace_action_for_surface(
    action: ActionKind,
    params: AppSurfaceParams,
) -> Result<WorkspaceAction, ControlError> {
    match action {
        ActionKind::AppSettingsOpen => settings_surface_action(params),
        ActionKind::AppCommandPaletteOpen => command_palette_surface_action(params),
        ActionKind::AppCommandSearchOpen => command_search_surface_action(params),
        ActionKind::AppWarpDriveOpen => {
            no_params_surface_action(action, params, WorkspaceAction::OpenWarpDrive)
        }
        ActionKind::AppWarpDriveToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleWarpDrive)
        }
        ActionKind::AppResourceCenterToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleResourceCenter)
        }
        ActionKind::AppAiAssistantToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleAIAssistant)
        }
        ActionKind::AppCodeReviewToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleRightPanel)
        }
        ActionKind::AppVerticalTabsToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleVerticalTabsPanel)
        }
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not an app surface action", action.as_str()),
        )),
    }
}

fn settings_surface_action(params: AppSurfaceParams) -> Result<WorkspaceAction, ControlError> {
    let section = params
        .page
        .as_deref()
        .map(settings_section_from_param)
        .transpose()?;
    match (section, params.query) {
        (Some(section), Some(query)) => Ok(WorkspaceAction::ShowSettingsPageWithSearch {
            search_query: query,
            section: Some(section),
        }),
        (None, Some(query)) => Ok(WorkspaceAction::ShowSettingsPageWithSearch {
            search_query: query,
            section: None,
        }),
        (Some(section), None) => Ok(WorkspaceAction::ShowSettingsPage(section)),
        (None, None) => Ok(WorkspaceAction::ShowSettings),
    }
}

fn command_palette_surface_action(
    params: AppSurfaceParams,
) -> Result<WorkspaceAction, ControlError> {
    reject_surface_page(ActionKind::AppCommandPaletteOpen, params.page)?;
    Ok(WorkspaceAction::OpenPalette {
        mode: PaletteMode::Command,
        source: PaletteSource::Keybinding,
        query: params.query,
    })
}

fn command_search_surface_action(
    params: AppSurfaceParams,
) -> Result<WorkspaceAction, ControlError> {
    reject_surface_page(ActionKind::AppCommandSearchOpen, params.page)?;
    let init_content = params
        .query
        .map(InitContent::Custom)
        .unwrap_or(InitContent::FromInputBuffer);
    Ok(WorkspaceAction::ShowCommandSearch(CommandSearchOptions {
        filter: None,
        init_content,
    }))
}

fn no_params_surface_action(
    action: ActionKind,
    params: AppSurfaceParams,
    workspace_action: WorkspaceAction,
) -> Result<WorkspaceAction, ControlError> {
    if params.query.is_some() || params.page.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} does not accept query or page parameters",
                action.as_str()
            ),
        ));
    }
    Ok(workspace_action)
}

fn reject_surface_page(action: ActionKind, page: Option<String>) -> Result<(), ControlError> {
    if page.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} does not accept a page parameter", action.as_str()),
        ));
    }
    Ok(())
}

fn settings_section_from_param(page: &str) -> Result<SettingsSection, ControlError> {
    let normalized = page.replace(['-', '_'], " ");
    let mut words = normalized.split_whitespace();
    let title_case = words.try_fold(String::new(), |mut output, word| {
        if !output.is_empty() {
            output.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            output.extend(first.to_uppercase());
            output.push_str(&chars.as_str().to_lowercase());
        }
        Some(output)
    });
    let mut candidates = vec![page.to_owned(), normalized];
    if let Some(title_case) = title_case {
        candidates.push(title_case);
    }
    candidates
        .iter()
        .find_map(|candidate| <SettingsSection as std::str::FromStr>::from_str(candidate).ok())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidParams,
                format!("unknown settings page {page}"),
            )
        })
}
fn select_window_ids(
    target: &TargetSelector,
    force_active_default: bool,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<WindowId>, ControlError> {
    if action == ActionKind::WindowList {
        reject_target_families(
            action,
            target.tab.is_some()
                || target.pane.is_some()
                || target.session.is_some()
                || target.block.is_some()
                || target.file.is_some()
                || target.drive.is_some(),
            "tab, pane, session, block, file, or drive selectors",
        )?;
    }
    match target.window.as_ref() {
        None if force_active_default => {
            let window_id =
                require_active_window_id_for_action(ctx.windows().active_window(), action)?;
            Ok(vec![window_id])
        }
        None => Ok(ctx.window_ids().collect()),
        Some(WindowTarget::Active) => {
            let window_id =
                require_active_window_id_for_action(ctx.windows().active_window(), action)?;
            Ok(vec![window_id])
        }
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .map(|window_id| vec![window_id])
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested window id", action.as_str()),
                )
            }),
        Some(WindowTarget::Index { .. } | WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active and opaque window id selectors",
                action.as_str()
            ),
        )),
    }
}

fn select_tab_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<TabEntry>, ControlError> {
    let force_active_window = matches!(
        target.tab,
        Some(TabTarget::Active | TabTarget::Index { .. })
    ) || matches!(
        target.pane,
        Some(PaneTarget::Active | PaneTarget::Index { .. })
    ) || matches!(target.session, Some(SessionTarget::Active));
    let window_ids = select_window_ids(target, force_active_window, action, ctx)?;
    let all_entries = tab_entries_for_windows(window_ids, ctx);
    let requires_active_tab_default = matches!(
        target.pane,
        Some(PaneTarget::Active | PaneTarget::Index { .. })
    ) || matches!(target.session, Some(SessionTarget::Active));
    match target.tab.as_ref() {
        None if requires_active_tab_default => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index == entry.workspace_active_tab_index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active tab", action.as_str()),
                ));
            }
            Ok(entries)
        }
        None => Ok(all_entries),
        Some(TabTarget::Active) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index == entry.workspace_active_tab_index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active tab", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(TabTarget::Id { id }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.pane_group.id().to_string() == id.0)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested tab id", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(TabTarget::Index { index }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested tab index", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(TabTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active, opaque tab id, and tab index selectors",
                action.as_str()
            ),
        )),
    }
}

fn tab_entries_for_windows(
    window_ids: Vec<WindowId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<TabEntry> {
    window_ids
        .into_iter()
        .filter_map(|window_id| {
            let workspace = ctx
                .views_of_type::<Workspace>(window_id)
                .and_then(|workspaces| workspaces.into_iter().next())?;
            Some(workspace.read(ctx, |workspace, _| {
                workspace
                    .tab_views()
                    .enumerate()
                    .map(|(index, pane_group)| TabEntry {
                        window_id,
                        index,
                        workspace_active_tab_index: workspace.active_tab_index(),
                        pane_group: pane_group.clone(),
                    })
                    .collect::<Vec<_>>()
            }))
        })
        .flatten()
        .collect()
}

fn select_pane_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<PaneEntry>, ControlError> {
    let tab_entries = select_tab_entries(target, action, ctx)?;
    let all_entries = pane_entries_for_tabs(tab_entries, ctx);
    match target.pane.as_ref() {
        None if matches!(target.session, Some(SessionTarget::Active)) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| {
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        pane_group.active_session_id(ctx).map(PaneId::from) == Some(entry.pane_id)
                    })
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active terminal session", action.as_str()),
                ));
            }
            Ok(entries)
        }
        None => Ok(all_entries),
        Some(PaneTarget::Active) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| {
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        pane_group.focused_pane_id(ctx) == entry.pane_id
                    })
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active pane", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(PaneTarget::Id { id }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.pane_id.to_string() == id.0)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested pane id", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(PaneTarget::Index { index }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!(
                        "{} cannot resolve the requested pane index",
                        action.as_str()
                    ),
                ));
            }
            Ok(entries)
        }
    }
}

fn pane_entries_for_tabs(
    tab_entries: Vec<TabEntry>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<PaneEntry> {
    tab_entries
        .into_iter()
        .flat_map(|entry| {
            let tab_id = entry.pane_group.id().to_string();
            let pane_group = entry.pane_group.clone();
            entry
                .pane_group
                .read(ctx, |pane_group, _| pane_group.visible_pane_ids())
                .into_iter()
                .enumerate()
                .map(move |(index, pane_id)| PaneEntry {
                    tab_id: tab_id.clone(),
                    index,
                    pane_group: pane_group.clone(),
                    pane_id,
                })
        })
        .collect()
}

fn reject_concrete_tab_selector_for_relative_activation(
    target: &TargetSelector,
) -> Result<(), ControlError> {
    if matches!(
        target.tab,
        Some(TabTarget::Id { .. } | TabTarget::Index { .. } | TabTarget::Title { .. })
    ) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.activate relative navigation only accepts the default or active tab selector",
        ));
    }
    Ok(())
}

fn select_single_tab_entry_for_mutation(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<TabEntry, ControlError> {
    reject_target_families(
        action,
        target.pane.is_some()
            || target.session.is_some()
            || target.block.is_some()
            || target.file.is_some()
            || target.drive.is_some(),
        "pane, session, block, file, or drive selectors",
    )?;
    let mut target = target.clone();
    if target.tab.is_none() {
        target.tab = Some(TabTarget::Active);
    }
    let entries = select_tab_entries(&target, action, ctx)?;
    single_entry(entries, action, "tab")
}

fn select_single_pane_entry_for_mutation(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<PaneEntry, ControlError> {
    reject_target_families(
        action,
        target.session.is_some()
            || target.block.is_some()
            || target.file.is_some()
            || target.drive.is_some(),
        "session, block, file, or drive selectors",
    )?;
    let mut target = target.clone();
    if target.tab.is_none() && target.pane.is_none() {
        target.tab = Some(TabTarget::Active);
    }
    if target.pane.is_none() {
        target.pane = Some(PaneTarget::Active);
    }
    let entries = select_pane_entries(&target, action, ctx)?;
    single_entry(entries, action, "pane")
}

fn single_entry<T>(
    mut entries: Vec<T>,
    action: ActionKind,
    target_name: &str,
) -> Result<T, ControlError> {
    if entries.len() == 1 {
        return Ok(entries.remove(0));
    }
    if entries.is_empty() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a target {target_name}", action.as_str()),
        ));
    }
    Err(ControlError::new(
        ErrorCode::TargetStateConflict,
        format!(
            "{} resolved more than one {target_name}; provide a more specific selector",
            action.as_str()
        ),
    ))
}

fn pane_direction(direction: PaneDirection) -> PaneGroupDirection {
    match direction {
        PaneDirection::Left => PaneGroupDirection::Left,
        PaneDirection::Right => PaneGroupDirection::Right,
        PaneDirection::Up => PaneGroupDirection::Up,
        PaneDirection::Down => PaneGroupDirection::Down,
    }
}

fn select_single_tab_for_mutation(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<TabEntry, ControlError> {
    let mut entries = select_tab_entries(target, action, ctx)?;
    if entries.len() > 1 {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} requires a single target tab; specify an active tab, tab id, tab index, or window",
                action.as_str()
            ),
        ));
    }
    entries.pop().ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a target tab", action.as_str()),
        )
    })
}

fn validate_drive_target(target: &TargetSelector, action: ActionKind) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.block.is_some()
        || target.file.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept window, tab, pane, session, block, or file selectors",
                action.as_str()
            ),
        ));
    }
    match (action, target.drive.as_ref()) {
        (
            ActionKind::DriveList
            | ActionKind::DriveGet
            | ActionKind::DriveCreate
            | ActionKind::DriveUpdate
            | ActionKind::DriveDelete
            | ActionKind::DriveRun
            | ActionKind::DriveInsert,
            None,
        ) => Ok(()),
        (
            ActionKind::DriveGet
            | ActionKind::DriveUpdate
            | ActionKind::DriveDelete
            | ActionKind::DriveRun
            | ActionKind::DriveInsert,
            Some(DriveTarget::Id { id, .. }),
        ) => {
            if id.0.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidSelector,
                    format!(
                        "{} requires a non-empty Drive object id selector",
                        action.as_str()
                    ),
                ));
            }
            Ok(())
        }
        (_, Some(DriveTarget::Name { .. })) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} does not support Drive name selectors yet",
                action.as_str()
            ),
        )),
        _ => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept a Drive selector", action.as_str()),
        )),
    }
}

fn validate_tab_create_target(target: &TargetSelector) -> Result<(), ControlError> {
    if matches!(target.window.as_ref(), Some(WindowTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "tab.create cannot resolve the requested window id",
        ));
    }
    if !matches!(target.window.as_ref(), None | Some(WindowTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create only supports the active window selector",
        ));
    }
    if matches!(target.tab.as_ref(), Some(TabTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "tab.create cannot resolve the requested tab id",
        ));
    }
    if !matches!(target.tab.as_ref(), None | Some(TabTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create does not accept a concrete tab selector",
        ));
    }
    if matches!(target.pane.as_ref(), Some(PaneTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "tab.create cannot resolve the requested pane id",
        ));
    }
    if !matches!(target.pane.as_ref(), None | Some(PaneTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create does not accept a concrete pane selector",
        ));
    }
    if target.session.is_some()
        || target.block.is_some()
        || target.file.is_some()
        || target.drive.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create does not accept session, block, file, or drive selectors",
        ));
    }
    Ok(())
}

fn validate_tab_rename_target(target: &TargetSelector) -> Result<(), ControlError> {
    reject_target_families(
        ActionKind::TabRename,
        target.pane.is_some()
            || target.session.is_some()
            || target.block.is_some()
            || target.file.is_some()
            || target.drive.is_some(),
        "pane, session, block, file, or drive selectors",
    )
}

fn validate_instance_metadata_read_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.block.is_some()
        || target.file.is_some()
        || target.drive.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept target selectors; it only reads state already represented in Warp",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn validate_file_mutation_target(
    action: ActionKind,
    target: &TargetSelector,
    path: &str,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.block.is_some()
        || target.drive.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept window, tab, pane, session, block, or drive selectors",
                action.as_str()
            ),
        ));
    }
    match target.file.as_ref() {
        None => Ok(()),
        Some(FileTarget::Path { path: target_path }) if target_path == path => Ok(()),
        Some(FileTarget::Path { .. }) => Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!(
                "{} file selector does not match the requested path",
                action.as_str()
            ),
        )),
        Some(FileTarget::Id { .. }) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} does not support file id selectors", action.as_str()),
        )),
    }
}

fn file_mutation_roots(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<PathBuf>, ControlError> {
    let mut roots = Vec::new();
    if let Some(path) = active_project_path(ctx) {
        roots.push(PathBuf::from(path));
    }
    ProjectManagementModel::handle(ctx).read(ctx, |model, _ctx| {
        roots.extend(
            model
                .all_projects()
                .map(|project| PathBuf::from(&project.path)),
        );
    });
    let mut canonical_roots = Vec::new();
    for root in roots {
        if let Ok(canonical_root) = root.canonicalize() {
            if canonical_root.is_dir() && !canonical_roots.contains(&canonical_root) {
                canonical_roots.push(canonical_root);
            }
        }
    }
    if canonical_roots.is_empty() {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "file mutations require an active local project or known workspace path",
        ));
    }
    Ok(canonical_roots)
}

fn resolve_file_mutation_path(
    action: ActionKind,
    path: &str,
    allowed_roots: &[PathBuf],
    allow_missing_file: bool,
) -> Result<PathBuf, ControlError> {
    if path.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty path", action.as_str()),
        ));
    }
    let requested = Path::new(path);
    if !path_has_safe_components(requested) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} path must not contain parent traversal", action.as_str()),
        ));
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        let [root] = allowed_roots else {
            return Err(ControlError::new(
                ErrorCode::InvalidSelector,
                format!(
                    "{} requires an absolute path when multiple workspace roots are available",
                    action.as_str()
                ),
            ));
        };
        root.join(requested)
    };
    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|err| {
            ControlError::with_details(
                ErrorCode::StaleTarget,
                format!("{} cannot resolve the requested file path", action.as_str()),
                err.to_string(),
            )
        })?
    } else if allow_missing_file {
        let parent = candidate.parent().ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidSelector,
                format!(
                    "{} requires a path with a parent directory",
                    action.as_str()
                ),
            )
        })?;
        let file_name = candidate.file_name().ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidSelector,
                format!("{} requires a file path", action.as_str()),
            )
        })?;
        let canonical_parent = parent.canonicalize().map_err(|err| {
            ControlError::with_details(
                ErrorCode::StaleTarget,
                format!(
                    "{} cannot resolve the requested parent directory",
                    action.as_str()
                ),
                err.to_string(),
            )
        })?;
        canonical_parent.join(file_name)
    } else {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{} cannot resolve the requested file path", action.as_str()),
        ));
    };
    if !allowed_roots.iter().any(|root| resolved.starts_with(root)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} path is outside the active project or known workspace paths",
                action.as_str()
            ),
        ));
    }
    Ok(resolved)
}

fn path_has_safe_components(path: &Path) -> bool {
    path.components().all(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::Normal(_)
        )
    })
}

fn validate_block_list_target(target: &TargetSelector) -> Result<(), ControlError> {
    validate_active_terminal_target(target, ActionKind::BlockList)?;
    if target.block.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "block.list does not accept a block selector",
        ));
    }
    Ok(())
}

fn validate_block_get_target(target: &TargetSelector) -> Result<(), ControlError> {
    validate_active_terminal_target(target, ActionKind::BlockGet)?;
    if target.block.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "block.get uses its block_id parameter instead of a block selector",
        ));
    }
    Ok(())
}

fn validate_active_terminal_target(
    target: &TargetSelector,
    action: ActionKind,
) -> Result<(), ControlError> {
    let action_name = action.as_str();
    if matches!(target.window.as_ref(), Some(WindowTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested window id"),
        ));
    }
    if !matches!(target.window.as_ref(), None | Some(WindowTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active window selector"),
        ));
    }
    if matches!(target.tab.as_ref(), Some(TabTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested tab id"),
        ));
    }
    if !matches!(target.tab.as_ref(), None | Some(TabTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active tab selector"),
        ));
    }
    if matches!(target.pane.as_ref(), Some(PaneTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested pane id"),
        ));
    }
    if !matches!(target.pane.as_ref(), None | Some(PaneTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active pane selector"),
        ));
    }
    if matches!(
        action,
        ActionKind::PaneSessionPrevious | ActionKind::PaneSessionNext
    ) && (target.session.is_some() || target.block.is_some())
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} does not accept session or block selectors"),
        ));
    }
    if target.file.is_some() || target.drive.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} does not accept file or drive selectors"),
        ));
    }
    Ok(())
}

fn validate_terminal_read_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    let action_name = action.as_str();
    if matches!(target.window.as_ref(), Some(WindowTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested window id"),
        ));
    }
    if !matches!(target.window.as_ref(), None | Some(WindowTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active window selector"),
        ));
    }
    if matches!(target.tab.as_ref(), Some(TabTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested tab id"),
        ));
    }
    if !matches!(target.tab.as_ref(), None | Some(TabTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active tab selector"),
        ));
    }
    if matches!(target.pane.as_ref(), Some(PaneTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested pane id"),
        ));
    }
    if !matches!(target.pane.as_ref(), None | Some(PaneTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active pane selector"),
        ));
    }
    if matches!(target.session.as_ref(), Some(SessionTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested session id"),
        ));
    }
    if !matches!(target.session.as_ref(), None | Some(SessionTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active session selector"),
        ));
    }
    if target.block.is_some() || target.file.is_some() || target.drive.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} does not accept block, file, or drive selectors"),
        ));
    }
    Ok(())
}

fn resolve_terminal_read_target(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ResolvedTerminalTarget, ControlError> {
    validate_terminal_read_target(action, target)?;
    let window_id = require_active_window_id_for_action(ctx.windows().active_window(), action)?;
    if let Some(workspace) = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
    {
        let pane_group = workspace.read(ctx, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        return resolve_terminal_in_pane_group(action, target, pane_group, ctx);
    }
    let terminal_view = ctx
        .views_of_type::<TerminalView>(window_id)
        .and_then(|terminals| terminals.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires a target terminal session", action.as_str()),
            )
        })?;
    Ok(ResolvedTerminalTarget { terminal_view })
}

fn resolve_terminal_in_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: ViewHandle<PaneGroup>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ResolvedTerminalTarget, ControlError> {
    let terminal_view = pane_group.read(ctx, |pane_group, ctx| {
        let pane_id = if matches!(target.pane, Some(PaneTarget::Active)) {
            pane_group.focused_pane_id(ctx)
        } else {
            pane_group
                .active_session_id(ctx)
                .map(PaneId::from)
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::MissingTarget,
                        format!("{} requires an active terminal session", action.as_str()),
                    )
                })?
        };
        let terminal_view = pane_group
            .terminal_view_from_pane_id(pane_id, ctx)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::TargetStateConflict,
                    format!(
                        "{} target pane does not contain a terminal session",
                        action.as_str()
                    ),
                )
            })?;
        Ok::<_, ControlError>(terminal_view)
    })?;
    Ok(ResolvedTerminalTarget { terminal_view })
}

fn active_session_id_for_terminal(
    terminal_view: &ViewHandle<TerminalView>,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SessionId, ControlError> {
    terminal_view
        .read(ctx, |terminal, _| terminal.active_block_session_id())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires a target terminal session", action.as_str()),
            )
        })
}

fn input_state_result_for_input(
    input: ViewHandle<Input>,
    session_id: SessionId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<InputStateResult, ControlError> {
    let (text, cursor_offset) = input.read(ctx, |input, ctx| {
        let cursor_offset = input
            .editor()
            .as_ref(ctx)
            .start_byte_index_of_first_selection(ctx)
            .as_usize();
        (input.buffer_text(ctx), cursor_offset)
    });
    let cursor_offset = u32::try_from(cursor_offset).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "input cursor offset is too large to encode",
            err.to_string(),
        )
    })?;
    Ok(InputStateResult {
        session_id: session_id.as_u64().to_string(),
        text,
        cursor_offset,
    })
}

fn validate_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    match action.kind {
        ActionKind::ActionGet => {
            let params = action.params_as::<ActionGetParams>()?;
            action_metadata_for_name(&params.action)?;
            Ok(())
        }
        ActionKind::SettingGet => action.params_as::<SettingGetParams>().map(|_| ()),
        ActionKind::AppPing
        | ActionKind::AppInspect
        | ActionKind::AppVersion
        | ActionKind::AppActive
        | ActionKind::ActionList
        | ActionKind::WindowList
        | ActionKind::TabList
        | ActionKind::TabCreate
        | ActionKind::PaneList
        | ActionKind::SessionList
        | ActionKind::ThemeList
        | ActionKind::AppearanceGet
        | ActionKind::SettingList
        | ActionKind::FileList
        | ActionKind::ProjectActive
        | ActionKind::ProjectList
        | ActionKind::InputGet => validate_empty_action_params(action),
        ActionKind::BlockList => action.params_as::<BlockListParams>().map(|_| ()),
        ActionKind::BlockGet => action.params_as::<BlockGetParams>().map(|_| ()),
        ActionKind::HistoryList => {
            let params = action.params.as_object().ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "history.list parameters must be an object",
                )
            })?;
            if params.keys().all(|key| key == "limit") {
                action.params_as::<HistoryListParams>()?;
                Ok(())
            } else {
                Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "history.list only accepts an optional limit parameter",
                ))
            }
        }
        ActionKind::DriveList => action.params_as::<DriveListParams>().map(|_| ()),
        ActionKind::DriveGet => action.params_as::<DriveGetParams>().and_then(|params| {
            if params.id.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.get requires a non-empty Drive object id",
                ));
            }
            Ok(())
        }),
        ActionKind::AppFocus => action.params_as::<AppFocusParams>().map(|_| ()),
        ActionKind::AppSettingsOpen
        | ActionKind::AppCommandPaletteOpen
        | ActionKind::AppCommandSearchOpen
        | ActionKind::AppWarpDriveOpen
        | ActionKind::AppWarpDriveToggle
        | ActionKind::AppResourceCenterToggle
        | ActionKind::AppAiAssistantToggle
        | ActionKind::AppCodeReviewToggle
        | ActionKind::AppVerticalTabsToggle => action.params_as::<AppSurfaceParams>().map(|_| ()),
        ActionKind::WindowCreate => action.params_as::<WindowCreateParams>().map(|_| ()),
        ActionKind::WindowFocus => action.params_as::<WindowFocusParams>().map(|_| ()),
        ActionKind::WindowClose => action.params_as::<WindowCloseParams>().map(|_| ()),
        ActionKind::TabActivate => action.params_as::<TabActivateParams>().map(|_| ()),
        ActionKind::TabMove => action.params_as::<TabMoveParams>().map(|_| ()),
        ActionKind::TabRename => action.params_as::<TabRenameParams>().and_then(|params| {
            if params
                .title
                .as_deref()
                .is_some_and(|title| title.trim().is_empty())
            {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "tab.rename title must be non-empty when provided",
                ));
            }
            Ok(())
        }),
        ActionKind::TabClose => action.params_as::<TabCloseParams>().map(|_| ()),
        ActionKind::PaneSplit => action.params_as::<PaneSplitParams>().map(|_| ()),
        ActionKind::PaneFocus => action.params_as::<PaneFocusParams>().map(|_| ()),
        ActionKind::PaneNavigate => action.params_as::<PaneNavigateParams>().map(|_| ()),
        ActionKind::PaneClose => action.params_as::<PaneCloseParams>().map(|_| ()),
        ActionKind::PaneMaximize => action.params_as::<PaneMaximizeParams>().map(|_| ()),
        ActionKind::PaneResize => action.params_as::<PaneResizeParams>().map(|_| ()),
        ActionKind::PaneSessionPrevious | ActionKind::PaneSessionNext => {
            validate_empty_action_params(action)
        }
        ActionKind::InputInsert => action.params_as::<InputInsertParams>().map(|_| ()),
        ActionKind::InputReplace => action.params_as::<InputReplaceParams>().map(|_| ()),
        ActionKind::InputClear => action.params_as::<InputClearParams>().map(|_| ()),
        ActionKind::InputModeSet => action.params_as::<InputModeSetParams>().map(|_| ()),
        ActionKind::InputRun => action.params_as::<InputRunParams>().and_then(|params| {
            if params.command.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "input.run requires a non-empty command",
                ));
            }
            Ok(())
        }),
        ActionKind::ThemeSet => action.params_as::<ThemeSetParams>().map(|_| ()),
        ActionKind::AppearanceSet => action.params_as::<AppearanceSetParams>().map(|_| ()),
        ActionKind::AppearanceFontSize => {
            action.params_as::<AppearanceFontSizeParams>().map(|_| ())
        }
        ActionKind::AppearanceZoom => action.params_as::<AppearanceZoomParams>().map(|_| ()),
        ActionKind::SettingSet => action.params_as::<SettingSetParams>().map(|_| ()),
        ActionKind::SettingToggle => action.params_as::<SettingToggleParams>().map(|_| ()),
        ActionKind::FileOpen => action.params_as::<FileOpenParams>().map(|_| ()),
        ActionKind::FileWrite => action.params_as::<FileWriteParams>().map(|_| ()),
        ActionKind::FileDelete => action.params_as::<FileDeleteParams>().map(|_| ()),
        ActionKind::DriveCreate => action.params_as::<DriveCreateParams>().map(|_| ()),
        ActionKind::DriveUpdate => action.params_as::<DriveUpdateParams>().map(|_| ()),
        ActionKind::DriveDelete => action.params_as::<DriveDeleteParams>().map(|_| ()),
        ActionKind::DriveRun => action.params_as::<DriveRunParams>().map(|_| ()),
        ActionKind::DriveInsert => action.params_as::<DriveInsertParams>().map(|_| ()),
        _ => Ok(()),
    }
}

fn validate_empty_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    if action
        .params
        .as_object()
        .is_some_and(serde_json::Map::is_empty)
    {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!("{} does not accept parameters", action.kind.as_str()),
    ))
}

fn action_metadata_for_name(
    action_name: &str,
) -> Result<::local_control::ActionMetadata, ControlError> {
    ActionKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == action_name)
        .map(ActionKind::metadata)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::NotAllowlisted,
                format!("{action_name} is not an allowlisted local-control action"),
            )
        })
}

fn open_file_summaries(ctx: &mut ModelContext<LocalControlBridge>) -> Vec<FileSummary> {
    let window_ids: Vec<_> = ctx.window_ids().collect();
    let mut files = Vec::new();
    for window_id in window_ids {
        let Some(code_views) = ctx.views_of_type::<CodeView>(window_id) else {
            continue;
        };
        for code_view in code_views {
            code_view.read(ctx, |code_view, _ctx| {
                for index in 0..code_view.tab_count() {
                    if let Some(location) = code_view.tab_at(index).and_then(|tab| tab.location()) {
                        files.push(FileSummary {
                            path: location.display_path(),
                            tab_id: None,
                        });
                    }
                }
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn active_project_path(ctx: &mut ModelContext<LocalControlBridge>) -> Option<String> {
    let window_id = ctx.windows().active_window()?;
    let repo_path = ActiveSession::as_ref(ctx)
        .terminal_view_id(window_id)
        .and_then(|terminal_view_id| ctx.view_with_id::<TerminalView>(window_id, terminal_view_id))
        .and_then(|terminal| {
            terminal
                .as_ref(ctx)
                .current_repo_path()
                .map(|path| path.display_path())
        });
    repo_path.or_else(|| {
        ActiveSession::as_ref(ctx)
            .working_directory(window_id)
            .map(|path| path.display_path())
    })
}

fn active_project_summary(ctx: &mut ModelContext<LocalControlBridge>) -> Option<ProjectSummary> {
    active_project_path(ctx).map(|path| ProjectSummary {
        path,
        is_active: true,
        last_opened_at: None,
    })
}

fn project_summaries(ctx: &mut ModelContext<LocalControlBridge>) -> Vec<ProjectSummary> {
    let active_path = active_project_path(ctx);
    let mut projects = BTreeMap::new();
    ProjectManagementModel::handle(ctx).read(ctx, |model, _ctx| {
        for project in model.all_projects() {
            projects.insert(
                project.path.clone(),
                ProjectSummary {
                    path: project.path.clone(),
                    is_active: active_path.as_ref() == Some(&project.path),
                    last_opened_at: project
                        .last_opened_ts
                        .map(|timestamp| timestamp.to_string()),
                },
            );
        }
    });
    if let Some(path) = active_path {
        projects.entry(path.clone()).or_insert(ProjectSummary {
            path,
            is_active: true,
            last_opened_at: None,
        });
    }
    projects.into_values().collect()
}

fn to_control_data<T: serde::Serialize>(data: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(data).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode local-control response",
            err.to_string(),
        )
    })
}
fn warp_control_cli_enabled() -> bool {
    FeatureFlag::WarpControlCli.is_enabled()
}

fn ensure_feature_enabled() -> Result<(), ControlError> {
    if warp_control_cli_enabled() {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "Warp control CLI is disabled by feature flag",
    ))
}

fn target_window_id(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<warpui::WindowId, ControlError> {
    require_active_window_id(ctx.windows().active_window())
}

fn require_active_window_id(
    active_window: Option<warpui::WindowId>,
) -> Result<warpui::WindowId, ControlError> {
    require_active_window_id_for_action(active_window, ActionKind::TabCreate)
}

fn require_active_window_id_for_action(
    active_window: Option<warpui::WindowId>,
    action: ActionKind,
) -> Result<warpui::WindowId, ControlError> {
    active_window.ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires an active Warp window", action.as_str()),
        )
    })
}
fn target_terminal_view(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<TerminalView>, ControlError> {
    let window_id = target_window_id(ctx)?;
    let workspace = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "block read requires a workspace in the target window",
            )
        })?;
    workspace
        .read(ctx, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .read(ctx, |pane_group, ctx| pane_group.active_session_view(ctx))
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "block read requires an active terminal session",
            )
        })
}

fn resolve_session_selector(
    target: Option<&SessionTarget>,
    active_session_id: Option<SessionId>,
    action: ActionKind,
) -> Result<SessionId, ControlError> {
    match target {
        None | Some(SessionTarget::Active) => active_session_id.ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires an active terminal session", action.as_str()),
            )
        }),
        Some(SessionTarget::Id { id }) => id.0.parse::<u64>().map(SessionId::from).map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidSelector,
                format!("{} received an invalid session id", action.as_str()),
                err.to_string(),
            )
        }),
    }
}

fn block_summary(
    block: &crate::terminal::model::block::Block,
    index: usize,
) -> Option<BlockSummary> {
    let session_id = block.session_id()?;
    let command = block.command_to_string();
    Some(BlockSummary {
        block_id: block.id().to_string(),
        session_id: session_id.as_u64().to_string(),
        index: index as u32,
        command: (!command.is_empty()).then_some(command),
    })
}

fn block_list_result_from_model(
    model: &TerminalModel,
    session_id: SessionId,
    explicit_session: bool,
    params: BlockListParams,
) -> Result<BlockListResult, ControlError> {
    let mut blocks: Vec<BlockSummary> = model
        .block_list()
        .blocks()
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let summary = block_summary(block, index)?;
            (summary.session_id == session_id.as_u64().to_string()).then_some(summary)
        })
        .collect();
    if explicit_session && blocks.is_empty() {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "block.list cannot resolve the requested session id",
        ));
    }
    if let Some(limit) = params.limit {
        let start = blocks.len().saturating_sub(limit as usize);
        blocks = blocks.split_off(start);
    }
    Ok(BlockListResult { blocks })
}

fn block_get_result_from_model(
    model: &TerminalModel,
    session_id: SessionId,
    block_id: &str,
) -> Result<BlockGetResult, ControlError> {
    model
        .block_list()
        .blocks()
        .iter()
        .enumerate()
        .find_map(|(index, block)| {
            if block.id().as_str() != block_id || block.session_id() != Some(session_id) {
                return None;
            }
            block_summary(block, index).map(|summary| BlockGetResult {
                block: summary,
                output: Some(block.output_to_string()),
            })
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "block.get cannot resolve the requested block id",
            )
        })
}

fn outside_warp_any_implemented_action_enabled(ctx: &ModelContext<LocalControlServer>) -> bool {
    let settings = LocalControlSettings::as_ref(ctx);
    ActionKind::implemented_metadata()
        .into_iter()
        .any(|metadata| {
            outside_warp_permission_enabled_for_settings(settings, metadata.permission_category)
        })
}

#[cfg(test)]
fn outside_warp_action_enabled_for_settings(
    settings: &LocalControlSettings,
    action: ActionKind,
) -> bool {
    outside_warp_permission_enabled_for_settings(settings, action.metadata().permission_category)
}

fn outside_warp_permission_enabled_for_settings(
    settings: &LocalControlSettings,
    permission: PermissionCategory,
) -> bool {
    let context = LocalControlInvocationContext::OutsideWarp;
    settings.is_context_enabled(context)
        && settings.is_permission_enabled(context, local_permission(permission))
}

#[cfg(test)]
fn capabilities() -> Vec<ActionKind> {
    ActionKind::implemented_metadata()
        .into_iter()
        .map(|metadata| metadata.kind)
        .collect()
}

fn local_invocation_context(context: InvocationContext) -> LocalControlInvocationContext {
    match context {
        InvocationContext::InsideWarp => LocalControlInvocationContext::InsideWarp,
        InvocationContext::OutsideWarp => LocalControlInvocationContext::OutsideWarp,
    }
}

fn local_permission(permission: PermissionCategory) -> LocalControlPermissionCategory {
    match permission {
        PermissionCategory::ReadMetadata => LocalControlPermissionCategory::MetadataReads,
        PermissionCategory::ReadUnderlyingData => {
            LocalControlPermissionCategory::UnderlyingDataReads
        }
        PermissionCategory::MutateAppState => LocalControlPermissionCategory::AppStateMutations,
        PermissionCategory::MutateMetadataConfiguration => {
            LocalControlPermissionCategory::MetadataConfigurationMutations
        }
        PermissionCategory::MutateUnderlyingData => {
            LocalControlPermissionCategory::UnderlyingDataMutations
        }
    }
}

fn ensure_action_allowed(
    context: InvocationContext,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    let settings = LocalControlSettings::as_ref(ctx);
    ensure_settings_allow_action(settings, context, action)
}

fn ensure_settings_allow_action(
    settings: &LocalControlSettings,
    context: InvocationContext,
    action: ActionKind,
) -> Result<(), ControlError> {
    let context = local_invocation_context(context);
    if !settings.is_context_enabled(context) {
        return Err(ControlError::new(
            ErrorCode::LocalControlDisabled,
            "local control is disabled for this invocation context",
        ));
    }
    let permission = local_permission(action.metadata().permission_category);
    if !settings.is_permission_enabled(context, permission) {
        return Err(ControlError::new(
            ErrorCode::InsufficientPermissions,
            format!(
                "{} requires a local-control permission that is disabled",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn authenticated_user_subject_for_action(
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Option<String>, ControlError> {
    if !action.metadata().requires_authenticated_user {
        return Ok(None);
    }
    authenticated_user_subject(ctx).map(Some)
}

fn ensure_authenticated_user_matches(
    grant: &CredentialGrant,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if !grant.authenticated_user.required {
        return Ok(());
    }
    let subject = authenticated_user_subject(ctx)?;
    if grant.authenticated_user.subject.as_deref() != Some(subject.as_str()) {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            "the authenticated Warp user no longer matches the credential grant",
        ));
    }
    Ok(())
}

fn ensure_input_run_policy_allows(
    grant: &CredentialGrant,
    action: &::local_control::Action,
) -> Result<(), ControlError> {
    if input_run_policy_allows(grant, action) {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::InsufficientPermissions,
        "input.run requires explicit local approval policy before command execution",
    ))
}

#[cfg(not(test))]
fn input_run_policy_allows(_grant: &CredentialGrant, _action: &::local_control::Action) -> bool {
    false
}

#[cfg(test)]
fn input_run_policy_allows(grant: &CredentialGrant, action: &::local_control::Action) -> bool {
    grant.action == ActionKind::InputRun
        && action.kind == ActionKind::InputRun
        && TEST_ALLOW_INPUT_RUN_POLICY.load(Ordering::SeqCst)
}

#[cfg(test)]
fn allow_input_run_policy_for_test() -> TestInputRunPolicyGuard {
    TestInputRunPolicyGuard {
        previous: TEST_ALLOW_INPUT_RUN_POLICY.swap(true, Ordering::SeqCst),
    }
}

#[cfg(test)]
struct TestInputRunPolicyGuard {
    previous: bool,
}

#[cfg(test)]
impl Drop for TestInputRunPolicyGuard {
    fn drop(&mut self) {
        TEST_ALLOW_INPUT_RUN_POLICY.store(self.previous, Ordering::SeqCst);
    }
}
fn authenticated_user_subject(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<String, ControlError> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            "this action requires a logged-in Warp user",
        ));
    }
    auth_state
        .user_id()
        .map(|uid| uid.as_string())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                "this action requires a logged-in Warp user",
            )
        })
}

fn authenticated_user_owner(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Owner, ControlError> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            "this action requires a logged-in Warp user",
        ));
    }
    auth_state
        .user_id()
        .map(|user_uid| Owner::User { user_uid })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                "this action requires a logged-in Warp user",
            )
        })
}

#[derive(serde::Deserialize)]
struct NotebookDriveContent {
    title: Option<String>,
    data: Option<String>,
}

fn workflow_from_drive_content(
    object_type: ControlDriveObjectType,
    fallback_name: &str,
    content: serde_json::Value,
) -> Result<Workflow, ControlError> {
    if let Ok(mut workflow) = serde_json::from_value::<Workflow>(content.clone()) {
        if workflow_kind_matches(object_type, &workflow) {
            if !fallback_name.is_empty() {
                workflow.set_name(fallback_name);
            }
            return Ok(workflow);
        }
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "Drive workflow content does not match the requested object type",
        ));
    }
    match object_type {
        ControlDriveObjectType::Workflow => {
            let command = content.get("command").and_then(serde_json::Value::as_str);
            let command = command.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.create/update workflow content requires a command string or typed workflow object",
                )
            })?;
            Ok(Workflow::new(fallback_name, command))
        }
        ControlDriveObjectType::Prompt => {
            let query = content.get("query").and_then(serde_json::Value::as_str);
            let query = query.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.create/update prompt content requires a query string or typed workflow object",
                )
            })?;
            Ok(Workflow::AgentMode {
                name: fallback_name.to_owned(),
                query: query.to_owned(),
                description: None,
                arguments: Vec::new(),
            })
        }
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "workflow content is only valid for workflow and prompt Drive object types",
        )),
    }
}

fn workflow_kind_matches(object_type: ControlDriveObjectType, workflow: &Workflow) -> bool {
    match object_type {
        ControlDriveObjectType::Workflow => workflow.is_command_workflow(),
        ControlDriveObjectType::Prompt => workflow.is_agent_mode_workflow(),
        _ => false,
    }
}

fn notebook_from_drive_content(
    fallback_title: &str,
    content: serde_json::Value,
    existing: Option<CloudNotebookModel>,
) -> Result<CloudNotebookModel, ControlError> {
    if let Some(data) = content.as_str() {
        return Ok(CloudNotebookModel {
            title: non_empty_string(fallback_title)
                .or_else(|| existing.as_ref().map(|notebook| notebook.title.clone()))
                .unwrap_or_default(),
            data: data.to_owned(),
            ai_document_id: existing
                .as_ref()
                .and_then(|notebook| notebook.ai_document_id),
            conversation_id: existing.and_then(|notebook| notebook.conversation_id),
        });
    }
    let typed = serde_json::from_value::<NotebookDriveContent>(content).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive.create/update notebook content requires a string or typed notebook object",
            err.to_string(),
        )
    })?;
    Ok(CloudNotebookModel {
        title: typed
            .title
            .or_else(|| non_empty_string(fallback_title))
            .or_else(|| existing.as_ref().map(|notebook| notebook.title.clone()))
            .unwrap_or_default(),
        data: typed
            .data
            .or_else(|| existing.as_ref().map(|notebook| notebook.data.clone()))
            .unwrap_or_default(),
        ai_document_id: existing
            .as_ref()
            .and_then(|notebook| notebook.ai_document_id),
        conversation_id: existing.and_then(|notebook| notebook.conversation_id),
    })
}

fn env_vars_from_drive_content(
    fallback_title: &str,
    content: serde_json::Value,
) -> Result<EnvVarCollection, ControlError> {
    let mut env_vars = serde_json::from_value::<EnvVarCollection>(content).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive.create/update environment content requires a typed environment-variable collection",
            err.to_string(),
        )
    })?;
    if env_vars.title.as_ref().is_none_or(String::is_empty) {
        env_vars.title = non_empty_string(fallback_title);
    }
    Ok(env_vars)
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn validate_drive_request_id(id: &str, action: ActionKind) -> Result<(), ControlError> {
    if id.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty Drive object id", action.as_str()),
        ));
    }
    Ok(())
}

fn validate_drive_target_matches_params(
    target: &TargetSelector,
    object_type: ControlDriveObjectType,
    id: &str,
    action: ActionKind,
) -> Result<(), ControlError> {
    if let Some(DriveTarget::Id {
        object_type: target_type,
        id: target_id,
    }) = target.drive.as_ref()
    {
        if *target_type != object_type || target_id.0 != id {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                format!(
                    "{} target selector does not match the requested Drive object",
                    action.as_str()
                ),
            ));
        }
    }
    Ok(())
}

fn drive_object_for_mutation<'a>(
    cloud_model: &'a CloudModel,
    object_type: ControlDriveObjectType,
    id: &str,
    action: ActionKind,
) -> Result<&'a dyn CloudObject, ControlError> {
    let object_uid = id.to_owned();
    let object = cloud_model.get_by_uid(&object_uid).ok_or_else(|| {
        ControlError::new(
            ErrorCode::StaleTarget,
            format!(
                "{} could not resolve the requested Drive object id",
                action.as_str()
            ),
        )
    })?;
    let summary = drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} does not support this Drive object type",
                action.as_str()
            ),
        )
    })?;
    if summary.object_type != object_type {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!(
                "{} Drive object type does not match the requested type",
                action.as_str()
            ),
        ));
    }
    Ok(object)
}

fn drive_mutation_result(
    object: &dyn CloudObject,
    object_type: ControlDriveObjectType,
) -> Result<serde_json::Value, ControlError> {
    let summary = drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            "Drive mutation does not support this Drive object type",
        )
    })?;
    if summary.object_type != object_type {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "Drive object type does not match the requested type",
        ));
    }
    to_control_data(DriveMutationResult {
        object: summary,
        execution_id: None,
    })
}

fn ensure_drive_execution_policy_approved(action: ActionKind) -> Result<(), ControlError> {
    Err(ControlError::new(
        ErrorCode::ExecutionContextNotAllowed,
        format!(
            "{} requires an explicit approval policy hook, but no approval is available",
            action.as_str()
        ),
    ))
}
fn drive_object_summary(object: &dyn CloudObject) -> Option<DriveObjectSummary> {
    Some(DriveObjectSummary {
        object_type: control_drive_object_type(object)?,
        id: object.uid(),
        name: object.display_name(),
    })
}

fn drive_object_get_result(
    object: &dyn CloudObject,
    requested_type: ControlDriveObjectType,
) -> Result<DriveGetResult, ControlError> {
    let summary = drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            "drive.get does not support this Drive object type",
        )
    })?;
    if summary.object_type != requested_type {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "drive.get Drive object type does not match the requested type",
        ));
    }
    Ok(DriveGetResult {
        object: summary,
        content: drive_object_content(object, requested_type)?,
    })
}

fn control_drive_object_type(object: &dyn CloudObject) -> Option<ControlDriveObjectType> {
    match object.object_type() {
        ObjectType::Workflow => {
            let workflow = object.as_any().downcast_ref::<CloudWorkflow>()?;
            if workflow.model().data.is_agent_mode_workflow() {
                Some(ControlDriveObjectType::Prompt)
            } else {
                Some(ControlDriveObjectType::Workflow)
            }
        }
        ObjectType::Notebook => Some(ControlDriveObjectType::Notebook),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::EnvVarCollection,
        )) => Some(ControlDriveObjectType::Environment),
        _ => None,
    }
}

fn drive_object_content(
    object: &dyn CloudObject,
    object_type: ControlDriveObjectType,
) -> Result<serde_json::Value, ControlError> {
    match object_type {
        ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => object
            .as_any()
            .downcast_ref::<CloudWorkflow>()
            .ok_or_else(drive_type_mismatch_error)
            .and_then(|workflow| {
                serde_json::to_value(&workflow.model().data).map_err(json_response_error)
            }),
        ControlDriveObjectType::Notebook => {
            let notebook = object
                .as_any()
                .downcast_ref::<CloudNotebook>()
                .ok_or_else(drive_type_mismatch_error)?;
            Ok(json!({
                "title": notebook.model().title.clone(),
                "data": notebook.model().data.clone(),
                "ai_document_id": notebook.model().ai_document_id.as_ref().map(|id| id.to_string()),
                "conversation_id": notebook.model().conversation_id.clone(),
            }))
        }
        ControlDriveObjectType::Environment => object
            .as_any()
            .downcast_ref::<CloudEnvVarCollection>()
            .ok_or_else(drive_type_mismatch_error)
            .and_then(|env_var_collection| {
                serde_json::to_value(&env_var_collection.model().string_model)
                    .map_err(json_response_error)
            }),
    }
}

fn drive_object_type_rank(object_type: ControlDriveObjectType) -> u8 {
    match object_type {
        ControlDriveObjectType::Workflow => 0,
        ControlDriveObjectType::Notebook => 1,
        ControlDriveObjectType::Environment => 2,
        ControlDriveObjectType::Prompt => 3,
    }
}

fn drive_type_mismatch_error() -> ControlError {
    ControlError::new(
        ErrorCode::TargetStateConflict,
        "drive.get Drive object type does not match the requested type",
    )
}

fn json_response_error(error: serde_json::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::Internal,
        "failed to encode local-control Drive response",
        error.to_string(),
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
