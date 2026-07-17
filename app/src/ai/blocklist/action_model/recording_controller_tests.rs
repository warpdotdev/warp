use ai::agent::action_result::StopRecordingResult;
use computer_use::RecordingHandle;
use futures::executor::block_on;

use super::*;

fn active_controller(recording_id: &str, conversation_id: AIConversationId) -> RecordingController {
    let mut controller = RecordingController::new();
    controller.try_begin_start(conversation_id).unwrap();
    let (handle, _) = RecordingHandle::new_test(1, 1);
    controller.finish_start(recording_id.to_string(), conversation_id, handle);
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

    assert!(controller
        .claim_finalization_for_conversation(AIConversationId::new())
        .is_none());
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

    assert!(controller
        .claim_finalization_for_conversation(AIConversationId::new())
        .is_none());
    assert!(matches!(
        controller.try_begin_start(AIConversationId::new()),
        Err(StartRecordingControllerError::AlreadyInProgress)
    ));
    assert!(controller
        .claim_finalization_for_conversation(owner)
        .is_none());
    assert!(controller.try_begin_start(AIConversationId::new()).is_ok());
}

#[test]
fn records_actions_only_for_the_owning_conversation() {
    let owner = AIConversationId::new();
    let other = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    controller.record_action(other, vec!["other".to_string()]);
    controller.record_action(owner, vec!["owner".to_string()]);

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert_eq!(recording.actions.len(), 1);
    assert_eq!(recording.actions[0].labels, ["owner"]);
}
