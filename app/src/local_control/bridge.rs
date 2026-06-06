//! Bridge between protocol-level control requests and Warp application models.
//!
//! The bridge validates protocol version, selectors, credentials, and settings
//! before routing each supported action to an app-side handler.
use ::local_control::auth::CredentialGrant;
use ::local_control::{
    Action, ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope, ResponseEnvelope,
};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::local_control::handlers::{
    app_state, data, drive, execution, layout, metadata, metadata_config, product_metadata,
    settings_surfaces,
};
use crate::local_control::permissions::{
    ensure_action_allowed, ensure_authenticated_user_matches, ensure_feature_enabled,
    ensure_protocol_version,
};
use crate::local_control::resolver::validate_action_params;

/// WarpUI model that executes already-authenticated local-control actions.
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

    pub(super) fn set_instance_id(&mut self, instance_id: InstanceId) {
        self.instance_id = Some(instance_id);
    }

    pub(super) fn handle_request(
        &mut self,
        request: RequestEnvelope,
        grant: CredentialGrant,
        ctx: &mut ModelContext<Self>,
    ) -> ResponseEnvelope {
        if let Err(error) = ensure_feature_enabled() {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = ensure_protocol_version(request.protocol_version) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        let Some(instance_id) = &self.instance_id else {
            return ResponseEnvelope::error(
                request.request_id,
                ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control bridge has no active instance identity",
                ),
            );
        };
        if let Err(error) = validate_request_authority(instance_id, &request.action, &grant) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) =
            ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
        {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        match request.action.kind {
            ActionKind::InstanceList => match metadata::instance(&self.instance_id) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppPing => match metadata::ping(&self.instance_id) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppVersion => match metadata::version(&self.instance_id) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppActive => {
                ResponseEnvelope::ok(request.request_id, metadata::active(&self.instance_id, ctx))
            }
            ActionKind::InstanceInspect => ResponseEnvelope::ok(
                request.request_id,
                metadata::inspect(&self.instance_id, ctx),
            ),
            ActionKind::CapabilityList => {
                ResponseEnvelope::ok(request.request_id, metadata::capability_list())
            }
            ActionKind::CapabilityInspect => match metadata::capability_inspect(&request.action) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::ActionList => {
                ResponseEnvelope::ok(request.request_id, metadata::action_list())
            }
            ActionKind::ActionInspect => match metadata::action_inspect(&request.action) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::WindowList => match metadata::window_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabList => match metadata::tab_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabInspect => match metadata::tab_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::PaneList => match metadata::pane_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::PaneInspect => match metadata::pane_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SessionList => match metadata::session_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SessionInspect => match metadata::session_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::WindowInspect => match metadata::window_inspect(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::ThemeList => match settings_surfaces::theme_list(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::ThemeGet => match settings_surfaces::theme_get(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::AppearanceGet => match settings_surfaces::appearance_get(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SettingList => match settings_surfaces::setting_list(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::SettingGet => match settings_surfaces::setting_get(&request.action, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::KeybindingList => match settings_surfaces::keybinding_list(ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::KeybindingGet => {
                match settings_surfaces::keybinding_get(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::FileList => match product_metadata::file_list(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::TabCreate => {
                match layout::create_terminal_tab(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppFocus
            | ActionKind::WindowCreate
            | ActionKind::WindowFocus
            | ActionKind::WindowClose
            | ActionKind::TabActivate
            | ActionKind::TabMove
            | ActionKind::TabClose
            | ActionKind::PaneSplit
            | ActionKind::PaneFocus
            | ActionKind::PaneNavigate
            | ActionKind::PaneResize
            | ActionKind::PaneMaximize
            | ActionKind::PaneUnmaximize
            | ActionKind::PaneClose
            | ActionKind::SessionActivate
            | ActionKind::SessionPrevious
            | ActionKind::SessionNext
            | ActionKind::SessionReopenClosed
            | ActionKind::InputInsert
            | ActionKind::InputReplace
            | ActionKind::InputClear
            | ActionKind::InputModeSet
            | ActionKind::SurfaceSettingsOpen
            | ActionKind::SurfaceCommandPaletteOpen
            | ActionKind::SurfaceCommandSearchOpen
            | ActionKind::SurfaceWarpDriveOpen
            | ActionKind::SurfaceWarpDriveToggle
            | ActionKind::SurfaceResourceCenterToggle
            | ActionKind::SurfaceAiAssistantToggle
            | ActionKind::SurfaceCodeReviewToggle
            | ActionKind::SurfaceLeftPanelToggle
            | ActionKind::SurfaceRightPanelToggle
            | ActionKind::SurfaceVerticalTabsToggle
            | ActionKind::FileOpen
            | ActionKind::DriveOpen
            | ActionKind::DriveNotebookOpen
            | ActionKind::DriveEnvVarCollectionOpen
            | ActionKind::DriveObjectShareOpen => {
                match app_state::handle(
                    &self.instance_id,
                    request.action.kind,
                    &request.action.params,
                    &request.target,
                    ctx,
                ) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabRename => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::tab_rename(
                    &self.instance_id,
                    &request.target,
                    &request.action,
                    ctx,
                ) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabResetName => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::tab_reset_name(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabColorSet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::tab_color_set(
                    &self.instance_id,
                    &request.target,
                    &request.action,
                    ctx,
                ) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabColorClear => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::tab_color_clear(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneRename => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::pane_rename(
                    &self.instance_id,
                    &request.target,
                    &request.action,
                    ctx,
                ) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneResetName => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::pane_reset_name(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ThemeSet
            | ActionKind::ThemeSystemSet
            | ActionKind::ThemeLightSet
            | ActionKind::ThemeDarkSet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::theme_set(request.action.kind, &request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceFontSizeIncrease
            | ActionKind::AppearanceFontSizeDecrease
            | ActionKind::AppearanceFontSizeReset
            | ActionKind::AppearanceZoomIncrease
            | ActionKind::AppearanceZoomDecrease
            | ActionKind::AppearanceZoomReset => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::appearance_mutation(request.action.kind, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingSet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::setting_set(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::SettingToggle => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata_config::setting_toggle(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveObjectCreate => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match drive::create_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveObjectUpdate => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match drive::update_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveObjectDelete => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match drive::delete_drive_object(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveObjectInsert => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match drive::insert_drive_object(&request) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveObjectShareToTeam => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match drive::share_drive_object_to_team(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputRun | ActionKind::DriveWorkflowRun => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                let Some(authenticated_user_subject) = grant.authenticated_user.subject.as_deref()
                else {
                    return ResponseEnvelope::error(
                        request.request_id,
                        ControlError::new(
                            ErrorCode::AuthenticatedUserRequired,
                            format!(
                                "{} requires an authenticated Warp user",
                                request.action.kind.as_str()
                            ),
                        ),
                    );
                };
                let result = match request.action.kind {
                    ActionKind::InputRun => {
                        execution::run_input(&request, authenticated_user_subject)
                    }
                    ActionKind::DriveWorkflowRun => {
                        execution::run_drive_workflow(&request, authenticated_user_subject)
                    }
                    action => Err(ControlError::new(
                        ErrorCode::UnsupportedAction,
                        format!("{} is not an execution-underlying action", action.as_str()),
                    )),
                };
                match result {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockList => {
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::list_blocks(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockInspect => {
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::get_block(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockOutput => {
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::get_block(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputGet => match data::get_input_state(&request.target, ctx) {
                Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                Err(error) => ResponseEnvelope::error(request.request_id, error),
            },
            ActionKind::HistoryList => {
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::list_history(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveList => {
                match drive::drive_list(&request.target, &request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveInspect => {
                match drive::drive_inspect(&request.target, &request.action, ctx) {
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
}

pub(crate) fn validate_request_authority(
    instance_id: &InstanceId,
    action: &Action,
    grant: &CredentialGrant,
) -> Result<(), ControlError> {
    grant.verify_for_action(instance_id, action.kind)?;
    if !action.kind.is_implemented() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} is not implemented by this local-control bridge",
                action.kind.as_str()
            ),
        ));
    }
    validate_action_params(action)
}
