//! Layout mutation handlers for local-control actions.
use ::local_control::protocol::TargetSelector;
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use warpui::{ModelContext, TypedActionView};

use crate::local_control::resolver::{target_window_id, validate_tab_create_target};
use crate::local_control::LocalControlBridge;
use crate::workspace::{Workspace, WorkspaceAction};

pub(crate) fn create_terminal_tab(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
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
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
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
