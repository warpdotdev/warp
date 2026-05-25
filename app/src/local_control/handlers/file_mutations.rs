use std::fs;
use std::path::{Component, Path, PathBuf};

use ::local_control::protocol::{
    FileDeleteParams, FileMutationResult, FileTarget, FileWriteParams,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, RequestEnvelope};
use warpui::{ModelContext, SingletonEntity};

use crate::local_control::LocalControlBridge;
use crate::projects::ProjectManagementModel;

pub(crate) fn write_file(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = request.action.params_as::<FileWriteParams>()?;
    validate_file_mutation_target(ActionKind::FileWrite, &request.target, &params.path)?;
    let roots = file_mutation_roots(ctx)?;
    let path = resolve_file_mutation_path(ActionKind::FileWrite, &params.path, &roots, true)?;
    if !params.create && !path.exists() {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "file.write cannot resolve the requested file path",
        ));
    }
    if path.exists() && !path.is_file() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "file.write only supports writing files",
        ));
    }
    fs::write(&path, params.contents).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TargetStateConflict,
            "file.write failed to write the requested file",
            err.to_string(),
        )
    })?;
    to_file_data(FileMutationResult {
        path: path.display().to_string(),
        tab_id: None,
    })
}

pub(crate) fn delete_file(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = request.action.params_as::<FileDeleteParams>()?;
    validate_file_mutation_target(ActionKind::FileDelete, &request.target, &params.path)?;
    if params.recursive {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "file.delete does not support recursive directory deletion",
        ));
    }
    let roots = file_mutation_roots(ctx)?;
    let path = resolve_file_mutation_path(ActionKind::FileDelete, &params.path, &roots, false)?;
    if !path.is_file() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "file.delete only supports deleting files",
        ));
    }
    fs::remove_file(&path).map_err(|err| {
        ControlError::with_details(
            ErrorCode::TargetStateConflict,
            "file.delete failed to delete the requested file",
            err.to_string(),
        )
    })?;
    to_file_data(FileMutationResult {
        path: path.display().to_string(),
        tab_id: None,
    })
}

pub(crate) fn validate_file_mutation_target(
    action: ActionKind,
    target: &::local_control::protocol::TargetSelector,
    path: &str,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.drive.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept window, tab, pane, session, or drive selectors",
                action.as_str()
            ),
        ));
    }
    match target.file.as_ref() {
        None => Ok(()),
        Some(FileTarget::Path { path: target_path }) if target_path == path => Ok(()),
        Some(FileTarget::Path { .. }) => Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!(
                "{} file selector does not match the requested path",
                action.as_str()
            ),
        )),
        Some(FileTarget::Id { .. }) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} does not support file id selectors", action.as_str()),
        )),
    }
}

fn file_mutation_roots(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<PathBuf>, ControlError> {
    let mut roots = Vec::new();
    ProjectManagementModel::handle(ctx).read(ctx, |model, _ctx| {
        roots.extend(
            model
                .all_projects()
                .map(|project| PathBuf::from(&project.path)),
        );
    });
    let mut canonical_roots = Vec::new();
    for root in roots {
        if let Ok(canonical_root) = root.canonicalize() {
            if canonical_root.is_dir() && !canonical_roots.contains(&canonical_root) {
                canonical_roots.push(canonical_root);
            }
        }
    }
    if canonical_roots.is_empty() {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "file mutations require an active local project or known workspace path",
        ));
    }
    Ok(canonical_roots)
}

pub(crate) fn resolve_file_mutation_path(
    action: ActionKind,
    path: &str,
    allowed_roots: &[PathBuf],
    allow_missing_file: bool,
) -> Result<PathBuf, ControlError> {
    if path.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty path", action.as_str()),
        ));
    }
    let requested = Path::new(path);
    if !path_has_safe_components(requested) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} path must not contain parent traversal", action.as_str()),
        ));
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        let [root] = allowed_roots else {
            return Err(ControlError::new(
                ErrorCode::InvalidSelector,
                format!(
                    "{} requires an absolute path when multiple workspace roots are available",
                    action.as_str()
                ),
            ));
        };
        root.join(requested)
    };
    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|err| {
            ControlError::with_details(
                ErrorCode::StaleTarget,
                format!("{} cannot resolve the requested file path", action.as_str()),
                err.to_string(),
            )
        })?
    } else if allow_missing_file {
        let parent = candidate.parent().ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidSelector,
                format!(
                    "{} requires a path with a parent directory",
                    action.as_str()
                ),
            )
        })?;
        let file_name = candidate.file_name().ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidSelector,
                format!("{} requires a file path", action.as_str()),
            )
        })?;
        let canonical_parent = parent.canonicalize().map_err(|err| {
            ControlError::with_details(
                ErrorCode::StaleTarget,
                format!(
                    "{} cannot resolve the requested parent directory",
                    action.as_str()
                ),
                err.to_string(),
            )
        })?;
        canonical_parent.join(file_name)
    } else {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{} cannot resolve the requested file path", action.as_str()),
        ));
    };
    if !allowed_roots.iter().any(|root| resolved.starts_with(root)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} path is outside the active project or known workspace paths",
                action.as_str()
            ),
        ));
    }
    Ok(resolved)
}

fn path_has_safe_components(path: &Path) -> bool {
    path.components().all(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::Normal(_)
        )
    })
}

fn to_file_data<T: serde::Serialize>(data: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(data).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode local-control file response",
            err.to_string(),
        )
    })
}
