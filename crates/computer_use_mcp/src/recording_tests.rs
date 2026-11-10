use computer_use::{RecordingCompletionStatus, RecordingHandle};

use super::*;

fn recording() -> ActiveRecording {
    let (handle, _) = RecordingHandle::new_test(1920, 1080);
    ActiveRecording::new("recording-1".to_owned(), handle, 15, None, None)
}

#[test]
fn committed_groups_are_logged_with_their_labels() {
    let mut recording = recording();
    let (group, sink) = recording.begin_group(vec!["ctrl+s".to_owned()]);
    assert_eq!(sink.recording_target, Target::Screen);
    recording.commit_group(group);

    let [entry] = recording.actions() else {
        panic!("expected one logged action group");
    };
    assert_eq!(entry.labels, ["ctrl+s"]);
    assert!(entry.finish_offset >= entry.offset);
}

#[test]
fn termination_reasons_explain_why_the_recording_stopped() {
    assert_eq!(
        termination_reason(StopReason::Requested, RecordingCompletionStatus::Completed),
        "Stopped by agent"
    );
    assert_eq!(
        termination_reason(
            StopReason::Requested,
            RecordingCompletionStatus::StoppedEarly
        ),
        "Recording stopped before the agent requested it"
    );
    assert_eq!(
        termination_reason(
            StopReason::SessionEnded,
            RecordingCompletionStatus::Completed
        ),
        "Finalized because the agent run ended without stopping the recording"
    );
}
