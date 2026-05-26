use ::local_control::protocol::{FileListResult, FileSummary, TargetSelector};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde::Serialize;
use warpui::ModelContext;

use crate::code::view::CodeView;
use crate::local_control::LocalControlBridge;

pub(crate) fn file_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_instance_metadata_read_target(ActionKind::FileList, target)?;
    to_control_data(FileListResult {
        files: open_file_summaries(ctx),
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

fn to_control_data<T: Serialize>(value: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(value).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control response",
            err.to_string(),
        )
    })
}
