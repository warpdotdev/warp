use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ::ai::index::full_source_code_embedding::manager::{
    CodebaseIndexFinishedStatus, CodebaseIndexStatus as LocalCodebaseIndexStatus,
};
use ::ai::index::full_source_code_embedding::SyncProgress;

use super::proto::{CodebaseIndexStatus, CodebaseIndexStatusState};

fn current_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(super) fn queued_codebase_index_status(repo_path: String) -> CodebaseIndexStatus {
    base_codebase_index_status(repo_path, CodebaseIndexStatusState::Queued)
}

pub(super) fn not_enabled_codebase_index_status(repo_path: String) -> CodebaseIndexStatus {
    base_codebase_index_status(repo_path, CodebaseIndexStatusState::NotEnabled)
}

pub(super) fn disabled_codebase_index_status(repo_path: String) -> CodebaseIndexStatus {
    base_codebase_index_status(repo_path, CodebaseIndexStatusState::Disabled)
}
pub(super) fn unavailable_codebase_index_status(
    repo_path: String,
    failure_message: String,
) -> CodebaseIndexStatus {
    CodebaseIndexStatus {
        failure_message: Some(failure_message),
        ..base_codebase_index_status(repo_path, CodebaseIndexStatusState::Unavailable)
    }
}

fn base_codebase_index_status(
    repo_path: String,
    state: CodebaseIndexStatusState,
) -> CodebaseIndexStatus {
    CodebaseIndexStatus {
        repo_path,
        state: state.into(),
        last_updated_epoch_millis: Some(current_epoch_millis()),
        progress_completed: None,
        progress_total: None,
        failure_message: None,
        root_hash: None,
    }
}

pub(super) fn codebase_index_status_to_proto(
    repo_path: &Path,
    status: &LocalCodebaseIndexStatus,
) -> CodebaseIndexStatus {
    let state = codebase_index_status_state(status);
    let (progress_completed, progress_total) = progress_from_codebase_index_status(status);

    CodebaseIndexStatus {
        repo_path: repo_path.to_string_lossy().to_string(),
        state: state.into(),
        last_updated_epoch_millis: Some(current_epoch_millis()),
        progress_completed,
        progress_total,
        failure_message: failure_message_from_codebase_index_status(status),
        root_hash: status.root_hash().map(|hash| hash.to_string()),
    }
}

fn codebase_index_status_state(status: &LocalCodebaseIndexStatus) -> CodebaseIndexStatusState {
    codebase_index_status_state_from_parts(
        status.has_pending(),
        status.has_synced_version(),
        status.last_sync_result(),
    )
}

fn codebase_index_status_state_from_parts(
    has_pending: bool,
    has_synced_version: bool,
    last_sync_result: Option<&CodebaseIndexFinishedStatus>,
) -> CodebaseIndexStatusState {
    match (has_pending, has_synced_version, last_sync_result) {
        (true, true, _) => CodebaseIndexStatusState::Stale,
        (true, false, _) => CodebaseIndexStatusState::Indexing,
        (false, _, Some(CodebaseIndexFinishedStatus::Completed)) => CodebaseIndexStatusState::Ready,
        (false, _, Some(CodebaseIndexFinishedStatus::Failed(_))) => {
            CodebaseIndexStatusState::Failed
        }
        (false, _, None) => CodebaseIndexStatusState::Queued,
    }
}

fn progress_from_sync_progress(sync_progress: Option<&SyncProgress>) -> (Option<u64>, Option<u64>) {
    match sync_progress {
        Some(SyncProgress::Discovering { total_nodes }) => (Some(0), Some(*total_nodes as u64)),
        Some(SyncProgress::Syncing {
            completed_nodes,
            total_nodes,
        }) => (Some(*completed_nodes as u64), Some(*total_nodes as u64)),
        None => (None, None),
    }
}

fn progress_from_codebase_index_status(
    status: &LocalCodebaseIndexStatus,
) -> (Option<u64>, Option<u64>) {
    progress_from_sync_progress(status.sync_progress())
}

fn failure_message_from_last_sync_result(
    last_sync_result: Option<&CodebaseIndexFinishedStatus>,
) -> Option<String> {
    match last_sync_result {
        Some(CodebaseIndexFinishedStatus::Failed(error)) => Some(error.to_string()),
        Some(CodebaseIndexFinishedStatus::Completed) | None => None,
    }
}

fn failure_message_from_codebase_index_status(status: &LocalCodebaseIndexStatus) -> Option<String> {
    failure_message_from_last_sync_result(status.last_sync_result())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::ai::index::full_source_code_embedding::manager::CodebaseIndexingError;

    #[test]
    fn pending_codebase_index_without_synced_version_maps_to_indexing() {
        assert_eq!(
            codebase_index_status_state_from_parts(true, false, None),
            CodebaseIndexStatusState::Indexing
        );
    }

    #[test]
    fn pending_codebase_index_with_synced_version_maps_to_stale() {
        assert_eq!(
            codebase_index_status_state_from_parts(true, true, None),
            CodebaseIndexStatusState::Stale
        );
    }

    #[test]
    fn completed_codebase_index_maps_to_ready() {
        let result = CodebaseIndexFinishedStatus::Completed;

        assert_eq!(
            codebase_index_status_state_from_parts(false, true, Some(&result)),
            CodebaseIndexStatusState::Ready
        );
    }

    #[test]
    fn failed_codebase_index_maps_to_failed_and_includes_message() {
        let result = CodebaseIndexFinishedStatus::Failed(CodebaseIndexingError::BuildTreeError);

        assert_eq!(
            codebase_index_status_state_from_parts(false, false, Some(&result)),
            CodebaseIndexStatusState::Failed
        );
        assert_eq!(
            failure_message_from_last_sync_result(Some(&result)).as_deref(),
            Some("Build tree error")
        );
    }

    #[test]
    fn sync_progress_maps_to_remote_progress_fields() {
        assert_eq!(
            progress_from_sync_progress(Some(&SyncProgress::Discovering { total_nodes: 5 })),
            (Some(0), Some(5))
        );
        assert_eq!(
            progress_from_sync_progress(Some(&SyncProgress::Syncing {
                completed_nodes: 3,
                total_nodes: 8,
            })),
            (Some(3), Some(8))
        );
        assert_eq!(progress_from_sync_progress(None), (None, None));
    }
}
