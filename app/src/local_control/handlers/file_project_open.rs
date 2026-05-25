//! App-state open handlers for the `file.open` and `project.open` actions.
//!
//! These handlers operate Warp's visible UI/app state (opening a file in an
//! editor tab or focusing/opening a repository) and do not expose file content
//! reads or filesystem-content mutations through the public `warpctrl`
//! catalog. File content CRUD remains intentionally out of scope per
//! `specs/warp-control-cli/PRODUCT.md`.
use std::path::PathBuf;

use ::local_control::protocol::{FileOpenParams, TargetSelector, WindowTarget};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde_json::json;
use warp_util::path::LineAndColumnArg;
use warpui::{ModelContext, TypedActionView, ViewHandle, WindowId};

use crate::local_control::resolver::require_active_window_id_for_action;
use crate::local_control::LocalControlBridge;
use crate::workspace::{Workspace, WorkspaceAction};

pub(crate) fn file_open(
    target: &TargetSelector,
    params: FileOpenParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let path = validate_path(&params.path, ActionKind::FileOpen)?;
    let line_and_column = line_and_column(params.line, params.column, ActionKind::FileOpen)?;
    let window_id = select_window_for_app_state_target(ActionKind::FileOpen, target, ctx)?;
    let workspace = workspace_for_window(ActionKind::FileOpen, window_id, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.handle_action(
            &WorkspaceAction::OpenFileInNewTab {
                full_path: path.clone(),
                line_and_column,
            },
            ctx,
        );
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::FileOpen.as_str(),
        "opened": true,
        "path": params.path,
        "new_tab": params.new_tab,
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn project_open(
    target: &TargetSelector,
    path: String,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_path(&path, ActionKind::ProjectOpen)?;
    let window_id = select_window_for_app_state_target(ActionKind::ProjectOpen, target, ctx)?;
    let workspace = workspace_for_window(ActionKind::ProjectOpen, window_id, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.handle_action(
            &WorkspaceAction::OpenRepository {
                path: Some(path.clone()),
            },
            ctx,
        );
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::ProjectOpen.as_str(),
        "opened": true,
        "path": path,
        "window_id": window_id.to_string(),
    }))
}

fn validate_path(path: &str, action: ActionKind) -> Result<PathBuf, ControlError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty path", action.as_str()),
        ));
    }
    Ok(PathBuf::from(trimmed))
}

fn line_and_column(
    line: Option<u32>,
    column: Option<u32>,
    action: ActionKind,
) -> Result<Option<LineAndColumnArg>, ControlError> {
    let Some(line_num) = line else {
        if column.is_some() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                format!("{} cannot accept a column without a line", action.as_str()),
            ));
        }
        return Ok(None);
    };
    let line_num = usize::try_from(line_num).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            format!("{} line number is out of range", action.as_str()),
            err.to_string(),
        )
    })?;
    let column_num = column
        .map(|column| {
            usize::try_from(column).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidParams,
                    format!("{} column number is out of range", action.as_str()),
                    err.to_string(),
                )
            })
        })
        .transpose()?;
    Ok(Some(LineAndColumnArg {
        line_num,
        column_num,
    }))
}

fn select_window_for_app_state_target(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<WindowId, ControlError> {
    if target.tab.is_some() || target.pane.is_some() || target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept tab, pane, or session selectors",
                action.as_str()
            ),
        ));
    }
    match target.window.as_ref() {
        None | Some(WindowTarget::Active) => {
            require_active_window_id_for_action(ctx.windows().active_window(), action)
        }
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
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
