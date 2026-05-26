use std::collections::BTreeMap;

use ::local_control::protocol::{
    FileListResult, FileSummary, ProjectActiveResult, ProjectListResult, ProjectSummary,
    TargetSelector,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde::Serialize;
use warpui::{ModelContext, SingletonEntity};

use crate::code::view::CodeView;
use crate::local_control::LocalControlBridge;
use crate::projects::ProjectManagementModel;
use crate::terminal::view::TerminalView;
use crate::workspace::ActiveSession;

pub(crate) fn file_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_instance_metadata_read_target(ActionKind::FileList, target)?;
    to_control_data(FileListResult {
        files: open_file_summaries(ctx),
    })
}

pub(crate) fn project_active(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_instance_metadata_read_target(ActionKind::ProjectActive, target)?;
    to_control_data(ProjectActiveResult {
        project: active_project_summary(ctx),
    })
}

pub(crate) fn project_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_instance_metadata_read_target(ActionKind::ProjectList, target)?;
    to_control_data(ProjectListResult {
        projects: project_summaries(ctx),
    })
}

pub(crate) fn validate_instance_metadata_read_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept target selectors; it reads app-state metadata already represented in Warp",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn open_file_summaries(ctx: &mut ModelContext<LocalControlBridge>) -> Vec<FileSummary> {
    let window_ids: Vec<_> = ctx.window_ids().collect();
    let mut files = Vec::new();
    for window_id in window_ids {
        let Some(code_views) = ctx.views_of_type::<CodeView>(window_id) else {
            continue;
        };
        for code_view in code_views {
            code_view.read(ctx, |code_view, _ctx| {
                for index in 0..code_view.tab_count() {
                    if let Some(location) = code_view.tab_at(index).and_then(|tab| tab.location()) {
                        files.push(FileSummary {
                            path: location.display_path(),
                            tab_id: None,
                        });
                    }
                }
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path && left.tab_id == right.tab_id);
    files
}

fn active_project_path(ctx: &mut ModelContext<LocalControlBridge>) -> Option<String> {
    let window_id = ctx.windows().active_window()?;
    let repo_path = ActiveSession::as_ref(ctx)
        .terminal_view_id(window_id)
        .and_then(|terminal_view_id| ctx.view_with_id::<TerminalView>(window_id, terminal_view_id))
        .and_then(|terminal| {
            terminal
                .as_ref(ctx)
                .current_repo_path()
                .map(|path| path.display_path())
        });
    repo_path.or_else(|| {
        ActiveSession::as_ref(ctx)
            .working_directory(window_id)
            .map(|path| path.display_path())
    })
}

fn active_project_summary(ctx: &mut ModelContext<LocalControlBridge>) -> Option<ProjectSummary> {
    active_project_path(ctx).map(|path| ProjectSummary {
        path,
        is_active: true,
        last_opened_at: None,
    })
}

fn project_summaries(ctx: &mut ModelContext<LocalControlBridge>) -> Vec<ProjectSummary> {
    let active_path = active_project_path(ctx);
    let mut projects = BTreeMap::new();
    ProjectManagementModel::handle(ctx).read(ctx, |model, _ctx| {
        for project in model.all_projects() {
            projects.insert(
                project.path.clone(),
                ProjectSummary {
                    path: project.path.clone(),
                    is_active: active_path.as_ref() == Some(&project.path),
                    last_opened_at: project
                        .last_opened_ts
                        .map(|timestamp| timestamp.to_string()),
                },
            );
        }
    });
    if let Some(path) = active_path {
        projects.entry(path.clone()).or_insert(ProjectSummary {
            path,
            is_active: true,
            last_opened_at: None,
        });
    }
    projects.into_values().collect()
}

fn to_control_data<T: Serialize>(value: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(value).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control response",
            err.to_string(),
        )
    })
}
