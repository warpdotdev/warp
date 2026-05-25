//! Bridge between protocol-level control requests and Warp application models.
//!
//! The bridge validates protocol version, selectors, credentials, and settings
//! before routing each supported action to an app-side handler.
use ::local_control::auth::CredentialGrant;
use ::local_control::{
    ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope, ResponseEnvelope,
    PROTOCOL_VERSION,
};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::local_control::handlers::{
    data, drive, file_mutations, layout, metadata, session_input, settings_surfaces, surfaces,
};
use crate::local_control::permissions::{
    ensure_action_allowed, ensure_authenticated_scripting_grant, ensure_authenticated_user_matches,
    ensure_feature_enabled,
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
        if let Err(error) = ensure_authenticated_user_matches(&grant, ctx) {
            return ResponseEnvelope::error(request.request_id, error);
        }
        if let Err(error) = ensure_authenticated_scripting_grant(&grant, request.action.kind, ctx) {
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
                ResponseEnvelope::ok(request.request_id, metadata::instance(&self.instance_id))
            }
            ActionKind::AppPing => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, metadata::ping(&self.instance_id))
            }
            ActionKind::AppVersion => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, metadata::version(&self.instance_id))
            }
            ActionKind::AppActive => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, metadata::active(&self.instance_id, ctx))
            }
            ActionKind::AppInspect => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(
                    request.request_id,
                    metadata::inspect(&self.instance_id, ctx),
                )
            }
            ActionKind::ActionList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                ResponseEnvelope::ok(request.request_id, metadata::action_list())
            }
            ActionKind::ActionGet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match metadata::action_get(&request.action) {
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
                match metadata::window_list(&request.target, ctx) {
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
                match metadata::tab_list(&request.target, ctx) {
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
                match metadata::pane_list(&request.target, ctx) {
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
                match metadata::session_list(&request.target, ctx) {
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
                match settings_surfaces::theme_list(ctx) {
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
                match settings_surfaces::appearance_get(ctx) {
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
                match settings_surfaces::setting_list(ctx) {
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
                match settings_surfaces::setting_get(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppFocus => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match layout::focus_app(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowCreate => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::create_window(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowFocus => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match layout::focus_window(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::WindowClose => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::close_window(&request.target, params, ctx))
                {
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
                match layout::create_terminal_tab(&self.instance_id, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabActivate => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::activate_tab(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabMove => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::move_tab(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::TabClose => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::close_tab(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneSplit => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::split_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneFocus => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match layout::focus_pane(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneNavigate => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::navigate_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneClose => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match layout::close_pane(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneMaximize => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::maximize_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneResize => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| layout::resize_pane(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::ThemeSet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match settings_surfaces::theme_set(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceSet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match settings_surfaces::appearance_set(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceFontSize => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match settings_surfaces::appearance_font_size(&request.action, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::AppearanceZoom => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match settings_surfaces::appearance_zoom(&request.action, ctx) {
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
                match settings_surfaces::setting_set(&request.action, ctx) {
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
                match settings_surfaces::setting_toggle(&request.action, ctx) {
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
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request.action.params_as().and_then(|params| {
                    surfaces::open_or_toggle_surface(
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
            ActionKind::PaneSessionPrevious | ActionKind::PaneSessionNext => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match session_input::cycle_session(request.action.kind, &request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::PaneSessionReopen => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match session_input::reopen_session(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputInsert => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| session_input::insert_input(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputReplace => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| session_input::replace_input(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputClear => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as::<::local_control::InputClearParams>()
                    .and_then(|_| session_input::clear_input(&request.target, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputModeSet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| session_input::set_input_mode(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::list_blocks(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::BlockGet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::get_block(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::InputGet => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match data::get_input_state(&request.target, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::HistoryList => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match request
                    .action
                    .params_as()
                    .and_then(|params| data::list_history(&request.target, params, ctx))
                {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::FileWrite => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match file_mutations::write_file(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::FileDelete => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match file_mutations::delete_file(&request, ctx) {
                    Ok(data) => ResponseEnvelope::ok(request.request_id, data),
                    Err(error) => ResponseEnvelope::error(request.request_id, error),
                }
            }
            ActionKind::DriveCreate => {
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
            ActionKind::DriveUpdate => {
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
            ActionKind::DriveDelete => {
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
            ActionKind::DriveRun | ActionKind::DriveInsert => {
                if let Err(error) =
                    ensure_action_allowed(grant.invocation_context, request.action.kind, ctx)
                {
                    return ResponseEnvelope::error(request.request_id, error);
                }
                match drive::execute_drive_action_with_policy(&request, ctx) {
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
