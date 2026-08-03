use remote_server::codebase_index_proto::{RemoteCodebaseIndexState, RemoteCodebaseIndexStatus};

use super::remote_codebase_index_limit_reached;

fn remote_status_with_failure(failure_message: Option<&str>) -> RemoteCodebaseIndexStatus {
    RemoteCodebaseIndexStatus {
        repo_path: "/workspaces/repo".to_string(),
        state: RemoteCodebaseIndexState::Unavailable,
        last_updated_epoch_millis: Some(1),
        progress_completed: None,
        progress_total: None,
        failure_message: failure_message.map(ToOwned::to_owned),
        root_hash: None,
    }
}

#[test]
fn remote_index_limit_failure_is_detected_from_status_message() {
    let status = remote_status_with_failure(Some(
        "Cannot index remote codebase because the maximum number of codebase indexes has been reached.",
    ));

    assert!(remote_codebase_index_limit_reached(&status));
}

#[test]
fn other_unavailable_failures_are_not_index_limit_failures() {
    let status = remote_status_with_failure(Some(
        "Cannot index remote codebase because indexing did not start.",
    ));

    assert!(!remote_codebase_index_limit_reached(&status));
}
