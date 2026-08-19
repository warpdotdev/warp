use std::time::Duration;

use ai::agent::action_result::StopRecordingResult;
use computer_use::{MouseButton, PointerEvent, PointerEventKind, RecordingHandle, Vector2I};
use futures::executor::block_on;

use super::*;

/// Convenience for the tests below — pick a distinctive reason so a stale
/// caller-claimed reason would never coincidentally match.
const TEST_REASON: FinalizeReason = FinalizeReason::LimitReached;

fn active_controller(recording_id: &str, conversation_id: AIConversationId) -> RecordingController {
    let mut controller = RecordingController::new();
    controller.try_begin_start(conversation_id).unwrap();
    let (handle, _) = RecordingHandle::new_test(1, 1);
    controller.finish_start(
        recording_id.to_string(),
        conversation_id,
        handle,
        15,
        1.5,
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

    controller.complete_finalization("recording", result.clone(), TEST_REASON);

    assert_eq!(block_on(first).unwrap(), (result.clone(), TEST_REASON));
    assert_eq!(block_on(second).unwrap(), (result.clone(), TEST_REASON));
    assert!(matches!(
        controller.claim_finalization_by_id("recording"),
        FinalizationClaim::Finished((ref ready, ref ready_reason))
            if ready == &result && ready_reason == &TEST_REASON
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
    controller.complete_finalization("recording", result.clone(), TEST_REASON);

    assert!(matches!(
        controller.claim_finalization_by_id("recording"),
        FinalizationClaim::Finished((ref ready, ref ready_reason))
            if ready == &result && ready_reason == &TEST_REASON
    ));
}

/// Regression test for a `StopRecording` that joins a finalization started by
/// a different path (e.g. the exit watcher's `FfmpegExited` / `LimitReached`,
/// or a conversation `RunCancelled`): the joining caller must observe the actual
/// [`FinalizeReason`] that drove finalization, not any reason it might have
/// claimed when joining. Otherwise `Recording.Stopped.termination_reason`
/// would misattribute the trigger — e.g. an ffmpeg crash would be reported
/// as `agent_stopped`.
#[test]
fn joining_caller_observes_actual_finalize_reason_not_claimed_one() {
    let conversation_id = AIConversationId::new();
    let mut controller = active_controller("recording", conversation_id);

    // First caller starts the work — imagine this is the exit watcher, which
    // claimed `FfmpegExited`.
    let starter = match controller.claim_finalization_by_id("recording") {
        FinalizationClaim::Claimed {
            result_receiver, ..
        } => result_receiver,
        _ => panic!("active recording should be claimed"),
    };
    // A later `StopRecording` action joins the in-progress work. It has no way
    // to influence the reason — the controller ignores anything but the
    // caller that actually started finalization.
    let joiner = match controller.claim_finalization_by_id("recording") {
        FinalizationClaim::InProgress(receiver) => receiver,
        _ => panic!("joining caller should subscribe to in-progress work"),
    };

    let result = StopRecordingResult::Error("ffmpeg crashed".to_string());
    let actual_reason = FinalizeReason::FfmpegExited;
    controller.complete_finalization("recording", result.clone(), actual_reason);

    // Both the starter and the joining caller must see the actual reason.
    let (starter_result, starter_reason) = block_on(starter).unwrap();
    let (joiner_result, joiner_reason) = block_on(joiner).unwrap();
    assert_eq!(starter_result, result);
    assert_eq!(joiner_result, result);
    assert_eq!(starter_reason, actual_reason);
    assert_eq!(joiner_reason, actual_reason);

    // A late caller that only reads the retained result must also see it.
    let FinalizationClaim::Finished((ready_result, ready_reason)) =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("retained result should be available after completion");
    };
    assert_eq!(ready_result, result);
    assert_eq!(ready_reason, actual_reason);
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
    // group is settled into the claimed recording's committed actions.
    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert_eq!(recording.actions.len(), 1);
    // A late commit lands on a controller that is now Finalizing, so it commits
    // nothing rather than recording on the wrong (finalized) recording.
    controller.commit_action_group(owner, Duration::from_millis(500), Vec::new());
    assert_eq!(recording.actions.len(), 1);
}

#[test]
fn detects_playwright_cli_commands() {
    assert!(is_playwright_cli_command(
        "playwright-cli open --headed https://example.com"
    ));
    assert!(is_playwright_cli_command(
        "PLAYWRIGHT_MCP_SANDBOX=0 playwright-cli open https://example.com"
    ));
    assert!(is_playwright_cli_command(
        "/usr/local/bin/playwright-cli attach"
    ));
    assert!(!is_playwright_cli_command("npm install playwright-cli"));
    assert!(!is_playwright_cli_command("echo playwright-cli"));
    assert!(!is_playwright_cli_command("cargo build"));
}

#[test]
fn finalization_commits_open_pending_group() {
    let owner = AIConversationId::new();
    let mut controller = active_controller("recording", owner);

    // A long-running command's group can still be pending when the recording
    // is stopped; finalization must keep its window rather than drop it.
    controller.begin_action_group(owner, vec![]);

    let FinalizationClaim::Claimed { recording, .. } =
        controller.claim_finalization_by_id("recording")
    else {
        panic!("active recording should be claimed");
    };
    assert!(recording.pending_group.is_none());
    assert_eq!(recording.actions.len(), 1);
    assert!(recording.actions[0].finish_offset >= recording.actions[0].offset);
}
