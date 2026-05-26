//! Target and parameter validation for the first local-control action slice.
use ::local_control::protocol::{ActionParams, PaneTarget, TabTarget, TargetSelector, WindowTarget};
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
    Ok(())
}

/// Validates action-specific params implemented by this branch stack layer.
///
/// This is intentionally narrow while `zach/warp-cli-core-foundation` is the
/// bottom branch of the stack: later branches add their own params and expand
/// this validation alongside the corresponding action handlers.
pub(crate) fn validate_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    match action.kind {
        ActionKind::TabCreate => validate_empty_action_params(action),
        ActionKind::InputRun => {
            let ActionParams::Text { text } = action.params_as::<ActionParams>()? else {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "input.run requires text parameters",
                ));
            };
            if text.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "input.run requires non-empty text",
                ));
            }
            Ok(())
        }
        ActionKind::DriveWorkflowRun => {
            let ActionParams::WorkflowRun(params) = action.params_as::<ActionParams>()? else {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.workflow.run requires workflow run parameters",
                ));
            };
            if params.id.0.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.workflow.run requires a non-empty workflow id",
                ));
            }
            for arg in &params.args {
                if excluded_submission_argument_name(&arg.name) {
                    return Err(ControlError::new(
                        ErrorCode::UnsupportedAction,
                        "drive.workflow.run does not accept accepted-command or agent-prompt submissions",
                    ));
                }
            }
            Ok(())
        }
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

fn excluded_submission_argument_name(name: &str) -> bool {
    matches!(
        name,
        "accepted_command"
            | "accepted-command"
            | "agent_prompt"
            | "agent-prompt"
            | "internal_dispatch"
            | "internal-dispatch"
    )
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
