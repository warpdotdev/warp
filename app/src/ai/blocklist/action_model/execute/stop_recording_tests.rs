use std::time::Duration;

use ai::agent::action_result::{RecordingStopped, StopRecordingResult};

use super::*;
use crate::ai::blocklist::action_model::RecordingTelemetryEvent;
use crate::ai::blocklist::action_model::recording_finalize::FinalizeReason;

fn success_result(
    completion_status: computer_use::RecordingCompletionStatus,
) -> StopRecordingResult {
    StopRecordingResult::Success(RecordingStopped {
        artifact_uid: "artifact-uid".to_string(),
        duration: Duration::from_secs(3),
        width_px: 1920,
        height_px: 1080,
        size_bytes: 4096,
        completion_status,
        termination_reason: "irrelevant for this test".to_string(),
    })
}

fn assert_termination_reason(event: RecordingTelemetryEvent, expected: &str) {
    let RecordingTelemetryEvent::Stopped {
        termination_reason, ..
    } = event
    else {
        panic!("expected Stopped event");
    };
    assert_eq!(termination_reason, expected);
}

/// Regression: an `Error` result must carry the *actual* finalization reason
/// (e.g. `FfmpegExited` → `encoding_failed`), not the stop action's own
/// `StoppedByAgent` claim. Before the fix, an ffmpeg crash that a later
/// `StopRecording` action joined would be reported as `agent_stopped`, which
/// silently misattributed the trigger in telemetry.
#[test]
fn error_result_reports_actual_reason_not_claimed_agent_stopped() {
    let event = recording_stopped_telemetry(
        "rec",
        FinalizeReason::FfmpegExited,
        &StopRecordingResult::Error("ffmpeg crashed".to_string()),
    );
    assert_termination_reason(event, "encoding_failed");
}

/// Regression: a `Success` result reports the actual reason, including cases
/// where a completed capture was joined by a stop action after the recorder
/// hit the configured limit.
#[test]
fn success_result_reports_actual_limit_reached_reason() {
    let event = recording_stopped_telemetry(
        "rec",
        FinalizeReason::LimitReached,
        &success_result(computer_use::RecordingCompletionStatus::Completed),
    );
    assert_termination_reason(event, "limit_reached");
}

/// A stop action joining a conversation `Cancelled` finalization still reports
/// `cancelled` — previously this was hard-coded, now it flows naturally from
/// the actual reason.
#[test]
fn cancelled_result_reports_cancelled_from_actual_reason() {
    let event = recording_stopped_telemetry(
        "rec",
        FinalizeReason::Cancelled,
        &StopRecordingResult::Cancelled,
    );
    assert_termination_reason(event, "cancelled");
}

/// The normal happy path: an agent-initiated stop that itself started the
/// finalization reports `agent_stopped`, unchanged by this refactor.
#[test]
fn happy_path_agent_stopped_still_reports_agent_stopped() {
    let event = recording_stopped_telemetry(
        "rec",
        FinalizeReason::StoppedByAgent,
        &success_result(computer_use::RecordingCompletionStatus::Completed),
    );
    assert_termination_reason(event, "agent_stopped");
}

/// An `AgentFinished` finalization (the driver's teardown path) surfaces as
/// `agent_finished`, even if a later stop action would have claimed
/// `StoppedByAgent`.
#[test]
fn agent_finished_joined_by_stop_action_reports_agent_finished() {
    let event = recording_stopped_telemetry(
        "rec",
        FinalizeReason::AgentFinished,
        &success_result(computer_use::RecordingCompletionStatus::StoppedEarly),
    );
    assert_termination_reason(event, "agent_finished");
}
