use super::*;

#[test]
fn local_wake_task_state_ready_allows_success_and_stale_in_progress() {
    assert!(is_local_wake_task_state_ready(
        AmbientAgentTaskState::Succeeded
    ));
    assert!(is_local_wake_task_state_ready(
        AmbientAgentTaskState::InProgress
    ));

    for state in [
        AmbientAgentTaskState::Queued,
        AmbientAgentTaskState::Pending,
        AmbientAgentTaskState::Claimed,
        AmbientAgentTaskState::Failed,
        AmbientAgentTaskState::Error,
        AmbientAgentTaskState::Blocked,
        AmbientAgentTaskState::Cancelled,
        AmbientAgentTaskState::Unknown,
    ] {
        assert!(!is_local_wake_task_state_ready(state));
    }
}
