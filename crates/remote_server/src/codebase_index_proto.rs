//! Conversion between remote codebase indexing domain types and proto-generated types.

use serde::Serialize;

use crate::proto;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCodebaseIndexStatus {
    pub repo_path: String,
    pub state: RemoteCodebaseIndexState,
    pub last_updated_epoch_millis: Option<u64>,
    pub progress_completed: Option<u64>,
    pub progress_total: Option<u64>,
    pub failure_message: Option<String>,
    pub root_hash: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCodebaseIndexState {
    NotEnabled,
    Unavailable,
    Disabled,
    Queued,
    Indexing,
    Ready,
    Stale,
    Failed,
}

// ── Rust → Proto ────────────────────────────────────────────

impl From<&RemoteCodebaseIndexStatus> for proto::CodebaseIndexStatus {
    fn from(status: &RemoteCodebaseIndexStatus) -> Self {
        Self {
            repo_path: status.repo_path.clone(),
            state: proto_state(status.state) as i32,
            last_updated_epoch_millis: status.last_updated_epoch_millis,
            progress_completed: status.progress_completed,
            progress_total: status.progress_total,
            failure_message: status.failure_message.clone(),
            root_hash: status.root_hash.clone(),
        }
    }
}

pub fn statuses_to_snapshot_proto<'a>(
    statuses: impl IntoIterator<Item = &'a RemoteCodebaseIndexStatus>,
) -> proto::CodebaseIndexStatusesSnapshot {
    proto::CodebaseIndexStatusesSnapshot {
        statuses: statuses
            .into_iter()
            .map(proto::CodebaseIndexStatus::from)
            .collect(),
    }
}

fn proto_state(state: RemoteCodebaseIndexState) -> proto::CodebaseIndexStatusState {
    match state {
        RemoteCodebaseIndexState::NotEnabled => proto::CodebaseIndexStatusState::NotEnabled,
        RemoteCodebaseIndexState::Unavailable => proto::CodebaseIndexStatusState::Unavailable,
        RemoteCodebaseIndexState::Disabled => proto::CodebaseIndexStatusState::Disabled,
        RemoteCodebaseIndexState::Queued => proto::CodebaseIndexStatusState::Queued,
        RemoteCodebaseIndexState::Indexing => proto::CodebaseIndexStatusState::Indexing,
        RemoteCodebaseIndexState::Ready => proto::CodebaseIndexStatusState::Ready,
        RemoteCodebaseIndexState::Stale => proto::CodebaseIndexStatusState::Stale,
        RemoteCodebaseIndexState::Failed => proto::CodebaseIndexStatusState::Failed,
    }
}

// ── Proto → Rust ──────────────────────────────────────────────────

pub fn proto_to_codebase_index_status(
    status: &proto::CodebaseIndexStatus,
) -> Option<RemoteCodebaseIndexStatus> {
    Some(RemoteCodebaseIndexStatus {
        repo_path: status.repo_path.clone(),
        state: proto_to_state(proto::CodebaseIndexStatusState::try_from(status.state).ok()?)?,
        last_updated_epoch_millis: status.last_updated_epoch_millis,
        progress_completed: status.progress_completed,
        progress_total: status.progress_total,
        failure_message: status.failure_message.clone(),
        root_hash: status.root_hash.clone(),
    })
}

pub fn proto_to_codebase_index_statuses_snapshot(
    snapshot: &proto::CodebaseIndexStatusesSnapshot,
) -> Vec<RemoteCodebaseIndexStatus> {
    snapshot
        .statuses
        .iter()
        .filter_map(proto_to_codebase_index_status)
        .collect()
}

pub fn proto_to_codebase_index_status_updated(
    update: &proto::CodebaseIndexStatusUpdated,
) -> Option<RemoteCodebaseIndexStatus> {
    proto_to_codebase_index_status(update.status.as_ref()?)
}

fn proto_to_state(state: proto::CodebaseIndexStatusState) -> Option<RemoteCodebaseIndexState> {
    match state {
        proto::CodebaseIndexStatusState::NotEnabled => Some(RemoteCodebaseIndexState::NotEnabled),
        proto::CodebaseIndexStatusState::Unavailable => Some(RemoteCodebaseIndexState::Unavailable),
        proto::CodebaseIndexStatusState::Disabled => Some(RemoteCodebaseIndexState::Disabled),
        proto::CodebaseIndexStatusState::Queued => Some(RemoteCodebaseIndexState::Queued),
        proto::CodebaseIndexStatusState::Indexing => Some(RemoteCodebaseIndexState::Indexing),
        proto::CodebaseIndexStatusState::Ready => Some(RemoteCodebaseIndexState::Ready),
        proto::CodebaseIndexStatusState::Stale => Some(RemoteCodebaseIndexState::Stale),
        proto::CodebaseIndexStatusState::Failed => Some(RemoteCodebaseIndexState::Failed),
        proto::CodebaseIndexStatusState::Unspecified => None,
    }
}

#[cfg(test)]
#[path = "codebase_index_proto_tests.rs"]
mod tests;
