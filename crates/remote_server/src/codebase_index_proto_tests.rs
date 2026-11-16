use super::*;
fn status(state: RemoteCodebaseIndexState) -> RemoteCodebaseIndexStatus {
    RemoteCodebaseIndexStatus {
        repo_path: "/repo".to_string(),
        state,
        last_updated_epoch_millis: Some(42),
        progress_completed: None,
        progress_total: None,
        failure_message: None,
        root_hash: None,
    }
}

#[test]
fn all_status_states_round_trip_through_proto() {
    for state in [
        RemoteCodebaseIndexState::NotEnabled,
        RemoteCodebaseIndexState::Unavailable,
        RemoteCodebaseIndexState::Disabled,
        RemoteCodebaseIndexState::Queued,
        RemoteCodebaseIndexState::Indexing,
        RemoteCodebaseIndexState::Ready,
        RemoteCodebaseIndexState::Stale,
        RemoteCodebaseIndexState::Failed,
    ] {
        let status = status(state);

        let proto = proto::CodebaseIndexStatus::from(&status);
        assert_eq!(proto_to_codebase_index_status(&proto), Some(status));
    }
}

#[test]
fn ready_status_round_trips_retrieval_metadata() {
    let status = RemoteCodebaseIndexStatus {
        root_hash: Some("root-hash".to_string()),
        ..status(RemoteCodebaseIndexState::Ready)
    };

    let proto = proto::CodebaseIndexStatus::from(&status);
    assert_eq!(proto.root_hash.as_deref(), Some("root-hash"));
    assert_eq!(proto_to_codebase_index_status(&proto), Some(status));
}

#[test]
fn indexing_status_round_trips_progress() {
    let status = RemoteCodebaseIndexStatus {
        progress_completed: Some(7),
        progress_total: Some(11),
        ..status(RemoteCodebaseIndexState::Indexing)
    };

    let proto = proto::CodebaseIndexStatus::from(&status);
    assert_eq!(proto.progress_completed, Some(7));
    assert_eq!(proto.progress_total, Some(11));
    assert_eq!(proto_to_codebase_index_status(&proto), Some(status));
}

#[test]
fn failed_status_round_trips_failure_message() {
    let status = RemoteCodebaseIndexStatus {
        failure_message: Some("failed to sync".to_string()),
        ..status(RemoteCodebaseIndexState::Failed)
    };

    let proto = proto::CodebaseIndexStatus::from(&status);
    assert_eq!(proto.failure_message.as_deref(), Some("failed to sync"));
    assert_eq!(proto_to_codebase_index_status(&proto), Some(status));
}

#[test]
fn unspecified_status_state_is_ignored() {
    let status = proto::CodebaseIndexStatus {
        repo_path: "/repo".to_string(),
        state: proto::CodebaseIndexStatusState::Unspecified as i32,
        last_updated_epoch_millis: None,
        progress_completed: None,
        progress_total: None,
        failure_message: None,
        root_hash: None,
    };

    assert_eq!(proto_to_codebase_index_status(&status), None);
}
