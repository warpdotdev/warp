use ::ai::index::full_source_code_embedding::manager::CodebaseIndexingError;

use super::*;

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
fn syncing_codebase_index_with_synced_version_maps_to_stale() {
    assert_eq!(
        codebase_index_status_state_from_parts(false, true, None),
        CodebaseIndexStatusState::Stale
    );
}

#[test]
fn failed_codebase_index_with_synced_version_maps_to_stale() {
    let result = CodebaseIndexFinishedStatus::Failed(CodebaseIndexingError::BuildTreeError);

    assert_eq!(
        codebase_index_status_state_from_parts(false, true, Some(&result)),
        CodebaseIndexStatusState::Stale
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
