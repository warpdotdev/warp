//! Layout mutation handlers for local-control actions.
#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
use ::local_control::protocol::{TabCreateParams, TabType, TargetSelector};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde::Serialize;
#[cfg(feature = "local_tty")]
use warpui::SingletonEntity;
use warpui::{ModelContext, TypedActionView};

use crate::local_control::resolver::{
    decode_params, target_window_id_for_target, validate_tab_create_target, workspace_for_window,
};
use crate::local_control::LocalControlBridge;
use crate::server::telemetry::AddTabWithShellSource;
use crate::terminal::available_shells::AvailableShell;
#[cfg(feature = "local_tty")]
use crate::terminal::available_shells::AvailableShells;
use crate::workspace::WorkspaceAction;
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

pub(crate) fn create_tab(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_tab_create_target(target)?;
    let window_id = target_window_id_for_target(ctx, target, ActionKind::TabCreate)?;
    let workspace = workspace_for_window(window_id, ActionKind::TabCreate, ctx)?;
    let action = tab_create_action(params, ctx)?;
    let (tab_id, previous_tab_count, tab_count, active_tab_index) =
        workspace.update(ctx, |workspace, ctx| {
            let previous_tab_count = workspace.tab_count();
            workspace.handle_action(&action, ctx);
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

fn tab_create_action(
    params: &serde_json::Value,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<WorkspaceAction, ControlError> {
    let params = decode_params::<TabCreateParams>(params)?;
    if let Some(shell_name) = params.shell.as_deref() {
        if matches!(params.tab_type, Some(TabType::Agent | TabType::CloudAgent)) {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "tab.create cannot combine an agent tab type with a shell",
            ));
        }
        return Ok(WorkspaceAction::AddTabWithShell {
            shell: resolve_shell(shell_name, ctx)?,
            source: AddTabWithShellSource::CommandPalette,
        });
    }
    match params.tab_type {
        None | Some(TabType::Terminal) => Ok(WorkspaceAction::AddTerminalTab {
            hide_homepage: false,
        }),
        Some(TabType::Agent) => Ok(WorkspaceAction::AddAgentTab),
        Some(TabType::Default) => Ok(WorkspaceAction::AddDefaultTab),
        Some(TabType::CloudAgent) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "tab.create does not support cloud-agent tabs",
        )),
    }
}

#[cfg_attr(not(feature = "local_tty"), allow(unused_variables))]
pub(super) fn resolve_shell(
    name: &str,
    ctx: &ModelContext<LocalControlBridge>,
) -> Result<AvailableShell, ControlError> {
    #[cfg(feature = "local_tty")]
    {
        AvailableShells::as_ref(ctx)
            .find_by_command_name(name)
            .or_else(|| AvailableShell::try_from(name).ok())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    format!("cannot resolve requested shell {name:?}"),
                )
            })
    }
    #[cfg(not(feature = "local_tty"))]
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        format!("shell selection is unavailable for requested shell {name:?}"),
    ))
}
