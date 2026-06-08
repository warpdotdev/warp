//! Layout mutation handlers for local-control actions.
#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
use ::local_control::protocol::TargetSelector;
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde::Serialize;
use warpui::{ModelContext, TypedActionView};

use crate::local_control::resolver::{target_window_id_for_target, validate_tab_create_target};
use crate::local_control::LocalControlBridge;
use crate::workspace::{Workspace, WorkspaceAction};
#[derive(Serialize)]
struct TabCreateResponse<'a> {
    action: &'static str,
    created: bool,
    instance_id: Option<&'a str>,
    window: TargetWindowResponse,
    tab: TabCountsResponse,
}

#[derive(Serialize)]
struct TargetWindowResponse {
    selector: &'static str,
    id: String,
}

#[derive(Serialize)]
struct TabCountsResponse {
    id: String,
    previous_count: usize,
    count: usize,
    active_index: usize,
}

pub(crate) fn create_terminal_tab(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_tab_create_target(target)?;
    let window_id = target_window_id_for_target(ctx, target, ActionKind::TabCreate)?;
    let workspace = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "tab.create requires a workspace in the target window",
            )
        })?;
    let (tab_id, previous_tab_count, tab_count, active_tab_index) =
        workspace.update(ctx, |workspace, ctx| {
            let previous_tab_count = workspace.tab_count();
            workspace.handle_action(
                &WorkspaceAction::AddTerminalTab {
                    hide_homepage: false,
                },
                ctx,
            );
            let tab_id = workspace
                .get_pane_group_view(workspace.active_tab_index())
                .map(|tab| tab.id().to_string())
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::Internal,
                        "tab.create did not produce an active tab identifier",
                    )
                })?;
            Ok((
                tab_id,
                previous_tab_count,
                workspace.tab_count(),
                workspace.active_tab_index(),
            ))
        })?;
    serde_json::to_value(TabCreateResponse {
        action: ActionKind::TabCreate.as_str(),
        created: true,
        instance_id: instance_id.as_ref().map(|id| id.0.as_str()),
        window: TargetWindowResponse {
            selector: "target",
            id: window_id.to_string(),
        },
        tab: TabCountsResponse {
            id: tab_id,
            previous_count: previous_tab_count,
            count: tab_count,
            active_index: active_tab_index,
        },
    })
    .map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control tab.create response",
            err.to_string(),
        )
    })
}
