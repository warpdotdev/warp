use session_sharing_protocol::common::ParticipantId;

use super::{PendingCliHarnessPromptQueue, QueuedCliHarnessPrompt};
use crate::ai::ambient_agents::AmbientAgentTaskId;

fn fixed_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440a00"
        .parse()
        .expect("valid task id")
}

#[test]
fn drain_on_empty_queue_returns_empty_vec() {
    let mut queue = PendingCliHarnessPromptQueue::default();
    assert!(queue.drain(fixed_task_id()).is_empty());
}

#[test]
fn queued_prompts_are_drained_in_fifo_order() {
    let mut queue = PendingCliHarnessPromptQueue::default();
    let task_id = fixed_task_id();

    queue.queue(
        task_id,
        QueuedCliHarnessPrompt {
            prompt: "first".to_string(),
            participant_id: ParticipantId::new(),
        },
    );
    queue.queue(
        task_id,
        QueuedCliHarnessPrompt {
            prompt: "second".to_string(),
            participant_id: ParticipantId::new(),
        },
    );

    let drained = queue.drain(task_id);
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].prompt, "first");
    assert_eq!(drained[1].prompt, "second");

    // Draining again must not redeliver.
    assert!(queue.drain(task_id).is_empty());
}

#[test]
fn queues_for_different_tasks_are_independent() {
    let mut queue = PendingCliHarnessPromptQueue::default();
    let task_a: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440a00"
        .parse()
        .expect("valid task id");
    let task_b: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440a01"
        .parse()
        .expect("valid task id");

    queue.queue(
        task_a,
        QueuedCliHarnessPrompt {
            prompt: "for a".to_string(),
            participant_id: ParticipantId::new(),
        },
    );

    assert!(queue.drain(task_b).is_empty());
    let drained_a = queue.drain(task_a);
    assert_eq!(drained_a.len(), 1);
    assert_eq!(drained_a[0].prompt, "for a");
}

#[test]
fn clear_drops_queued_prompts_without_delivering() {
    let mut queue = PendingCliHarnessPromptQueue::default();
    let task_id = fixed_task_id();

    queue.queue(
        task_id,
        QueuedCliHarnessPrompt {
            prompt: "dropped".to_string(),
            participant_id: ParticipantId::new(),
        },
    );
    queue.clear(task_id);

    assert!(queue.drain(task_id).is_empty());
}
