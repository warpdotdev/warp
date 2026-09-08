use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ::ai::index::full_source_code_embedding::SyncProgress;
use ::ai::index::full_source_code_embedding::manager::{
    CodebaseIndexFinishedStatus, CodebaseIndexStatus as LocalCodebaseIndexStatus,
};

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
    match (has_synced_version, has_pending, last_sync_result) {
        (true, false, Some(CodebaseIndexFinishedStatus::Completed)) => {
            CodebaseIndexStatusState::Ready
        }
        // Match local search behavior: any status with a synced root can still serve search
        // requests from that last good root while new incremental work catches up.
        (true, _, _) => CodebaseIndexStatusState::Stale,
        (false, true, _) => CodebaseIndexStatusState::Indexing,
        (false, false, Some(CodebaseIndexFinishedStatus::Failed(_))) => {
            CodebaseIndexStatusState::Failed
        }
        (false, false, Some(CodebaseIndexFinishedStatus::Completed)) => {
            CodebaseIndexStatusState::Ready
        }
        (false, false, None) => CodebaseIndexStatusState::Queued,
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
#[path = "codebase_index_status_tests.rs"]
mod tests;
