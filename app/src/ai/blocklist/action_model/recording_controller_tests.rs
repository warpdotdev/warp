use std::time::Duration;

use ai::agent::action_result::StopRecordingResult;
use computer_use::{MouseButton, PointerEvent, PointerEventKind, RecordingHandle, Vector2I};
use futures::executor::block_on;

use super::*;

fn active_controller(recording_id: &str, conversation_id: AIConversationId) -> RecordingController {
    let mut controller = RecordingController::new();
    controller.try_begin_start(conversation_id).unwrap();
    let (handle, _) = RecordingHandle::new_test(1, 1);
    controller.finish_start(
        recording_id.to_string(),
        conversation_id,
        handle,
        15,
        None,
        None,
        computer_use::Target::Screen,
    );
    controller
}

#[test]
fn finalization_is_shared_and_retained_until_consumed() {
    let conversation_id = AIConversationId::new();
    let mut controller = active_controller("recording", conversation_id);
    assert!(matches!(
        controller.try_begin_start(AIConversationId::new()),
        Err(StartRecordingControllerError::AlreadyInProgress)
    ));

    let first = match controller.claim_finalization_by_id("recording") {
        FinalizationClaim::Claimed {
            result_receiver, ..
        } => result_receiver,
        _ => panic!("active recording should be claimed"),
    };
    let second = match controller.claim_finalization_by_id("recording") {
        FinalizationClaim::InProgress(receiver) => receiver,
        _ => panic!("second caller should wait"),
    };
    assert!(matches!(
        controller.try_begin_start(AIConversationId::new()),
        Err(StartRecordingControllerError::FinalizationInProgress { .. })
    ));
    let result = StopRecordingResult::Error("finished".to_string());

    controller.complete_finalization("recording", result.clone());

    assert_eq!(block_on(first).unwrap(), result);
    assert_eq!(block_on(second).unwrap(), result);
    assert!(matches!(
        controller.claim_finalization_by_id("recording"),
        FinalizationClaim::Finished(ref ready) if ready == &result
    ));
    assert!(matches!(
        controller.try_begin_start(conversation_id),
        Err(StartRecordingControllerError::FinalizedResultPendingDelivery { .. })
    ));

    controller.consume_finalized("recording");
    assert!(controller.try_begin_start(conversation_id).is_ok());
}

#[test]
fn dropped_waiter_does_not_discard_finalized_result() {
    let conversation_id = AIConversationId::new();
    let mut controller = active_controller("recording", conversation_id);
    let receiver = match controller.claim_finalization_by_id("recording") {
        FinalizationClaim::Claimed {
            result_receiver, ..
        } => result_receiver,
        _ => panic!("active recording should be claimed"),
    };
    drop(receiver);

    let result = StopRecordingResult::Error("finished".to_string());
    controller.complete_finalization("recording", result.clone());

    assert!(matches!(
        controller.claim_finalization_by_id("recording"),
        FinalizationClaim::Finished(ref ready) if ready == &result
    ));
}

#[test]
fn mismatched_claim_preserves_active_recording() {
    let conversation_id = AIConversationId::new();
    let mut controller = active_controller("recording", conversation_id);

    assert!(matches!(
        controller.claim_finalization_by_id("other"),
        FinalizationClaim::NotFound
    ));
    assert!(matches!(
        controller.claim_finalization_by_id("recording"),
        FinalizationClaim::Claimed { .. }
    ));
}

#[test]
fn conversation_finalization_only_matches_owner() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    assert!(
        controller
            .claim_finalization_for_conversation(AIConversationId::new())
            .is_none()
    );
    assert!(matches!(
        controller.claim_finalization_for_conversation(owner),
        Some(FinalizationClaim::Claimed { .. })
    ));
}

#[test]
fn matching_conversation_cancels_start_reservation() {
    let owner = AIConversationId::new();
    let mut controller = RecordingController::new();
    controller.try_begin_start(owner).unwrap();

    assert!(
        controller
            .claim_finalization_for_conversation(AIConversationId::new())
            .is_none()
    );
    assert!(matches!(
        controller.try_begin_start(AIConversationId::new()),
        Err(StartRecordingControllerError::AlreadyInProgress)
    ));
    assert!(
        controller
            .claim_finalization_for_conversation(owner)
            .is_none()
    );
    assert!(controller.try_begin_start(AIConversationId::new()).is_ok());
}

#[test]
fn begin_and_commit_record_finish_offset_and_labels() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    // `begin_action_group` reserves a pending group and returns the capture
    // start instant; `commit_action_group` records the finish offset measured
    // after the action sequence (here 500 ms) returns.
    assert!(
        controller
            .begin_action_group(owner, vec!["ctrl+a".to_string()])
            .is_some()
    );
    controller.commit_action_group(owner, Duration::from_millis(500), Vec::new());

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert_eq!(recording.actions.len(), 1);
    let entry = &recording.actions[0];
    assert_eq!(entry.labels, ["ctrl+a"]);
    assert_eq!(entry.finish_offset, Duration::from_millis(500));
    // The finish is after the start, capturing the whole multi-action sequence.
    assert!(entry.finish_offset > entry.offset);
}

#[test]
fn commit_clamps_finish_to_start() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    controller.begin_action_group(owner, vec!["a".to_string()]);
    // A finish before the start is clamped up to the start so the segment
    // builder's one-frame minimum can apply downstream.
    controller.commit_action_group(owner, Duration::ZERO, Vec::new());

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert_eq!(recording.actions.len(), 1);
    assert!(recording.actions[0].finish_offset >= recording.actions[0].offset);
}

#[test]
fn pointer_only_group_commits_with_empty_labels_and_geometry() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    controller.begin_action_group(owner, vec![]);
    let pointer_events = vec![PointerEvent {
        offset: Duration::from_millis(50),
        kind: PointerEventKind::Down,
        button: Some(MouseButton::Left),
        point: Vector2I::new(10, 20),
    }];
    controller.commit_action_group(owner, Duration::from_millis(200), pointer_events);

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert_eq!(recording.actions.len(), 1);
    // A pointer-only group keeps its geometry even though it has no text labels.
    assert!(recording.actions[0].labels.is_empty());
    assert_eq!(recording.actions[0].pointer_events.len(), 1);
}

#[test]
fn discard_drops_pending_group_without_committing() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    controller.begin_action_group(owner, vec!["a".to_string()]);
    // A failed or cancelled `UseComputer` call discards the pending group.
    controller.discard_action_group(owner);

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert!(recording.actions.is_empty());
    assert!(recording.pending_group.is_none());
}

#[test]
fn commit_without_begin_is_noop() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    controller.commit_action_group(owner, Duration::from_millis(500), Vec::new());

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert!(recording.actions.is_empty());
}

#[test]
fn begin_and_commit_are_scoped_to_the_owning_conversation() {
    let owner = AIConversationId::new();
    let other = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    assert!(
        controller
            .begin_action_group(owner, vec!["owner".to_string()])
            .is_some()
    );
    // Another conversation cannot begin (returns None) and cannot commit; the
    // owner's pending group is untouched.
    assert!(
        controller
            .begin_action_group(other, vec!["other".to_string()])
            .is_none()
    );
    controller.commit_action_group(other, Duration::from_millis(999), Vec::new());
    controller.commit_action_group(owner, Duration::from_millis(300), Vec::new());

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert_eq!(recording.actions.len(), 1);
    assert_eq!(recording.actions[0].labels, ["owner"]);
    assert_eq!(
        recording.actions[0].finish_offset,
        Duration::from_millis(300)
    );
}

#[test]
fn begin_while_pending_auto_commits_prior_group() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    // Start the first group without committing it.
    controller.begin_action_group(owner, vec!["click".to_string()]);
    // Beginning a second group auto-commits the first with an implicit finish
    // rather than silently discarding it.
    controller.begin_action_group(owner, vec!["type".to_string()]);
    // Commit the second group explicitly.
    controller.commit_action_group(owner, Duration::from_millis(700), Vec::new());

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    // Both groups should be present: the auto-committed first and the
    // explicitly-committed second.
    assert_eq!(recording.actions.len(), 2);
    assert_eq!(recording.actions[0].labels, ["click"]);
    assert_eq!(recording.actions[1].labels, ["type"]);
    assert_eq!(
        recording.actions[1].finish_offset,
        Duration::from_millis(700)
    );
    // Auto-committed group's finish is >= its own start.
    assert!(recording.actions[0].finish_offset >= recording.actions[0].offset);
}

#[test]
fn commit_after_finalization_is_noop() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    assert!(
        controller
            .begin_action_group(owner, vec!["a".to_string()])
            .is_some()
    );
    // The recording is finalized while the action is in flight; the pending
    // group leaves with the claimed recording.
    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    // A late commit lands on a controller that is now Finalizing, so it commits
    // nothing rather than recording on the wrong (finalized) recording.
    controller.commit_action_group(owner, Duration::from_millis(500), Vec::new());
    assert!(recording.actions.is_empty());
}
