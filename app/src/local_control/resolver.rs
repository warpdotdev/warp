//! Target and parameter validation for the first local-control action slice.
use crate::local_control::handlers::metadata::action_metadata_for_name;
use ::local_control::protocol::{
    ActionGetParams, ActionParams, BlockListParams, BlockOutputParams, FileOpenParams,
    HistoryListParams, InputInsertParams, InputModeSetParams, InputReplaceParams,
    PaneMaximizeParams, PaneNavigateParams, PaneResizeParams, PaneSplitParams, PaneTarget,
    SessionTarget, SettingGetParams, SettingSetParams, SettingToggleParams, TabActivateParams,
    TabCloseParams, TabMoveParams, TabTarget, TargetSelector, WindowCloseParams,
    WindowCreateParams, WindowTarget,
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
            let params = action.params_as::<ActionGetParams>()?;
            action_metadata_for_name(&params.action)?;
            Ok(())
        }
        ActionKind::SettingGet => {
            action.params_as::<SettingGetParams>()?;
            Ok(())
        }
        ActionKind::WindowCreate => action.params_as::<WindowCreateParams>().map(|_| ()),
        ActionKind::WindowClose => action.params_as::<WindowCloseParams>().map(|_| ()),
        ActionKind::TabActivate => action.params_as::<TabActivateParams>().map(|_| ()),
        ActionKind::TabMove => action.params_as::<TabMoveParams>().map(|_| ()),
        ActionKind::TabClose => action.params_as::<TabCloseParams>().map(|_| ()),
        ActionKind::PaneSplit => action.params_as::<PaneSplitParams>().map(|_| ()),
        ActionKind::PaneNavigate => action.params_as::<PaneNavigateParams>().map(|_| ()),
        ActionKind::PaneMaximize => action.params_as::<PaneMaximizeParams>().map(|_| ()),
        ActionKind::PaneResize => action.params_as::<PaneResizeParams>().and_then(|params| {
            if params.amount == Some(0) {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "pane.resize amount must be greater than zero",
                ));
            }
            Ok(())
        }),
        ActionKind::BlockList => action.params_as::<BlockListParams>().map(|_| ()),
        ActionKind::BlockOutput => action.params_as::<BlockOutputParams>().map(|_| ()),
        ActionKind::HistoryList => action.params_as::<HistoryListParams>().map(|_| ()),
        ActionKind::InputInsert => action.params_as::<InputInsertParams>().map(|_| ()),
        ActionKind::InputReplace => action.params_as::<InputReplaceParams>().map(|_| ()),
        ActionKind::InputModeSet => action.params_as::<InputModeSetParams>().map(|_| ()),
        ActionKind::SettingSet => action.params_as::<SettingSetParams>().map(|_| ()),
        ActionKind::SettingToggle => action.params_as::<SettingToggleParams>().map(|_| ()),
        ActionKind::FileOpen => action.params_as::<FileOpenParams>().and_then(|params| {
            if params.path.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "file.open requires a non-empty path",
                ));
            }
            if params.line.is_none() && params.column.is_some() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "file.open cannot accept a column without a line",
                ));
            }
            Ok(())
        }),
        ActionKind::ProjectOpen => {
            action
                .params_as::<ActionParams>()
                .and_then(|params| match params {
                    ActionParams::Path { path } if !path.trim().is_empty() => Ok(()),
                    ActionParams::Path { .. } => Err(ControlError::new(
                        ErrorCode::InvalidParams,
                        "project.open requires a non-empty path",
                    )),
                    _ => Err(ControlError::new(
                        ErrorCode::InvalidParams,
                        "project.open requires a path payload",
                    )),
                })
        }
        ActionKind::InstanceList
        | ActionKind::InstanceInspect
        | ActionKind::AppPing
        | ActionKind::AppVersion
        | ActionKind::AppActive
        | ActionKind::AppFocus
        | ActionKind::ActionList
        | ActionKind::CapabilityList
        | ActionKind::WindowList
        | ActionKind::WindowInspect
        | ActionKind::WindowFocus
        | ActionKind::TabList
        | ActionKind::TabInspect
        | ActionKind::TabCreate
        | ActionKind::PaneList
        | ActionKind::PaneInspect
        | ActionKind::PaneFocus
        | ActionKind::PaneClose
        | ActionKind::SessionList
        | ActionKind::SessionInspect
        | ActionKind::InputGet
        | ActionKind::InputClear
        | ActionKind::SessionPrevious
        | ActionKind::SessionNext
        | ActionKind::SessionReopenClosed
        | ActionKind::ThemeList
        | ActionKind::AppearanceGet
        | ActionKind::SettingList => validate_empty_action_params(action),
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
    active_window.ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            "tab.create requires an active Warp window",
        )
    })
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
