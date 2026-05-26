//! Target and parameter validation for the first local-control action slice.
use crate::local_control::handlers::metadata::action_metadata_for_name;
use ::local_control::protocol::{
    ActionNameParams, BlockIdParams, BlockListParams, DriveInspectParams, DriveListParams,
    HistoryListParams, BindingNameParams, PaneTarget, SessionTarget, SettingGetParams, TabTarget,
    TargetSelector, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use warpui::ModelContext;

use crate::local_control::LocalControlBridge;

pub(crate) fn validate_tab_create_target(target: &TargetSelector) -> Result<(), ControlError> {
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
    if matches!(target.session.as_ref(), Some(SessionTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "tab.create cannot resolve the requested session id",
        ));
    }
    if !matches!(target.session.as_ref(), None | Some(SessionTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create does not accept a concrete session selector",
        ));
    }
    Ok(())
}

/// Validates action-specific params implemented by this branch stack layer.
///
/// This is intentionally narrow while `zach/warp-cli-core-foundation` is the
/// bottom branch of the stack: later branches add their own params and expand
/// this validation alongside the corresponding action handlers.
pub(crate) fn validate_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    match action.kind {
        ActionKind::ActionInspect | ActionKind::CapabilityInspect => {
            let params = action.params_as::<ActionNameParams>()?;
            action_metadata_for_name(&params.action)?;
            Ok(())
        }
        ActionKind::SettingGet => action.params_as::<SettingGetParams>().map(|_| ()),
        ActionKind::KeybindingGet => action
            .params_as::<BindingNameParams>()
            .and_then(|params| {
                if params.binding_name.is_empty() {
                    return Err(ControlError::new(
                        ErrorCode::InvalidParams,
                        "keybinding.get requires a non-empty name",
                    ));
                }
                Ok(())
            }),
        ActionKind::AppPing
        | ActionKind::InstanceInspect
        | ActionKind::AppVersion
        | ActionKind::AppActive
        | ActionKind::CapabilityList
        | ActionKind::ActionList
        | ActionKind::WindowList
        | ActionKind::WindowInspect
        | ActionKind::TabList
        | ActionKind::TabInspect
        | ActionKind::TabCreate
        | ActionKind::PaneList
        | ActionKind::PaneInspect
        | ActionKind::SessionList
        | ActionKind::SessionInspect
        | ActionKind::InputGet
        | ActionKind::ThemeList
        | ActionKind::ThemeGet
        | ActionKind::AppearanceGet
        | ActionKind::SettingList
        | ActionKind::KeybindingList
        | ActionKind::FileList
        | ActionKind::ProjectActive
        | ActionKind::ProjectList => validate_empty_action_params(action),
        ActionKind::BlockList => action.params_as::<BlockListParams>().map(|_| ()),
        ActionKind::BlockInspect | ActionKind::BlockOutput => action.params_as::<BlockIdParams>().and_then(|params| {
            if params.block_id.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "block.inspect requires a non-empty block id",
                ));
            }
            Ok(())
        }),
        ActionKind::HistoryList => action.params_as::<HistoryListParams>().map(|_| ()),
        ActionKind::DriveList => action.params_as::<DriveListParams>().map(|_| ()),
        ActionKind::DriveInspect => action.params_as::<DriveInspectParams>().and_then(|params| {
            if params.id.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.inspect requires a non-empty Drive object id",
                ));
            }
            Ok(())
        }),
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

pub(super) fn target_window_id(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<warpui::WindowId, ControlError> {
    require_active_window_id(ctx.windows().active_window())
}

pub(crate) fn require_active_window_id(
    active_window: Option<warpui::WindowId>,
) -> Result<warpui::WindowId, ControlError> {
    require_active_window_id_for_action(active_window, ActionKind::TabCreate)
}

pub(crate) fn require_active_window_id_for_action(
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
