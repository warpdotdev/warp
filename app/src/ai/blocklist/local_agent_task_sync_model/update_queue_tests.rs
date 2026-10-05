use warp_graphql::ai::AgentTaskState;

use super::{LocalTaskUpdate, LocalTaskUpdateQueue};
use crate::ai::ambient_agents::AmbientAgentTaskId;

fn task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440a00".parse().unwrap()
}

fn progress_update(force_task_state: bool) -> LocalTaskUpdate {
    LocalTaskUpdate {
        task_state: Some(AgentTaskState::InProgress),
        force_task_state,
        ..Default::default()
    }
}

#[test]
fn forced_progress_bypasses_cache_but_normal_progress_remains_deduplicated() {
    let mut queue = LocalTaskUpdateQueue::default();
    assert!(queue.enqueue(task_id(), progress_update(false)).is_some());
    assert!(queue.record_result(task_id(), true).is_none());
    assert!(queue.enqueue(task_id(), progress_update(false)).is_none());

    let update = queue.enqueue(task_id(), progress_update(true)).unwrap();
    assert_eq!(update.task_state, Some(AgentTaskState::InProgress));
    assert!(queue.record_result(task_id(), true).is_none());
    assert!(queue.enqueue(task_id(), progress_update(false)).is_none());
}

#[test]
fn every_forced_progress_survives_in_flight_acknowledgement_and_coalescing() {
    let mut queue = LocalTaskUpdateQueue::default();
    assert!(queue.enqueue(task_id(), progress_update(false)).is_some());
    assert!(queue.enqueue(task_id(), progress_update(false)).is_none());
    assert!(queue.enqueue(task_id(), progress_update(true)).is_none());
    assert!(queue.enqueue(task_id(), progress_update(true)).is_none());
    assert!(queue.enqueue(task_id(), progress_update(false)).is_none());

    for _ in 0..2 {
        let update = queue.record_result(task_id(), true).unwrap();
        assert_eq!(update.task_state, Some(AgentTaskState::InProgress));
        assert!(update.force_task_state);
    }
    assert!(queue.record_result(task_id(), true).is_none());
    assert!(queue.is_idle(&task_id()));
}

#[test]
fn forced_progress_preserves_terminal_transition_order() {
    let mut queue = LocalTaskUpdateQueue::default();
    assert!(queue.enqueue(task_id(), progress_update(false)).is_some());
    assert!(
        queue
            .enqueue(
                task_id(),
                LocalTaskUpdate {
                    task_state: Some(AgentTaskState::Succeeded),
                    ..Default::default()
                },
            )
            .is_none()
    );
    assert!(queue.enqueue(task_id(), progress_update(true)).is_none());
    assert_eq!(
        queue.record_result(task_id(), true).unwrap().task_state,
        Some(AgentTaskState::Succeeded)
    );
    assert_eq!(
        queue.record_result(task_id(), true).unwrap().task_state,
        Some(AgentTaskState::InProgress)
    );
    assert!(queue.record_result(task_id(), true).is_none());
}

#[test]
fn forced_progress_still_deduplicates_conversation_tokens() {
    let mut queue = LocalTaskUpdateQueue::default();
    let update = |force_task_state| LocalTaskUpdate {
        server_conversation_token: Some("conversation-token".to_owned()),
        ..progress_update(force_task_state)
    };
    assert!(queue.enqueue(task_id(), update(false)).is_some());
    assert!(queue.record_result(task_id(), true).is_none());
    let update = queue.enqueue(task_id(), update(true)).unwrap();
    assert_eq!(update.task_state, Some(AgentTaskState::InProgress));
    assert!(update.server_conversation_token.is_none());
    assert!(queue.record_result(task_id(), true).is_none());
}
