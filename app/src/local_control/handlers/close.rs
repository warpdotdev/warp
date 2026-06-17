//! Close handlers for local-control window, tab, and pane actions.
use ::local_control::protocol::{TabCloseMode, TabCloseParams, TabTarget};
use ::local_control::{Action, ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope};
use warpui::platform::TerminationMode;
use warpui::ModelContext;

use crate::local_control::handlers::ack;
use crate::local_control::resolver::{
    reject_target_families, tab_index_from_target, target_pane_group, target_pane_id,
    target_window_id_for_target, target_workspace,
};
use crate::local_control::LocalControlBridge;
use crate::workspace::view::OpenDialogSource;

fn tab_close_mode(action: &Action) -> Result<TabCloseMode, ControlError> {
    Ok(action.params_as::<TabCloseParams>()?.mode)
}

fn validate_empty_params(action: &Action) -> Result<(), ControlError> {
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

pub(crate) fn window_close(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_empty_params(&request.action)?;
    reject_target_families(
        ActionKind::WindowClose,
        request.target.tab.is_some()
            || request.target.pane.is_some()
            || request.target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, &request.target, ActionKind::WindowClose)?;
    ctx.windows()
        .close_window(window_id, TerminationMode::Cancellable);
    Ok(ack(instance_id, ActionKind::WindowClose))
}

pub(crate) fn tab_close(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabClose,
        request.target.pane.is_some() || request.target.session.is_some(),
        "pane or session selectors",
    )?;
    let mode = tab_close_mode(&request.action)?;
    let workspace = target_workspace(ActionKind::TabClose, &request.target, ctx)?;
    let closed = workspace.update(ctx, |workspace, ctx| {
        let selected_index = tab_index_from_target(&request.target, workspace, ctx)?;
        let tab_count = workspace.tab_count();
        let tab_indices: Vec<usize> = match mode {
            TabCloseMode::Target => vec![selected_index],
            TabCloseMode::Active => {
                if !matches!(request.target.tab.as_ref(), None | Some(TabTarget::Active)) {
                    return Err(ControlError::new(
                        ErrorCode::InvalidSelector,
                        "tab.close active does not accept a concrete tab selector",
                    ));
                }
                vec![workspace.active_tab_index()]
            }
            TabCloseMode::Others => (0..tab_count)
                .filter(|index| *index != selected_index)
                .collect(),
            TabCloseMode::RightOf => ((selected_index + 1)..tab_count).collect(),
        };
        if tab_indices.is_empty() {
            return Ok(true);
        }
        let closed = workspace.close_tabs(
            tab_indices.into_iter(),
            OpenDialogSource::CloseTab {
                tab_index: selected_index,
            },
            false,
            true,
            ctx,
        );
        Ok(closed)
    })?;
    if closed {
        return Ok(ack(instance_id, ActionKind::TabClose));
    }
    Err(ControlError::new(
        ErrorCode::TargetStateConflict,
        "tab close was cancelled by an existing app warning",
    ))
}

pub(crate) fn pane_close(
    instance_id: &Option<InstanceId>,
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_empty_params(&request.action)?;
    reject_target_families(
        ActionKind::PaneClose,
        request.target.session.is_some(),
        "session selectors",
    )?;
    let pane_group = target_pane_group(ActionKind::PaneClose, &request.target, ctx)?;
    let pane_id = target_pane_id(ActionKind::PaneClose, &request.target, &pane_group, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| pane_group.close_pane(pane_id, ctx));
    Ok(ack(instance_id, ActionKind::PaneClose))
}
